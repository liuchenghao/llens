use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use super::store::Config;

/// Frame-level record: one screenshot + its AI understanding.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Frame {
    /// ISO-8601 local time of the frame
    pub time: String,
    /// relative path under data_root (screenshots/YYYY-MM/HHMMSS.png) — legacy full-size
    pub image: String,
    /// extra display screenshots, same layout as `image` (screenshots/YYYY-MM/HHMMSS_d2.png …)
    pub extra_images: Vec<String>,
    /// timeline thumbnail (<100KB, JPEG ≤400px wide); may be empty for legacy frames
    #[serde(default)]
    pub thumb: String,
    /// large preview image (JPEG <1MB, ≤2048px wide); may be empty for legacy frames
    #[serde(default)]
    pub preview: String,
    /// extra display thumbnails, parallel to `extra_images`
    #[serde(default)]
    pub extra_thumbs: Vec<String>,
    /// extra display preview images, parallel to `extra_images`
    #[serde(default)]
    pub extra_previews: Vec<String>,
    /// exactly 5 short sentences from the model (empty if pending/failed)
    pub summary5: Vec<String>,
    /// 工作 | 学习 | 娱乐 | 社交 | 其他
    pub activity: Option<String>,
    /// app name visible in the screenshot (or "unknown")
    pub app: Option<String>,
    /// project name/workspace hint (or "unknown")
    pub project: Option<String>,
    /// 熄屏/黑屏时段标记：true 表示该帧检测到屏幕熄灭（未生成截图、未调 LLM），
    /// 该时段计入休息而非工作。旧数据默认 false。
    #[serde(default)]
    pub rest: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HourFile {
    /// "YYYY-MM-DD HH:00"
    pub hour: String,
    pub frames: Vec<Frame>,
}

/// Run the 20s capture loop (spawned at app start).
pub async fn run_loop(handle: tauri::AppHandle) {
    use tauri::Manager;
    let data_root = super::tstate::AppState::data_root();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(20));
    interval.tick().await;
    loop {
        let state = handle.state::<super::tstate::AppState>();
        if state.recording.load(std::sync::atomic::Ordering::SeqCst) {
            let cfg = Config::load(&data_root);
            if let Err(e) = capture_once(&cfg, &data_root).await {
                eprintln!("capture error: {e}");
            }
        }
        interval.tick().await;
    }
}

// ---------- shared summarization helpers ----------

/// Compress a screenshot to a JPEG data-URL (≤1280px, q70).
async fn compress_to_data_url(p: &std::path::Path) -> Result<String, String> {
    let img = std::fs::read(p).map_err(|e| format!("read shot {}: {e}", p.display()))?;
    let (_jpeg, data_url) = compress_for_llm(&img).await?;
    Ok(data_url)
}

/// Build the frame-summarization prompt.
fn frame_prompt(n_displays: usize, time_str: &str) -> String {
    let display_note = if n_displays == 1 {
        "这是用户 macOS 的屏幕截图（1 块屏幕）。".to_string()
    } else {
        format!(
            "这是用户 macOS 多显示器在同一时刻的 {n_displays} 张截图（按显示器 1..{n_displays} 排列，第 1 张是主屏）。请综合所有屏幕理解用户当前活动。"
        )
    };
    format!(
        r#"{}请看截图，请严格按如下 JSON 输出（不要输出 JSON 以外的任何文字）：
{{"summary5":["第一句","第二句","第三句","第四句","第五句"],"activity":"工作|学习|娱乐|社交|其他","app":"当前主应用名，无法判断则 unknown","project":"可见的项目/工作区名，无法判断则 unknown"}}
其中 summary5 恰好 5 句话，每句不超过 30 字，用中文，客观描述用户正在做什么（如果多屏在同时做不同的事，请合并概括）。
截图时间：{time_str}。"#,
        display_note
    )
}

/// Parse JSON out of messy model output.
fn parse_llm_json(text: &str) -> serde_json::Value {
    let mut parsed = serde_json::Value::Null;
    if let Some(start) = text.find('{') {
        if let Some(end) = text.rfind('}') {
            if end > start {
                parsed = serde_json::from_str(&text[start..=end]).unwrap_or(serde_json::Value::Null);
            }
        }
    }
    parsed
}

/// Run the LLM with retry/backoff and return the parsed JSON.
async fn llm_summarize(cfg: &Config, data_urls: &[String], n_displays: usize, time_str: &str) -> serde_json::Value {
    let prompt = frame_prompt(n_displays, time_str);
    let mut llm_text = String::new();
    let client = reqwest::Client::new();
    for attempt in 0..3 {
        let msg = super::llm::multi_image_msg(data_urls, &prompt);
        match super::llm::chat(&client, cfg, &[msg]).await {
            Ok(t) => {
                llm_text = t;
                break;
            }
            Err(e) => {
                if attempt < 2 {
                    eprintln!("llm attempt {} failed: {e}; retrying", attempt + 1);
                    tokio::time::sleep(std::time::Duration::from_secs(u64::from(2u32.pow(attempt + 1) * 4))).await;
                } else {
                    eprintln!("llm failed after 3 attempts: {e}");
                }
            }
        }
    }
    parse_llm_json(&llm_text)
}

/// Re-summarize an existing frame (screenshot already on disk). Used by the
/// retry button in the timeline. Returns the updated frame.
pub async fn retry_frame(cfg: &Config, data_root: &PathBuf, frame: &Frame) -> Result<Frame, String> {
    // 原图（image / extra_images）在 LLM 解释完成后已被删除；重试时
    // fallback 到压缩预览图（preview / extra_previews，≤2048px JPEG），质量足够 LLM 理解。
    let shot = data_root.join(&frame.image);
    let main_src: std::path::PathBuf = if shot.exists() {
        shot
    } else if !frame.preview.is_empty() {
        data_root.join(&frame.preview)
    } else {
        return Err(format!(
            "screenshot not found: {} (and no preview image)",
            frame.image
        ));
    };
    let mut paths: Vec<std::path::PathBuf> = vec![main_src];
    for (i, extra) in frame.extra_images.iter().enumerate() {
        let p = data_root.join(extra);
        if p.exists() {
            paths.push(p);
        } else {
            // 原图已删 → 用对应的压缩预览图
            if let Some(ep) = frame.extra_previews.get(i) {
                let pp = data_root.join(ep);
                if pp.exists() {
                    paths.push(pp);
                }
            }
        }
    }
    let time_str = frame.time.clone();
    let data_urls = futures_join_paths(&paths).await?;
    let parsed = llm_summarize(cfg, &data_urls, paths.len(), &time_str).await;

    let mut out = frame.clone();
    out.summary5 = parsed["summary5"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .take(5)
                .collect()
        })
        .unwrap_or_default();
    out.activity = parsed["activity"].as_str().map(|s| s.to_string());
    out.app = parsed["app"].as_str().map(|s| s.to_string());
    out.project = parsed["project"].as_str().map(|s| s.to_string());
    Ok(out)
}

/// Replace a single frame's summary fields in the day's hour log files.
pub fn replace_frame_in_day(data_root: &PathBuf, frame_time: &str, updated: &Frame) {
    let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();
    let day = frame_time.get(..10).unwrap_or(&today_str);
    let month = day.get(..7).unwrap_or_default();
    let log_dir = data_root.join("logs").join(month);
    let mut found = false;
    for h in 0..24u32 {
        let p = log_dir
            .join(format!("{day}_{h:02}"))
            .with_extension("json");
        if !p.exists() {
            continue;
        }
        let s = match std::fs::read_to_string(&p) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut hf: HourFile = match serde_json::from_str(&s) {
            Ok(hf) => hf,
            Err(_) => continue,
        };
        if let Some(idx) = hf.frames.iter().position(|f| f.time == frame_time) {
            hf.frames[idx] = updated.clone();
            write_hour_file(&p, &hf);
            found = true;
            break;
        }
    }
    if !found {
        eprintln!("replace_frame_in_day: frame {frame_time} not found in any hour file");
    }
}

/// Compress all paths to data-URLs.
async fn futures_join_paths(paths: &[std::path::PathBuf]) -> Result<Vec<String>, String> {
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        out.push(compress_to_data_url(p).await?);
    }
    Ok(out)
}

/// One 20s tick: screenshot + AI summary + hour JSON append + 10min boundary check.
pub async fn capture_once(cfg: &Config, data_root: &PathBuf) -> Result<Frame, String> {
    let now = chrono::Local::now();
    let shot_dir = data_root
        .join("screenshots")
        .join(now.format("%Y-%m").to_string());
    std::fs::create_dir_all(&shot_dir)
        .map_err(|e| format!("mkdir screenshots: {e}"))?;
    let base_stem = now.format("%H%M%S").to_string();

    // 0) Screen-off / blackout check: when the main display is asleep (screen
    //    off, lid closed, or blackout app) we record a lightweight "rest" frame
    //    instead of taking a screenshot or calling the LLM. This period counts
    //    as rest, not work, and no image is generated on disk.
    if super::display::main_display_asleep() {
        let rest_frame = Frame {
            time: now.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
            image: String::new(),
            extra_images: Vec::new(),
            thumb: String::new(),
            preview: String::new(),
            extra_thumbs: Vec::new(),
            extra_previews: Vec::new(),
            summary5: vec!["屏幕熄灭，处于休息状态".to_string()],
            activity: Some("休息".to_string()),
            app: None,
            project: None,
            rest: true,
        };
        // Still append the rest frame so the 10-min slice can count this
        // period as rest. No screenshots are written.
        append_hour_frame(data_root, &now, &rest_frame);
        let bucket = (now.timestamp() / 600) * 600;
        let sfile = super::store::sum_file_for(data_root, &now, bucket);
        if !sfile.exists() {
            let _ = super::store::write_10min_summary(data_root, &now, cfg).await;
            let cfg_clone = cfg.clone();
            let root_clone = data_root.clone();
            tokio::spawn(async move {
                let _ = super::diary::regenerate_pending(&root_clone, &cfg_clone, 3).await;
            });
        }
        return Ok(rest_frame);
    }

    /// Capture one display. display 1 is always the main display.
    fn capture_display(shot_dir: &PathBuf, stem: &str, display: u32) -> Option<std::path::PathBuf> {
        let fname = if display == 1 {
            format!("{stem}.png")
        } else {
            format!("{stem}_d{display}.png")
        };
        let path = shot_dir.join(&fname);
        let out = std::process::Command::new("screencapture")
            .arg("-x")
            .arg("-D")
            .arg(display.to_string())
            .arg(&path)
            .output()
            .ok()?;
        if out.status.success() && path.exists() {
            Some(path)
        } else {
            let _ = std::fs::remove_file(&path);
            None
        }
    }

    // 1) Capture every connected display (1 = main; try up to 4 total).
    // Multi-monitor: the user works across screens, so each display is
    // captured into its own file and all images go to the LLM together.
    let main_path = match capture_display(&shot_dir, &base_stem, 1) {
        Some(p) => p,
        None => {
            // Fall back: -D 1 can fail on some systems; try bare -x.
            let p = shot_dir.join(format!("{base_stem}.png"));
            let out = std::process::Command::new("screencapture")
                .arg("-x")
                .arg(&p)
                .output()
                .map_err(|e| format!("spawn screencapture: {e}"))?;
            if !out.status.success() || !p.exists() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Err(format!(
                    "screencapture failed (screen-recording permission?): {stderr}"
                ));
            }
            p
        }
    };
    let mut extra_paths: Vec<std::path::PathBuf> = Vec::new();
    for d in 2..=4u32 {
        match capture_display(&shot_dir, &base_stem, d) {
            Some(p) => extra_paths.push(p),
            None => break, // displays are numbered sequentially; stop at the first gap
        }
    }

    // 2) Compress every display to JPEG ≤1280px — upstream LLM 500s on
    // multi-MB PNG payloads (Retina 3x full-screen PNGs are 5-10MB each).
    let all_paths = std::iter::once(main_path.clone()).chain(extra_paths.clone()).collect::<Vec<_>>();
    let data_urls = futures_join_paths(&all_paths).await?;

    let time_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
    let parsed = llm_summarize(cfg, &data_urls, all_paths.len(), &time_str).await;

    let summary5: Vec<String> = parsed["summary5"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(|s| s.trim().to_string())
                .take(5)
                .collect()
        })
        .unwrap_or_default();
    let activity = parsed["activity"].as_str().map(|s| s.to_string());
    let app = parsed["app"].as_str().map(|s| s.to_string());
    let project = parsed["project"].as_str().map(|s| s.to_string());

    let main_rel = format!(
        "screenshots/{}/{base_stem}.png",
        now.format("%Y-%m")
    );
    let extra_rels: Vec<String> = extra_paths
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .map(|fn2| format!("screenshots/{}/{}", now.format("%Y-%m"), fn2))
        .collect();

    // 5) Generate timeline thumbnails (<100KB) + large previews (<1MB) for each display,
    //    then delete the original full-size PNGs (the LLM already consumed its copy).
    let month = now.format("%Y-%m").to_string();
    let thumb_rel = format!("screenshots/{}/{}_t.jpg", month, base_stem);
    let preview_rel = format!("screenshots/{}/{}_p.jpg", month, base_stem);
    let mut extra_thumb_rels: Vec<String> = Vec::new();
    let mut extra_preview_rels: Vec<String> = Vec::new();
    let mut originals_to_delete: Vec<std::path::PathBuf> = Vec::new();

    if make_thumbnail(&main_path, &shot_dir.join(format!("{base_stem}_t.jpg"))).is_ok()
        && make_preview(&main_path, &shot_dir.join(format!("{base_stem}_p.jpg"))).is_ok()
    {
        originals_to_delete.push(main_path.clone());
    }
    for ep in extra_paths.iter() {
        // 与主屏一致：直接用完整 PathBuf 拼出 {stem}_t.jpg / {stem}_p.jpg，
        // 避免 with_extension("_t.jpg") 产生 {stem}._t.jpg（多一个点）导致相对路径对不上。
        let stem_extra = ep
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
            .trim_end_matches(".png")
            .to_string();
        let t_path = shot_dir.join(format!("{stem_extra}_t.jpg"));
        let p_path = shot_dir.join(format!("{stem_extra}_p.jpg"));
        if make_thumbnail(ep, &t_path).is_ok() && make_preview(ep, &p_path).is_ok() {
            extra_thumb_rels.push(format!("screenshots/{month}/{stem_extra}_t.jpg"));
            extra_preview_rels.push(format!("screenshots/{month}/{stem_extra}_p.jpg"));
            originals_to_delete.push(ep.clone());
        }
    }

    let main_ok = !originals_to_delete.is_empty()
        && originals_to_delete.iter().any(|p| p == &main_path);

    let frame = Frame {
        time: now.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        image: main_rel,
        extra_images: extra_rels,
        thumb: if main_ok { thumb_rel } else { String::new() },
        preview: if main_ok { preview_rel } else { String::new() },
        extra_thumbs: extra_thumb_rels,
        extra_previews: extra_preview_rels,
        summary5,
        activity,
        app,
        project,
        rest: false,
    };

    // 6) Remove the original full-size PNGs (compressed copies already written).
    for p in &originals_to_delete {
        let _ = std::fs::remove_file(p);
    }

    // 3) append to current-hour JSON atomically
    append_hour_frame(data_root, &now, &frame);

    // 4) maybe trigger a 10-min summary when crossing a boundary
    let bucket = (now.timestamp() / 600) * 600;
    let sfile = super::store::sum_file_for(data_root, &now, bucket);
    if !sfile.exists() {
        let _ = super::store::write_10min_summary(data_root, &now, cfg).await;
        // Auto-regenerate stale diaries in the background so the diary view
        // updates without the user having to click refresh. The check is
        // cheap: it only re-runs the LLM for days whose 10-min slice count
        // has grown since the last diary generation.
        let cfg_clone = cfg.clone();
        let root_clone = data_root.clone();
        tokio::spawn(async move {
            let _ = super::diary::regenerate_pending(&root_clone, &cfg_clone, 3).await;
        });
    }

    Ok(frame)
}

/// Append a frame into the hour JSON file (atomic rewrite via tmp+rename).
pub fn append_hour_frame(
    data_root: &PathBuf,
    now: &chrono::DateTime<chrono::Local>,
    frame: &Frame,
) {
    let hour_key = now.format("%Y-%m-%d %H:00").to_string();
    let month = now.format("%Y-%m").to_string();
    let hour_dir = data_root.join("logs").join(&month);
    let _ = std::fs::create_dir_all(&hour_dir);
    let hfile = hour_dir
        .join(now.format("%Y-%m-%d_%H").to_string())
        .with_extension("json");
    let mut hf = read_hour_file(&hfile, &hour_key);
    hf.frames.push(frame.clone());
    write_hour_file(&hfile, &hf);
}

/// Compress a raw screenshot (PNG/JPEG) to JPEG ≤1280px wide, q=70, via macOS `sips`.
/// Returns (jpeg_bytes, data_url). The original file on disk is untouched.
async fn compress_for_llm(raw: &[u8]) -> Result<(Vec<u8>, String), String> {
    // write raw to a temp file, sips -> jpeg
    let tmp_in = std::env::temp_dir().join(format!("llens_raw_{}.bin", uuid::Uuid::new_v4()));
    std::fs::write(&tmp_in, raw).map_err(|e| format!("write tmp: {e}"))?;
    let tmp_out = tmp_in.with_extension("jpg");
    let sips = std::process::Command::new("sips")
        .args(["-Z", "1280", "-s", "format", "jpeg", "-s", "formatOptions", "70"])
        .arg(&tmp_in)
        .arg("--out")
        .arg(&tmp_out)
        .output();
    let _ = std::fs::remove_file(&tmp_in);
    let sips = sips.map_err(|e| format!("sips spawn: {e}"))?;
    if !sips.status.success() {
        let stderr = String::from_utf8_lossy(&sips.stderr);
        eprintln!("sips failed, falling back to raw: {stderr}");
        let b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            raw,
        );
        return Ok((raw.to_vec(), format!("data:image/png;base64,{b64}")));
    }
    let jpeg = std::fs::read(&tmp_out).map_err(|e| format!("read sips out: {e}"))?;
    let _ = std::fs::remove_file(&tmp_out);
    let b64 = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &jpeg,
    );
    Ok((jpeg, format!("data:image/jpeg;base64,{b64}")))
}

/// Create a thumbnail JPEG (≤400px wide, q=72) from a source image on disk.
/// Target: <100KB (Retina 3x at 400px q72 is typically 15-40KB).
/// Writes to `out_path`; returns Ok on success.
fn make_thumbnail(src: &std::path::Path, out_path: &std::path::Path) -> Result<(), String> {
    let sips = std::process::Command::new("sips")
        .args(["-Z", "400", "-s", "format", "jpeg", "-s", "formatOptions", "72"])
        .arg(src)
        .arg("--out")
        .arg(out_path)
        .output()
        .map_err(|e| format!("sips thumb spawn: {e}"))?;
    if !sips.status.success() {
        let stderr = String::from_utf8_lossy(&sips.stderr);
        return Err(format!("sips thumb failed: {stderr}"));
    }
    Ok(())
}

/// Create a large preview JPEG (≤2048px wide) from a source image on disk,
/// targeting <1MB. Tries quality 85 → 75 → 60 → 45 until under 1MB.
fn make_preview(src: &std::path::Path, out_path: &std::path::Path) -> Result<(), String> {
    const TARGET: u64 = 1_000_000; // 1MB
    for q in ["85", "75", "60", "45"] {
        let sips = std::process::Command::new("sips")
            .args(["-Z", "2048", "-s", "format", "jpeg", "-s", "formatOptions", q])
            .arg(src)
            .arg("--out")
            .arg(out_path)
            .output()
            .map_err(|e| format!("sips preview spawn: {e}"))?;
        if !sips.status.success() {
            let stderr = String::from_utf8_lossy(&sips.stderr);
            return Err(format!("sips preview failed: {stderr}"));
        }
        if out_path.metadata().map(|m| m.len() < TARGET).unwrap_or(false) {
            return Ok(());
        }
        // too big, try lower quality
    }
    // 45 still over 1MB (rare, e.g. very complex screen) — accept it anyway
    Ok(())
}

fn read_hour_file(p: &PathBuf, hour_key: &str) -> HourFile {
    match std::fs::read_to_string(p) {
        Ok(s) => serde_json::from_str(&s).unwrap_or(HourFile {
            hour: hour_key.into(),
            frames: vec![],
        }),
        Err(_) => HourFile {
            hour: hour_key.into(),
            frames: vec![],
        },
    }
}

fn write_hour_file(p: &PathBuf, hf: &HourFile) {
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let s = serde_json::to_string_pretty(hf).unwrap_or_default();
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, s).is_ok() {
        let _ = std::fs::rename(&tmp, p);
    }
}
