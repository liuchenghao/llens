use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use image::{GenericImage, ImageEncoder};
use image::imageops::FilterType;
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
    /// extra display preview images, parallel to `extra_previews`
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
    // Auto-cleanup cadence (PRD §4.6): run retention cleanup at most once per 6h.
    const CLEANUP_INTERVAL_SECS: i64 = 6 * 3600;
    loop {
        let state = handle.state::<super::tstate::AppState>();
        if state.recording.load(std::sync::atomic::Ordering::SeqCst) {
            let cfg = Config::load(&data_root);
            if let Err(e) = capture_once(&cfg, &data_root).await {
                eprintln!("capture error: {e}");
            }
        }
        // Periodic data cleanup based on configured retention days.
        let now_ts = chrono::Local::now().timestamp();
        let due = {
            let g = state.last_cleanup.lock().unwrap();
            now_ts.saturating_sub(*g) >= CLEANUP_INTERVAL_SECS
        };
        if due {
            let cfg = Config::load(&data_root);
            match super::cleanup::run_cleanup(&data_root, &cfg) {
                Ok(rep) => {
                    *state.last_cleanup.lock().unwrap() = now_ts;
                    if rep.removed_images + rep.removed_json > 0 {
                        eprintln!(
                            "cleanup: removed {} images, {} json files, freed {} bytes",
                            rep.removed_images, rep.removed_json, rep.freed_bytes
                        );
                    }
                }
                Err(e) => eprintln!("cleanup error: {e}"),
            }
        }
        interval.tick().await;
    }
}

// ---------- shared summarization helpers ----------

/// Compress a screenshot file to a JPEG data-URL (≤1280px, q70).
async fn compress_to_data_url(p: &std::path::Path) -> Result<String, String> {
    let img = std::fs::read(p).map_err(|e| format!("read shot {}: {e}", p.display()))?;
    let (_jpeg, data_url) = compress_for_llm(&img).await?;
    Ok(data_url)
}

/// Stitch the per-display preview JPEGs horizontally into a single composed
/// timeline thumbnail. All displays are resized to a common height (66px),
/// concatenated side-by-side, and encoded as JPEG <100KB.
#[allow(dead_code)] // 非 macOS 平台（Linux/Windows 单屏）不使用此函数
fn compose_displays_thumb(
    display_paths: &[std::path::PathBuf],
    out_path: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    if display_paths.is_empty() {
        return Err("no display images".into());
    }
    let target_h: u32 = 66;
    let imgs: Vec<image::DynamicImage> = display_paths
        .iter()
        .map(|p| image::open(p).map_err(|e| format!("open {}: {e}", p.display())))
        .collect::<Result<Vec<_>, String>>()?;
    // Resize each to target height, preserving aspect ratio
    let total_w: u32 = imgs
        .iter()
        .map(|img| {
            ((img.width() as f64 * target_h as f64 / img.height().max(1) as f64))
                .round()
                .max(1.0) as u32
        })
        .sum();
    let mut canvas = image::DynamicImage::new_rgb8(total_w, target_h);
    let mut x = 0u32;
    for img in &imgs {
        let w = ((img.width() as f64 * target_h as f64 / img.height().max(1) as f64))
            .round()
            .max(1.0) as u32;
        let resized = img.resize(w, target_h, FilterType::Lanczos3);
        let _ = canvas.copy_from(&resized, x, 0); // OOB tail safe to ignore
        x += resized.width();
    }
    // Encode JPEG, iterate quality 75→45 if >100KB.
    let rgb = canvas.to_rgb8();
    let (w, h) = (rgb.width(), rgb.height());
    let rgb_buf: Vec<u8> = rgb.into_raw();
    for q in [75u8, 70, 60, 50, 45] {
        let mut buf: Vec<u8> = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q);
        encoder
            .write_image(&rgb_buf, w, h, image::ExtendedColorType::Rgb8)
            .map_err(|e| format!("jpeg encode: {e}"))?;
        if buf.len() <= 100_000 || q == 45 {
            std::fs::write(out_path, &buf).map_err(|e| format!("write thumb: {e}"))?;
            break;
        }
    }
    Ok(out_path.to_path_buf())
}

/// Build the frame-summarization prompt.
fn frame_prompt(n_displays: usize, time_str: &str) -> String {
    let os_name = if cfg!(target_os = "macos") { "macOS" } else if cfg!(target_os = "linux") { "Linux" } else { "Windows" };
    let display_note = if n_displays == 1 {
        format!("这是用户 {os_name} 的屏幕截图（1 块屏幕）。")
    } else {
        format!(
            "这是用户 {os_name} 多显示器在同一时刻的 {n_displays} 张截图（按显示器 1..{n_displays} 排列，第 1 张是主屏）。请综合所有屏幕理解用户当前活动。"
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
    // 若 image 为空字符串（rest 休息帧或老数据无图），直接报可读错误，避免把 data_root 目录当文件读。
    let image = frame.image.trim();
    let preview = frame.preview.trim();
    let main_src: std::path::PathBuf;
    if !image.is_empty() && data_root.join(image).exists() {
        main_src = data_root.join(image);
    } else if !preview.is_empty() && data_root.join(preview).exists() {
        main_src = data_root.join(preview);
    } else {
        return Err(format!(
            "该帧无可用截图（image = '{image}'，preview = '{preview}'），无法重新生成。rest 休息帧或未生成图的老帧不支持重试。"
        ));
    }
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
                let _ = super::diary::regenerate_pending(&root_clone, &cfg_clone, 3, 30).await;
            });
        }
        return Ok(rest_frame);
    }

    #[cfg(target_os = "linux")]
    {
        // ---- Linux: single screen via scrot (Wayland 需要 maim/sway 等，当前按 X11/Xorg 设计) ----
        let shot_path = shot_dir.join(format!("{base_stem}.png"));
        let out = std::process::Command::new("scrot")
            .arg(&shot_path)
            .output()
            .map_err(|e| format!("spawn scrot: {e} (请确认已安装 scrot)"))?;
        if !out.status.success() || !shot_path.exists() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!("scrot failed (X11 环境缺失?): {stderr}"));
        }

        let month = now.format("%Y-%m").to_string();
        let preview_rel = format!("screenshots/{month}/{base_stem}.jpg");
        let preview_path = shot_dir.join(format!("{base_stem}.jpg"));
        let preview_ok = make_preview(&shot_path, &preview_path).is_ok();

        // 删除原图，保留预览 JPEG
        let _ = std::fs::remove_file(&shot_path);

        let data_src = if preview_ok {
            vec![preview_path.clone()]
        } else {
            return Err("Linux: 截图预览生成失败，无法调用 LLM".into());
        };

        let data_urls = futures_join_paths(&data_src).await?;
        let time_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let parsed = llm_summarize(cfg, &data_urls, 1, &time_str).await;

        let frame = Frame {
            time: now.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
            image: String::new(), // Linux 端原图已删，直接存空，preview 为准
            extra_images: Vec::new(),
            thumb: if preview_ok { preview_rel.clone() } else { String::new() },
            preview: if preview_ok { preview_rel.clone() } else { String::new() },
            extra_thumbs: Vec::new(),
            extra_previews: Vec::new(),
            summary5: parsed["summary5"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(|s| s.trim().to_string())
                        .take(5)
                        .collect()
                })
                .unwrap_or_default(),
            activity: parsed["activity"].as_str().map(|s| s.to_string()),
            app: parsed["app"].as_str().map(|s| s.to_string()),
            project: parsed["project"].as_str().map(|s| s.to_string()),
            rest: false,
        };

        append_hour_frame(data_root, &now, &frame);

        // 10min boundary check
        let bucket = (now.timestamp() / 600) * 600;
        let sfile = super::store::sum_file_for(data_root, &now, bucket);
        if !sfile.exists() {
            let _ = super::store::write_10min_summary(data_root, &now, cfg).await;
            let cfg_clone = cfg.clone();
            let root_clone = data_root.clone();
            tokio::spawn(async move {
                let _ = super::diary::regenerate_pending(&root_clone, &cfg_clone, 3, 30).await;
            });
        }

        return Ok(frame);
    }

    #[cfg(target_os = "windows")]
    {
        // ---- Windows: 单屏截屏，通过 PowerShell -WindowStyle Hidden + CREATE_NO_WINDOW 双保险隐藏窗口 ----
        let shot_path = shot_dir.join(format!("{base_stem}.png"));
        let shot_str = shot_path.to_string_lossy().to_string();
        // Windows 路径中的反斜杠在 PowerShell 字符串里无需转义；
        // 仅处理可能出现的双引号。
        // 双引号转义（Windows 路径里极少出现，但保险起见处理）
        let shot_arg = shot_str.replace('"', "\\\"");
        // 方案：把 C# 源写到临时 .cs 文件，再 Add-Type -Path 编译加载。
        // 这样完全避开 shell 单行化/换行/引号转义问题（之前 -TypeDefinition 多行内联
        // 被 PowerShell 压成单行，导致 C# 编译器类型推导失败）。
        let cs_code = r#"using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Windows.Forms;
using System.Runtime.InteropServices;
public class LLensCapture {
    [DllImport("user32.dll")] static extern int GetSystemMetrics(int i);
    [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
    public static void Capture(string path) {
        SetProcessDPIAware();
        // 主屏宽高：优先 GetSystemMetrics，失败时回退 SystemInformation（不依赖窗口句柄）
        int w = GetSystemMetrics(0);
        int h = GetSystemMetrics(1);
        if (w <= 0 || h <= 0) {
            System.Drawing.Rectangle b = SystemInformation.VirtualScreen;
            w = b.Width; h = b.Height;
        }
        if (w <= 0 || h <= 0) throw new System.Exception("cannot determine screen size");
        Bitmap bmp = new Bitmap(w, h);
        Graphics g = Graphics.FromImage(bmp);
        g.CopyFromScreen(0, 0, 0, 0, new Size(w, h));
        bmp.Save(path, ImageFormat.Png);
        g.Dispose();
        bmp.Dispose();
    }
}"#;
        // 写到临时 .cs 文件
        let tmp_cs = std::env::temp_dir().join(format!("llens_capture_{}.cs", std::process::id()));
        std::fs::write(&tmp_cs, cs_code)
            .map_err(|e| format!("write temp .cs: {e}"))?;
        let tmp_cs_arg = tmp_cs.to_string_lossy().replace('"', "\\\"");
        // PowerShell: Add-Type 从 .cs 文件编译 C# 类（注意：-Path 与 -Language 不能同用，
        // -Path 属于含 -ReferencedAssemblies 的参数集，去掉 -Language 即可）
        let ps_cmd = format!(
            "Add-Type -Path '{tmp_cs_arg}' -ReferencedAssemblies System.Drawing,System.Windows.Forms; [LLensCapture]::Capture('{shot_arg}'); Remove-Item '{tmp_cs_arg}' -ErrorAction SilentlyContinue",
        );
        let mut cmd = std::process::Command::new("powershell");
        cmd.arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-STA")
            .arg("-WindowStyle")
            .arg("Hidden")
            .arg("-Command")
            .arg(&ps_cmd);
        // 隐藏 PowerShell 控制台窗口（Windows 下不弹黑色命令框）
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let out = cmd
            .output()
            .map_err(|e| format!("spawn powershell: {e}"))?;
        let _ = tmp_cs; // 文件已由 PowerShell 里 Remove-Item 清理
        if !out.status.success() || !shot_path.exists() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(format!("powershell screenshot failed: {stderr}"));
        }

        let month = now.format("%Y-%m").to_string();
        let preview_rel = format!("screenshots/{month}/{base_stem}.jpg");
        let preview_path = shot_dir.join(format!("{base_stem}.jpg"));
        let preview_ok = make_preview(&shot_path, &preview_path).is_ok();

        let _ = std::fs::remove_file(&shot_path);

        let data_src = if preview_ok {
            vec![preview_path.clone()]
        } else {
            return Err("Windows: 截图预览生成失败，无法调用 LLM".into());
        };

        let data_urls = futures_join_paths(&data_src).await?;
        let time_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let parsed = llm_summarize(cfg, &data_urls, 1, &time_str).await;

        let frame = Frame {
            time: now.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
            image: String::new(),
            extra_images: Vec::new(),
            thumb: if preview_ok { preview_rel.clone() } else { String::new() },
            preview: if preview_ok { preview_rel.clone() } else { String::new() },
            extra_thumbs: Vec::new(),
            extra_previews: Vec::new(),
            summary5: parsed["summary5"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(|s| s.trim().to_string())
                        .take(5)
                        .collect()
                })
                .unwrap_or_default(),
            activity: parsed["activity"].as_str().map(|s| s.to_string()),
            app: parsed["app"].as_str().map(|s| s.to_string()),
            project: parsed["project"].as_str().map(|s| s.to_string()),
            rest: false,
        };

        append_hour_frame(data_root, &now, &frame);

        let bucket = (now.timestamp() / 600) * 600;
        let sfile = super::store::sum_file_for(data_root, &now, bucket);
        if !sfile.exists() {
            let _ = super::store::write_10min_summary(data_root, &now, cfg).await;
            let cfg_clone = cfg.clone();
            let root_clone = data_root.clone();
            tokio::spawn(async move {
                let _ = super::diary::regenerate_pending(&root_clone, &cfg_clone, 3, 30).await;
            });
        }

        return Ok(frame);
    }

    #[cfg(target_os = "macos")]
    {
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

        // 2) Generate per-display preview JPEGs (≤2048px, <1MB) → kept on disk.
        //    Main: {stem}.jpg ; Extra: {stem}_d{N}.jpg
        let month = now.format("%Y-%m").to_string();
        let main_preview = shot_dir.join(format!("{base_stem}.jpg"));
        let main_preview_rel = format!("screenshots/{month}/{base_stem}.jpg");
        let main_preview_ok = make_preview(&main_path, &main_preview).is_ok();
        let mut extra_preview_paths: Vec<std::path::PathBuf> = Vec::new();
        let mut extra_preview_rels: Vec<String> = Vec::new();
        for ep in extra_paths.iter() {
            let stem_extra = ep
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
                .trim_end_matches(".png")
                .to_string();
            let p_path = shot_dir.join(format!("{stem_extra}.jpg"));
            if make_preview(ep, &p_path).is_ok() {
                extra_preview_paths.push(p_path.clone());
                extra_preview_rels.push(format!("screenshots/{month}/{stem_extra}.jpg"));
            }
        }

        // 3) Compose a single timeline thumbnail from all per-display previews.
        //    Main preview (if available) + all extra previews → one horizontal row.
        let thumb_path = shot_dir.join(format!("{base_stem}_t.jpg"));
        let thumb_rel = format!("screenshots/{month}/{base_stem}_t.jpg");
        let mut display_imgs: Vec<std::path::PathBuf> = Vec::new();
        if main_preview_ok && main_preview.exists() {
            display_imgs.push(main_preview.clone());
        } else {
            // fall back to the raw main PNG if preview generation failed
            display_imgs.push(main_path.clone());
        }
        display_imgs.extend(extra_preview_paths.iter().cloned());
        let thumb = match compose_displays_thumb(&display_imgs, &thumb_path) {
            Ok(_path) => thumb_rel,
            Err(e) => {
                eprintln!("thumbnail composition failed: {e}");
                String::new()
            }
        };

        // 4) Delete the original full-size PNGs (LLM consumed them, previews written).
        let mut originals_to_delete: Vec<std::path::PathBuf> = vec![main_path.clone()];
        for ep in extra_paths.iter() {
            originals_to_delete.push(ep.clone());
        }
        for p in &originals_to_delete {
            let _ = std::fs::remove_file(p);
        }

        // 5) Run LLM summarization on the compressed preview JPEGs (same quality
        //    as before, but read from the kept-on-disk previews instead of the
        //    already-deleted original PNGs).
        let all_paths: Vec<std::path::PathBuf> = {
            let mut v = Vec::new();
            if main_preview_ok {
                v.push(main_preview.clone());
            } else {
                return Err("main preview generation failed; cannot summarize frame".into());
            }
            v.extend(extra_preview_paths.clone());
            v
        };
        let data_urls = futures_join_paths(&all_paths).await?;
        let time_str = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let parsed = llm_summarize(cfg, &data_urls, all_paths.len(), &time_str).await;

        let frame = Frame {
            time: now.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
            image: format!("screenshots/{month}/{base_stem}.png"),
            extra_images: extra_paths
                .iter()
                .map(|p| p
                    .file_name()
                    .map(|n| format!("screenshots/{month}/{}", n.to_string_lossy()))
                    .unwrap_or_default())
                .collect(),
            thumb,
            preview: if main_preview_ok { main_preview_rel.clone() } else { String::new() },
            extra_thumbs: Vec::new(),
            extra_previews: extra_preview_rels,
            summary5: parsed["summary5"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(|s| s.trim().to_string())
                        .take(5)
                        .collect()
                })
                .unwrap_or_default(),
            activity: parsed["activity"].as_str().map(|s| s.to_string()),
            app: parsed["app"].as_str().map(|s| s.to_string()),
            project: parsed["project"].as_str().map(|s| s.to_string()),
            rest: false,
        };

        // 6) Append to hour log.
        append_hour_frame(data_root, &now, &frame);

        // 7) 10-min boundary check: when the 10-minute slice ends, write the
        //    summary and trigger diary regeneration in the background.
        let bucket = (now.timestamp() / 600) * 600;
        let sfile = super::store::sum_file_for(data_root, &now, bucket);
        if !sfile.exists() {
            let _ = super::store::write_10min_summary(data_root, &now, cfg).await;
            let cfg_clone = cfg.clone();
            let root_clone = data_root.clone();
            tokio::spawn(async move {
                let _ = super::diary::regenerate_pending(&root_clone, &cfg_clone, 3, 30).await;
            });
        }

        return Ok(frame);
    }
}

/// Append a frame to its hour log file (creating it if needed).
fn append_hour_frame(data_root: &PathBuf, now: &chrono::DateTime<chrono::Local>, frame: &Frame) {
    let day = now.format("%Y-%m-%d").to_string();
    let hour_key = now.format("%Y-%m-%d %H:00").to_string();
    let month = now.format("%Y-%m").to_string();
    let log_dir = data_root.join("logs").join(month);
    let _ = std::fs::create_dir_all(&log_dir);
    let p = log_dir
        .join(format!("{day}_{:02}", now.format("%H").to_string().parse::<u32>().unwrap_or(0)))
        .with_extension("json");

    let mut hf: HourFile = if p.exists() {
        std::fs::read_to_string(&p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| HourFile { hour: hour_key.clone(), frames: Vec::new() })
    } else {
        HourFile { hour: hour_key, frames: Vec::new() }
    };
    hf.frames.push(frame.clone());
    write_hour_file(&p, &hf);
}

fn write_hour_file(p: &std::path::Path, hf: &HourFile) {
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(hf) {
        let _ = std::fs::write(p, json);
    }
}

/// Compress a source image into a preview JPEG (≤2048px wide, <1MB on disk).
fn make_preview(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    let img = image::open(src).map_err(|e| format!("open src: {e}"))?;
    let w = img.width();
    let h = img.height();
    let max_w: u32 = 2048;
    let resized = if w > max_w {
        img.resize(max_w, h * max_w / w, FilterType::Lanczos3)
    } else {
        img
    };
    let rgb = resized.to_rgb8();
    let (rw, rh) = (rgb.width(), rgb.height());
    let rgb_buf: Vec<u8> = rgb.into_raw();
    for q in [90u8, 80, 70, 60] {
        let mut buf: Vec<u8> = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q);
        encoder
            .write_image(&rgb_buf, rw, rh, image::ExtendedColorType::Rgb8)
            .map_err(|e| format!("preview encode: {e}"))?;
        if buf.len() <= 1_000_000 || q == 60 {
            std::fs::write(dst, &buf).map_err(|e| format!("write preview: {e}"))?;
            return Ok(());
        }
    }
    Ok(())
}

/// Compress raw screenshot bytes to JPEG for LLM payload (≤1280px, q70).
async fn compress_for_llm(raw: &[u8]) -> Result<(Vec<u8>, String), String> {
    let img = image::load_from_memory(raw).map_err(|e| format!("decode: {e}"))?;
    let w = img.width();
    let h = img.height();
    let max_w: u32 = 1280;
    let resized = if w > max_w {
        img.resize(max_w, h * max_w / w, FilterType::Lanczos3)
    } else {
        img
    };
    let rgb = resized.to_rgb8();
    let (rw, rh) = (rgb.width(), rgb.height());
    let rgb_buf: Vec<u8> = rgb.into_raw();
    let mut buf: Vec<u8> = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 70)
        .write_image(&rgb_buf, rw, rh, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("jpeg encode: {e}"))?;
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &buf);
    Ok((buf, format!("data:image/jpeg;base64,{b64}")))
}
