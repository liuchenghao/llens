use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use super::capture::Frame;
use chrono::{DateTime, Duration, Local, NaiveDate};

// ---------- Config ----------

/// User-editable app config, persisted at <data_root>/config.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub recording_enabled: bool,
    /// How many days back to auto-generate diaries (0 = today only, 1 = today + yesterday, etc.)
    pub diary_lookback_days: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: "sk-iKm9rZjMEyuW2lgJhELPGpeby3RWZAjzxEtpQaGiyDwQF0IQ".into(),
            base_url: "https://apihub.agnes-ai.com/v1".into(),
            model: "agnes-3.0-flash".into(),
            recording_enabled: true,
            diary_lookback_days: 1,
        }
    }
}

impl Config {
    fn path(root: &PathBuf) -> PathBuf {
        root.join("config.json")
    }

    /// Ensure config file exists (writes defaults if missing).
    pub fn ensure_file(root: &PathBuf) {
        let p = Self::path(root);
        if !p.exists() {
            if let Ok(cfg) = serde_json::to_string_pretty(&Config::default()) {
                let _ = std::fs::write(&p, cfg);
            }
        }
    }

    pub fn load(root: &PathBuf) -> Self {
        match std::fs::read_to_string(Self::path(root)) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => {
                Self::ensure_file(root);
                Self::default()
            }
        }
    }

    pub fn save(&self, root: &PathBuf) -> Result<(), String> {
        if let Some(dir) = Self::path(root).parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create dir: {e}"))?;
        }
        let mut s =
            serde_json::to_string_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        s.push('\n');
        let p = Self::path(root);
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, &s).map_err(|e| format!("write: {e}"))?;
        std::fs::rename(&tmp, &p).map_err(|e| format!("rename: {e}"))?;
        Ok(())
    }

    /// Redacted view for the frontend (never expose full key).
    pub fn redacted(&self) -> RedactedConfig {
        RedactedConfig {
            api_key_tail: self
                .api_key
                .chars()
                .rev()
                .take(6)
                .collect::<String>()
                .chars()
                .rev()
                .collect(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            recording_enabled: self.recording_enabled,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RedactedConfig {
    pub api_key_tail: String,
    pub base_url: String,
    pub model: String,
    pub recording_enabled: bool,
}

// ---------- 10-minute summaries ----------

/// Rolling 10-minute bucket summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct T10 {
    pub bucket: i64, // unix seconds of bucket start (10min alignment)
    pub from: String,
    pub to: String,
    pub narrative: String,
    pub activities: Vec<String>,
    pub top_apps: Vec<String>,
    pub category_breakdown: serde_json::Value,
    pub frame_count: u32,
}

pub fn sum_file_for(root: &PathBuf, now: &DateTime<Local>, bucket: i64) -> PathBuf {
    let n = bucket / 600;
    root.join("summaries")
        .join(now.format("%Y-%m").to_string())
        .join(format!("{}_10min_{n}.json", now.format("%H")))
}

/// Collect frames of one 10-min slice from the hour JSON files and write a summary.
pub async fn write_10min_summary(
    root: &PathBuf,
    now: &DateTime<Local>,
    cfg: &Config,
) -> Result<T10, String> {
    let bucket = (now.timestamp() / 600) * 600;
    let start = chrono::DateTime::from_timestamp(bucket, 0)
        .map(|t| t.with_timezone(&Local))
        .unwrap_or(*now);
    let end = start + Duration::minutes(10);
    let frames = collect_frames_in_range(root, &start, &end);

    let mut activities: Vec<String> = frames
        .iter()
        .filter_map(|f| f.activity.clone())
        .collect();
    activities.sort();
    activities.dedup();
    let mut app_counts: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();
    let mut cat: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for f in &frames {
        if let Some(a) = &f.app {
            if a != "unknown" {
                *app_counts.entry(a.clone()).or_insert(0) += 1;
            }
        }
        if let Some(c) = &f.activity {
            *cat.entry(c.clone()).or_insert(0) += 1;
        }
    }
    let mut top_apps: Vec<(String, u32)> = app_counts.into_iter().collect();
    top_apps.sort_by(|a, b| b.1.cmp(&a.1));
    let top_apps = top_apps.into_iter().take(5).map(|(k, _)| k).collect();

    let mut narrative = String::new();
    if !frames.is_empty() {
        let texts: Vec<String> = frames
            .iter()
            .take(30)
            .map(|f| {
                format!(
                    "{} | {} | {} | {}",
                    f.time,
                    f.app.clone().unwrap_or_default(),
                    f.activity.clone().unwrap_or_default(),
                    f.summary5.join(" ")
                )
            })
            .collect();
        let prompt = format!(
            "下面是 {} ~ {} 内每 20 秒一帧的屏幕活动记录（时间 | 应用 | 活动类型 | 摘要）：\n{}\n\n请用不超过 150 字的中文连贯叙述，总结用户这段时间在做什么。只输出叙述本身。",
            start.format("%H:%M"),
            end.format("%H:%M"),
            texts.join("\n")
        );
        match super::llm::chat(
            &reqwest::Client::new(),
            cfg,
            &[super::llm::text_msg(&prompt)],
        )
        .await
        {
            Ok(t) => narrative = t.trim().to_string(),
            Err(e) => eprintln!("10min narrative failed: {e}"),
        }
    }

    let t10 = T10 {
        bucket,
        from: start.format("%Y-%m-%dT%H:%M:%S").to_string(),
        to: end.format("%Y-%m-%dT%H:%M:%S").to_string(),
        narrative,
        activities,
        top_apps,
        category_breakdown: serde_json::to_value(&cat).unwrap_or_default(),
        frame_count: frames.len() as u32,
    };

    let p = sum_file_for(root, now, bucket);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let s = serde_json::to_string_pretty(&t10).unwrap_or_default();
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, s).map_err(|e| format!("write summary: {e}"))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("rename summary: {e}"))?;
    Ok(t10)
}

// ---------- frame collection ----------

/// All frames whose `time` falls in [start, end), loaded from logs/<month>/<day>_<hour>.json.
pub fn collect_frames_in_range(
    root: &PathBuf,
    start: &DateTime<Local>,
    end: &DateTime<Local>,
) -> Vec<Frame> {
    let mut out = Vec::new();
    let mut day = start.date_naive();
    let end_day = end.date_naive();
    while day <= end_day {
        collect_day_frames(root, day, &mut out, start, end);
        day = day + chrono::Duration::days(1);
    }
    out
}

fn collect_day_frames(
    root: &PathBuf,
    day: NaiveDate,
    out: &mut Vec<Frame>,
    range_start: &DateTime<Local>,
    range_end: &DateTime<Local>,
) {
    let month = day.format("%Y-%m").to_string();
    let log_dir = root.join("logs").join(&month);
    if !log_dir.exists() {
        return;
    }
    let day_str = day.to_string(); // "YYYY-MM-DD" (with dashes, matches writer)
    for h in 0..24 {
        let p = log_dir
            .join(format!("{day_str}_{h:02}"))
            .with_extension("json");
        if !p.exists() {
            continue;
        }
        if let Ok(s) = std::fs::read_to_string(&p) {
            if let Ok(hf) = serde_json::from_str::<super::capture::HourFile>(&s) {
                for f in hf.frames {
                    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&f.time) {
                        if t >= *range_start && t < *range_end {
                            out.push(f);
                        }
                    }
                }
            }
        }
    }
}

// ---------- range listing (for board) ----------

/// Load frames for a date range (inclusive start day .. exclusive end day).
pub fn list_frames_range(
    root: &PathBuf,
    start_day: NaiveDate,
    end_day_excl: NaiveDate,
) -> Vec<Frame> {
    let start = day_midnight(&start_day);
    let end = day_midnight(&end_day_excl);
    collect_frames_in_range(root, &start, &end)
}

/// Start of the given local day as a local DateTime.
fn day_midnight(d: &NaiveDate) -> chrono::DateTime<Local> {
    d.and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .earliest()
        .unwrap()
}

/// All T10 summaries in a date range.
pub fn list_t10_range(
    root: &PathBuf,
    start_day: NaiveDate,
    end_day_excl: NaiveDate,
) -> Vec<T10> {
    let mut out = Vec::new();
    let mut d = start_day;
    while d < end_day_excl {
        let month = d.format("%Y-%m").to_string();
        let dir = root.join("summaries").join(&month);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "json").unwrap_or(false) {
                    if let Ok(s) = std::fs::read_to_string(&p) {
                        if let Ok(t) = serde_json::from_str::<T10>(&s) {
                            if t.frame_count > 0 || !t.narrative.is_empty() {
                                out.push(t);
                            }
                        }
                    }
                }
            }
        }
        d = d + chrono::Duration::days(1);
    }
    out.sort_by_key(|t| t.bucket);
    out
}
