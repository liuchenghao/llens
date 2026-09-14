// LLENS unit tests

use chrono::{Duration, Local, NaiveDate};
use llens_lib::capture::{Frame, HourFile};
use llens_lib::diary::Diary;
use llens_lib::search;
use llens_lib::store;

fn tmp_root() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "llens_test_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn test_hour_file_roundtrip() {
    let root = tmp_root();
    let now = Local::now();
    let f = Frame {
        time: now.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        image: "screenshots/2025-09/180000.png".into(),
        summary5: vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
        activity: Some("工作".into()),
        app: Some("Xcode".into()),
        project: Some("llens".into()),
    };
    llens_lib::capture::append_hour_frame(&root, &now, &f);
    let hf: HourFile = {
        let p = root
            .join("logs")
            .join(now.format("%Y-%m").to_string())
            .join(now.format("%Y-%m-%d_%H").to_string())
            .with_extension("json");
        let s = std::fs::read_to_string(p).unwrap();
        serde_json::from_str(&s).unwrap()
    };
    assert_eq!(hf.frames.len(), 1);
    assert_eq!(hf.frames[0].app.as_deref(), Some("Xcode"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_10min_slice_boundary() {
    // bucket alignment: 18:00, 18:10, ... each independent slice
    let t1 = chrono::DateTime::from_timestamp(1_757_700_000, 0).unwrap(); // some epoch
    let bucket1 = (t1.timestamp() / 600) * 600;
    let t2 = chrono::DateTime::from_timestamp(bucket1 + 600, 0).unwrap();
    let bucket2 = (t2.timestamp() / 600) * 600;
    assert_eq!(bucket1 / 600, bucket2 / 600 - 1); // consecutive slices
}

#[test]
fn test_search_truncation() {
    let root = tmp_root();
    // create one diary with long text
    let day = NaiveDate::from_ymd_opt(2025, 9, 10).unwrap();
    let mut d = Diary::default();
    d.date = "2025-09-10".into();
    d.brief = "x".repeat(500); // > 200
    d.status = "done".into();
    llens_lib::diary::save(&root, &d).unwrap();

    let hits = search::search(&root, "diary", day, day + Duration::days(1), "x", 5);
    assert!(!hits.is_empty());
    assert!(hits[0].text.chars().count() <= 201); // 200 + ellipsis
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn test_config_default_key_present() {
    let root = tmp_root();
    let cfg = store::Config::load(&root);
    assert!(cfg.api_key.starts_with("sk-"));
    assert_eq!(cfg.model, "agnes-3.0-flash");
    let red = cfg.redacted();
    assert!(red.api_key_tail.len() == 6);
    let _ = std::fs::remove_dir_all(&root);
}
