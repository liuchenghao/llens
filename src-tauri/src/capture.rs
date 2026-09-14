use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use super::store::Config;

/// Frame-level record: one screenshot + its AI understanding.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Frame {
    /// ISO-8601 local time of the frame
    pub time: String,
    /// relative path under data_root (screenshots/YYYY-MM/HHMMSS.png)
    pub image: String,
    /// exactly 5 short sentences from the model (empty if pending/failed)
    pub summary5: Vec<String>,
    /// 工作 | 学习 | 娱乐 | 社交 | 其他
    pub activity: Option<String>,
    /// app name visible in the screenshot (or "unknown")
    pub app: Option<String>,
    /// project name/workspace hint (or "unknown")
    pub project: Option<String>,
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

/// One 20s tick: screenshot + AI summary + hour JSON append + 10min boundary check.
pub async fn capture_once(cfg: &Config, data_root: &PathBuf) -> Result<Frame, String> {
    let now = chrono::Local::now();
    let shot_dir = data_root
        .join("screenshots")
        .join(now.format("%Y-%m").to_string());
    std::fs::create_dir_all(&shot_dir)
        .map_err(|e| format!("mkdir screenshots: {e}"))?;
    let fname = now.format("%H%M%S").to_string() + ".png";
    let shot_path = shot_dir.join(&fname);

    // 1) screencapture (silent; TCC prompt on first run)
    let out = std::process::Command::new("screencapture")
        .arg("-x")
        .arg(&shot_path)
        .output()
        .map_err(|e| format!("spawn screencapture: {e}"))?;
    if !out.status.success() || !shot_path.exists() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "screencapture failed (screen-recording permission?): {stderr}"
        ));
    }

    // 2) AI: 5-sentence summary + activity/app/project
    // Compress to JPEG ≤1280px before encoding — the upstream LLM 500s on
    // multi-MB PNG payloads (Retina 3x full-screen PNGs are 5-10MB).
    let img = std::fs::read(&shot_path).map_err(|e| format!("read shot: {e}"))?;
    let (_jpeg, data_url) = compress_for_llm(&img).await?;

    let prompt = format!(
        r#"你看一张 macOS 屏幕截图。请严格按如下 JSON 输出（不要输出 JSON 以外的任何文字）：
{{"summary5":["第一句","第二句","第三句","第四句","第五句"],"activity":"工作|学习|娱乐|社交|其他","app":"当前主应用名，无法判断则 unknown","project":"可见的项目/工作区名，无法判断则 unknown"}}
其中 summary5 恰好 5 句话，每句不超过 30 字，用中文，客观描述用户正在做什么。
截图时间：{}。"#,
        now.format("%Y-%m-%d %H:%M:%S")
    );

    let mut llm_text = String::new();
    let client = reqwest::Client::new();
    // 3 attempts with exponential backoff (2s, 8s) — upstream 500s are transient
    for attempt in 0..3 {
        match super::llm::chat(&client, cfg, &[super::llm::image_msg(&data_url, &prompt)]).await {
            Ok(t) => {
                llm_text = t;
                break;
            }
            Err(e) => {
                if attempt < 2 {
                    eprintln!("llm attempt {} failed: {e}; retrying", attempt + 1);
                    tokio::time::sleep(std::time::Duration::from_secs(
                        u64::from(2u32.pow(attempt + 1) * 4),
                    ))
                    .await;
                } else {
                    eprintln!("llm failed after 3 attempts (frame saved without summary): {e}");
                }
            }
        }
    }

    // parse JSON out of the (possibly messy) model output
    let mut parsed = serde_json::Value::Null;
    if let Some(start) = llm_text.find('{') {
        if let Some(end) = llm_text.rfind('}') {
            if end > start {
                parsed =
                    serde_json::from_str(&llm_text[start..=end]).unwrap_or(serde_json::Value::Null);
            }
        }
    }
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

    let frame = Frame {
        time: now.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        image: format!(
            "screenshots/{}/{}",
            now.format("%Y-%m"), fname
        ),
        summary5,
        activity,
        app,
        project,
    };

    // 3) append to current-hour JSON atomically
    append_hour_frame(data_root, &now, &frame);

    // 4) maybe trigger a 10-min summary when crossing a boundary
    let bucket = (now.timestamp() / 600) * 600;
    let sfile = super::store::sum_file_for(data_root, &now, bucket);
    if !sfile.exists() {
        let _ = super::store::write_10min_summary(data_root, &now, cfg).await;
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
