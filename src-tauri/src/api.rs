use std::sync::atomic::Ordering;
use tauri::State;
use chrono::{Duration, NaiveDate};
use serde::Serialize;

use crate::capture::Frame;
use crate::diary::Diary;
use crate::qasearch::{self, Plan};
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

/// All diary dates that exist on disk.
#[tauri::command]
pub fn list_diary_dates() -> Vec<String> {
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
                            out.push(s.trim_end_matches(".json").to_string());
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
pub async fn regenerate_diaries(limit: Option<usize>) -> Result<Vec<Diary>, String> {
    let out = crate::diary::regenerate_pending(&root(), &cfg(), limit.unwrap_or(5)).await;
    Ok(out)
}

/// One QA search round (local, no LLM).
#[tauri::command]
pub fn qa_search(plan: Plan) -> Vec<Hit> {
    qasearch::search_round(&root(), &plan)
}

#[tauri::command]
pub async fn qa_plan(question: String) -> Result<Plan, String> {
    qasearch::plan(&cfg(), &question).await
}

#[tauri::command]
pub async fn qa_refine(question: String, prev_hits: Vec<Hit>) -> Result<Plan, String> {
    qasearch::refine(&cfg(), &question, &prev_hits).await
}

#[tauri::command]
pub async fn qa_answer(question: String, hits: Vec<Hit>) -> Result<String, String> {
    qasearch::answer(&cfg(), &question, &hits).await
}
