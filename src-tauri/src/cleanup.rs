//! Retention-based cleanup (PRD §4.6): remove screenshots older than
//! `retention_image_days` and JSON logs/summaries older than `retention_json_days`.
//! Both are 0 = disabled.

use std::path::PathBuf;
use crate::store::Config;

#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
pub struct CleanupReport {
    pub removed_images: u64,
    pub removed_json: u64,
    pub freed_bytes: u64,
}

/// Remove expired data under `root` according to `cfg`.
///
/// Layout:
///   screenshots/<YYYY-MM>/<files>   — images (main .jpg / _t.jpg thumb / _dN.jpg)
///   logs/<YYYY-MM>/<day>_<h>.json   — hour JSON
///   summaries/<YYYY-MM>/<...>.json  — 10-min summaries
///
/// Files are named by their absolute time, so we delete any whose
/// embedded month is older than the cutoff. `image_cutoff`/`json_cutoff`
/// are NaiveDate; 0 retention disables that layer.
pub fn run_cleanup(
    root: &PathBuf,
    cfg: &Config,
) -> Result<CleanupReport, String> {
    let now = chrono::Local::now().date_naive();
    let img_cutoff: Option<chrono::NaiveDate> =
        (cfg.retention_image_days > 0).then(|| now - chrono::Duration::days(cfg.retention_image_days as i64));
    let json_cutoff: Option<chrono::NaiveDate> =
        (cfg.retention_json_days > 0).then(|| now - chrono::Duration::days(cfg.retention_json_days as i64));

    let mut report = CleanupReport::default();
    if let Some(c) = img_cutoff {
        clean_month_dir(&root.join("screenshots"), c, &mut report, true);
    }
    if let Some(c) = json_cutoff {
        clean_month_dir(&root.join("logs"), c, &mut report, false);
        clean_month_dir(&root.join("summaries"), c, &mut report, false);
    }
    Ok(report)
}

/// Walk `dir/<YYYY-MM>/` and remove files whose month (parsed from the dir name)
/// is strictly before `cutoff`. Empty month dirs are removed.
fn clean_month_dir(
    base: &PathBuf,
    cutoff: chrono::NaiveDate,
    report: &mut CleanupReport,
    is_image: bool,
) {
    let months = match std::fs::read_dir(base) {
        Ok(x) => x,
        Err(_) => return,
    };
    for m in months.flatten() {
        if !m.path().is_dir() {
            continue;
        }
        let name = match m.file_name().to_str().map(|s| s.to_string()) {
            Some(s) => s,
            None => continue,
        };
        // dir name is "YYYY-MM"
        let month_start = match chrono::NaiveDate::parse_from_str(&format!("{name}-01"), "%Y-%m-%d") {
            Ok(d) => d,
            Err(_) => continue,
        };
        if month_start >= cutoff {
            continue; // this month is within retention
        }
        // Entire month is expired → delete all files inside.
        if let Ok(files) = std::fs::read_dir(&m.path()) {
            for f in files.flatten() {
                if f.path().is_file() {
                    if let Ok(meta) = f.metadata() {
                        report.freed_bytes += meta.len();
                    }
                    let _ = std::fs::remove_file(f.path());
                    if is_image {
                        report.removed_images += 1;
                    } else {
                        report.removed_json += 1;
                    }
                }
            }
        }
        // Remove the now-empty month dir.
        let _ = std::fs::remove_dir(&m.path());
    }
}
