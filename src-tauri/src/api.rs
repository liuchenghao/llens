use std::sync::atomic::Ordering;
use tauri::State;
use tauri::Emitter;
use chrono::{Duration, NaiveDate};
use serde::Serialize;

use crate::capture::Frame;
use crate::diary::Diary;
use crate::diary::DaySliceCount;
use crate::qasearch::{self, Plan, ConvTurn, QaAnswer};
use crate::search::Hit;
use crate::store::{Config, T10};
use crate::tstate::{self, AppState};

fn root() -> std::path::PathBuf {
    AppState::data_root()
}

fn cfg() -> Config {
    Config::load(&root())
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> tstate::Status {
    state.status()
}

#[tauri::command]
pub fn set_recording(state: State<'_, AppState>, on: bool) {
    state.recording.store(on, Ordering::SeqCst);
}

#[tauri::command]
pub fn get_config() -> Config {
    cfg()
}

#[tauri::command]
pub fn save_config(new: Config) -> Result<Config, String> {
    new.save(&root())?;
    Ok(new)
}

/// Frames of a calendar day (from hour JSON files).
#[tauri::command]
pub fn list_day(day: String) -> Vec<Frame> {
    let d = NaiveDate::parse_from_str(&day, "%Y-%m-%d")
        .unwrap_or_else(|_| chrono::Local::now().date_naive());
    let start = day_midnight(&d);
    let end = day_midnight(&(d + Duration::days(1)));
    crate::store::collect_frames_in_range(&root(), &start, &end)
}

/// Start of the given local day as a local DateTime.
fn day_midnight(d: &NaiveDate) -> chrono::DateTime<chrono::Local> {
    d.and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(chrono::Local)
        .earliest()
        .unwrap()
}

/// Returns the absolute data root (for frontend file-path construction).
#[tauri::command]
pub fn data_root() -> String {
    root().to_string_lossy().to_string()
}

/// Read a screenshot file and return its contents as a base64 data-URL.
/// Works regardless of asset-protocol scope, so image display survives
/// a user-changed data directory.
#[tauri::command]
pub fn read_image_as_data_url(path: String) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("empty image path".to_string());
    }
    let pb = std::path::Path::new(trimmed);
    if !pb.exists() {
        return Err(format!("file not found: {trimmed}"));
    }
    if pb.is_dir() {
        return Err(format!("path is a directory, not an image: {trimmed}"));
    }
    let bytes = std::fs::read(pb).map_err(|e| format!("read failed: {e}"))?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let ext = pb
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "image/png",
    };
    Ok(format!("data:{mime};base64,{b64}"))
}

/// Change the data directory. Persists override to <default_root>/data_root.txt
/// and immediately switches the running app to use it.
#[tauri::command]
pub fn set_data_root(new_path: String) -> Result<String, String> {
    let new_path = new_path.trim().to_string();
    if new_path.is_empty() {
        return Err("path cannot be empty".into());
    }
    let new_pb = std::path::PathBuf::from(&new_path);
    // Expand ~ if needed
    let new_pb = if new_path.starts_with('~') {
        let rest = new_path.strip_prefix('~').unwrap_or("");
        std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join(rest))
            .unwrap_or(new_pb.clone())
    } else {
        new_pb
    };
    std::fs::create_dir_all(&new_pb)
        .map_err(|e| format!("cannot create directory {:?}: {e}", new_pb))?;
    let default_root = crate::store::default_data_root();
    let p = default_root.join("data_root.txt");
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&p, new_pb.to_string_lossy().as_ref())
        .map_err(|e| e.to_string())?;
    Ok(new_pb.to_string_lossy().to_string())
}

/// Reset the data directory back to the default (~/.screenlog).
#[tauri::command]
pub fn reset_data_root() -> Result<String, String> {
    let p = crate::store::default_data_root().join("data_root.txt");
    let _ = std::fs::remove_file(&p);
    Ok(crate::store::default_data_root().to_string_lossy().to_string())
}

#[derive(Serialize)]
pub struct RangeData {
    pub frames: Vec<Frame>,
    pub t10: Vec<T10>,
    pub diaries: Vec<Diary>,
}

/// Aggregated data for a day range [start_day, end_day).
#[tauri::command]
pub fn list_range(start_day: String, end_day: String) -> RangeData {
    let s = NaiveDate::parse_from_str(&start_day, "%Y-%m-%d")
        .unwrap_or_else(|_| chrono::Local::now().date_naive());
    let e = NaiveDate::parse_from_str(&end_day, "%Y-%m-%d")
        .unwrap_or_else(|_| s + Duration::days(1));
    let mut diaries = Vec::new();
    let mut d = s;
    while d < e {
        if let Some(x) = crate::diary::load(&root(), d) {
            diaries.push(x);
        }
        d = d + chrono::Duration::days(1);
    }
    RangeData {
        frames: crate::store::list_frames_range(&root(), s, e),
        t10: crate::store::list_t10_range(&root(), s, e),
        diaries,
    }
}

/// Regenerate the AI summary for a single frame (screenshot still on disk).
#[tauri::command]
pub async fn retry_frame_summary(day: String, time: String) -> Result<Frame, String> {
    let d = NaiveDate::parse_from_str(&day, "%Y-%m-%d")
        .unwrap_or_else(|_| chrono::Local::now().date_naive());
    let start = day_midnight(&d);
    let end = day_midnight(&(d + Duration::days(1)));
    let frames = crate::store::collect_frames_in_range(&root(), &start, &end);
    let idx = frames
        .iter()
        .position(|f| f.time == time)
        .ok_or_else(|| format!("frame {time} not found in {day}"))?;
    let f = &frames[idx];
    let new_frame = crate::capture::retry_frame(&cfg(), &root(), f).await?;
    crate::capture::replace_frame_in_day(&root(), &f.time, &new_frame);
    Ok(new_frame)
}

/// Get one day's diary.
#[tauri::command]
pub fn get_diary(day: String) -> Result<Option<Diary>, String> {
    let d = NaiveDate::parse_from_str(&day, "%Y-%m-%d").map_err(|e| e.to_string())?;
    Ok(crate::diary::load(&root(), d))
}

/// Force-regenerate a specific day's diary (always re-runs the LLM),
/// ignoring whether it was already done. This is what the diary UI
/// "update" button uses. `min_task_minutes` 控制 top3 统计的任务时长阈值。
#[tauri::command]
pub async fn force_regenerate_day(
    day: String,
    min_task_minutes: Option<u32>,
) -> Result<Diary, String> {
    let d = NaiveDate::parse_from_str(&day, "%Y-%m-%d").map_err(|e| e.to_string())?;
    crate::diary::regenerate_day(&root(), &cfg(), d, true, min_task_minutes.unwrap_or(30)).await
}

/// Smart-regenerate a single day: if the day's 10-min data has grown since
/// the last diary generation, re-run the LLM; otherwise just return the
/// existing diary (cheap fast path, no LLM call). This is what the diary
/// UI calls on date-click so a freshly-arrived 10-min slice auto-refreshes
/// the day without the user having to click "refresh".
#[tauri::command]
pub async fn smart_regenerate_day(
    day: String,
    min_task_minutes: Option<u32>,
) -> Result<Diary, String> {
    let d = NaiveDate::parse_from_str(&day, "%Y-%m-%d").map_err(|e| e.to_string())?;
    crate::diary::regenerate_day(&root(), &cfg(), d, false, min_task_minutes.unwrap_or(30)).await
}

/// All diary dates that exist on disk, filtered to the configured lookback window.
#[tauri::command]
pub fn list_diary_dates() -> Vec<String> {
    let cfg = cfg();
    let today = chrono::Local::now().date_naive();
    let min_day = today - chrono::Duration::days(cfg.diary_lookback_days as i64);
    let mut out = Vec::new();
    let dir = root().join("diary");
    if let Ok(months) = std::fs::read_dir(&dir) {
        for m in months.flatten() {
            if !m.path().is_dir() {
                continue;
            }
            if let Ok(days) = std::fs::read_dir(m.path()) {
                for d in days.flatten() {
                    if d.path().extension().map(|x| x == "json").unwrap_or(false) {
                        if let Some(s) = d.file_name().to_str() {
                            let date_str = s.trim_end_matches(".json").to_string();
                            if let Ok(nd) = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
                                if nd >= min_day {
                                    out.push(date_str);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// Regenerate pending diaries (background-safe; returns how many completed).
#[tauri::command]
pub async fn regenerate_diaries(
    limit: Option<usize>,
    min_task_minutes: Option<u32>,
) -> Result<Vec<Diary>, String> {
    let out = crate::diary::regenerate_pending(
        &root(),
        &cfg(),
        limit.unwrap_or(5),
        min_task_minutes.unwrap_or(30),
    )
    .await;
    Ok(out)
}

/// Run a retention-based data cleanup (PRD §4.6). Returns a report of what was
/// removed so the Settings UI can show the effect. Retention days come from the
/// current config; a 0 day-count disables that layer.
#[tauri::command]
pub fn run_cleanup() -> Result<crate::cleanup::CleanupReport, String> {
    crate::cleanup::run_cleanup(&root(), &cfg())
}

/// List days that have 10-min data (within lookback window), each with slice
/// count and whether a diary file exists. Used by the diary calendar to mark
/// "待生成" days and by the auto-catchup on page load / date change.
#[tauri::command]
pub fn days_with_slices() -> Vec<crate::diary::DaySliceCount> {
    let cfg = cfg();
    let up_to = chrono::Local::now().date_naive();
    crate::diary::days_with_slices(&root(), up_to, cfg.diary_lookback_days)
}

/// One QA search round (local, no LLM). `limit` 为 0 时用 plan.limit，否则用该值（前端可动态调整）。
#[tauri::command]
pub fn qa_search(plan: Plan, limit: Option<i64>) -> Vec<Hit> {
    qasearch::search_round(&root(), &plan, limit.unwrap_or(0))
}

#[tauri::command]
pub async fn qa_plan(question: String, history: Option<Vec<ConvTurn>>) -> Result<Plan, String> {
    qasearch::plan(&cfg(), &question, history.as_deref()).await
}

#[tauri::command]
pub async fn qa_refine(
    question: String,
    prev_hits: Vec<Hit>,
    history: Option<Vec<ConvTurn>>,
) -> Result<Plan, String> {
    qasearch::refine(&cfg(), &question, &prev_hits, history.as_deref()).await
}

/// QA 最终回答：基于累积命中生成回答，并按 layer/time 定位到来源。
/// 返回结构体而非纯字符串，便于前端渲染可点引用。
#[tauri::command]
pub async fn qa_answer(
    question: String,
    hits: Vec<Hit>,
    history: Option<Vec<ConvTurn>>,
) -> Result<QaAnswer, String> {
    let text = qasearch::answer(&cfg(), &question, &hits, history.as_deref()).await?;
    let citations = qasearch::citations(&text, &hits);
    Ok(QaAnswer {
        text,
        citations,
    })
}

/// QA 跳转：点击来源引用时，通知前端切到对应 tab 并定位到该日期/帧。
#[tauri::command]
pub fn qa_jump(app: tauri::AppHandle, layer: String, time: String) -> Result<(), String> {
    let date = time.chars().take(10).collect::<String>(); // "YYYY-MM-DD"
    let target: &str = match layer.as_str() {
        "diary" => "diary",
        _ => "overview",
    };
    let _ = app.emit("qa_jump", serde_json::json!({ "layer": layer, "time": date, "target": target }));
    Ok(())
}
