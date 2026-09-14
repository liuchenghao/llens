use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use serde::Serialize;

/// Global app state: recording flag (polled by the capture loop each tick).
pub struct AppState {
    pub recording: AtomicBool,
}

#[derive(Serialize)]
pub struct Status {
    pub recording: bool,
    pub data_root: String,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            recording: AtomicBool::new(true),
        }
    }

    /// Data root: ~/.screenlog
    pub fn data_root() -> PathBuf {
        std::env::var("HOME")
            .map(|h| PathBuf::from(h).join(".screenlog"))
            .unwrap_or_else(|_| PathBuf::from("./.screenlog"))
    }

    pub fn status(&self) -> Status {
        Status {
            recording: self.recording.load(Ordering::SeqCst),
            data_root: Self::data_root().to_string_lossy().to_string(),
        }
    }

    /// Start the background 20s capture loop (spawned once at setup).
    pub fn start_recorder(&self, handle: tauri::AppHandle) {
        let data_root = Self::data_root();
        let _ = std::fs::create_dir_all(&data_root);
        super::store::Config::ensure_file(&data_root);
        tauri::async_runtime::spawn(async move {
            super::capture::run_loop(handle).await;
        });
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
