use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use super::capture::Frame;
use super::diary::Diary;
use super::store::T10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    pub layer: String, // "frame" | "t10" | "diary"
    pub time: String,
    pub text: String, // truncated to ~200 chars
    pub source: String,
}

/// Substring search across the three layers, case-insensitive, truncated results.
pub fn search(
    root: &PathBuf,
    layer: &str,
    start: chrono::NaiveDate,
    end_excl: chrono::NaiveDate,
    keyword: &str,
    limit: usize,
) -> Vec<Hit> {
    if keyword.is_empty() {
        return vec![];
    }
    let kw = keyword.to_lowercase();
    let mut hits = Vec::new();
    match layer {
        "frame" => {
            for f in super::store::list_frames_range(root, start, end_excl) {
                if let Some(h) = frame_hit(&f, &kw) {
                    hits.push(h);
                }
                if hits.len() >= limit {
                    break;
                }
            }
        }
        "t10" => {
            for t in super::store::list_t10_range(root, start, end_excl) {
                if let Some(h) = t10_hit(&t, &kw) {
                    hits.push(h);
                }
                if hits.len() >= limit {
                    break;
                }
            }
        }
        "diary" => {
            let mut d = start;
            while d < end_excl {
                if let Some(d) = super::diary::load(root, d) {
                    if let Some(h) = diary_hit(&d, &kw) {
                        hits.push(h);
                    }
                }
                if hits.len() >= limit {
                    break;
                }
                d = d + chrono::Duration::days(1);
            }
        }
        _ => {}
    }
    hits
}

fn trunc(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n {
        s.to_string()
    } else {
        c[..n].iter().collect::<String>() + "…"
    }
}

fn frame_hit(f: &Frame, kw: &str) -> Option<Hit> {
    let hay = format!(
        "{} {} {} {}",
        f.activity.as_deref().unwrap_or(""),
        f.app.as_deref().unwrap_or(""),
        f.project.as_deref().unwrap_or(""),
        f.summary5.join(" ")
    )
    .to_lowercase();
    if hay.contains(kw) {
        Some(Hit {
            layer: "frame".into(),
            time: f.time.clone(),
            text: trunc(&f.summary5.join(" "), 200),
            source: f.image.clone(),
        })
    } else {
        None
    }
}

fn t10_hit(t: &T10, kw: &str) -> Option<Hit> {
    let hay = format!(
        "{} {} {}",
        t.narrative,
        t.activities.join(" "),
        t.top_apps.join(" ")
    )
    .to_lowercase();
    if hay.contains(kw) {
        Some(Hit {
            layer: "t10".into(),
            time: t.from.clone(),
            text: trunc(&t.narrative, 200),
            source: format!("t10:{}", t.bucket),
        })
    } else {
        None
    }
}

fn diary_hit(d: &Diary, kw: &str) -> Option<Hit> {
    let hay = format!(
        "{} {} {} {} {}",
        d.brief,
        d.top3.join(" "),
        d.highlights.join(" "),
        d.todos
            .iter()
            .map(|t| t.text.clone())
            .collect::<Vec<_>>()
            .join(" "),
        d.advice.join(" ")
    )
    .to_lowercase();
    if hay.contains(kw) {
        Some(Hit {
            layer: "diary".into(),
            time: d.date.clone(),
            text: trunc(&d.brief, 200),
            source: format!("diary:{}", d.date),
        })
    } else {
        None
    }
}
