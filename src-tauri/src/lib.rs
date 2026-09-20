pub mod capture;
pub mod cleanup;
pub mod diary;
mod llm;
pub mod qasearch;
pub mod search;
pub mod store;
pub mod tstate;
mod display;

use tauri::Manager;

pub mod api;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(tstate::AppState::new())
        .setup(|app| {
            let handle = app.handle().clone();
            let state: &tstate::AppState = &app.state();
            if state.recording.load(std::sync::atomic::Ordering::SeqCst) {
                state.start_recorder(handle.clone());
            }
            // PRD §4.3: on app start, auto-generate missing/stale diaries in
            // the background (days that have 10-min data but no fresh diary).
            let data_root = tstate::AppState::data_root();
            tauri::async_runtime::spawn(async move {
                let cfg = crate::store::Config::load(&data_root);
                let days = crate::diary::pending_days(
                    &data_root,
                    chrono::Local::now().date_naive(),
                    cfg.diary_lookback_days,
                );
                for d in days {
                    match crate::diary::generate_day(&data_root, &cfg, d, 30).await {
                        Ok(_) => eprintln!("auto-catchup: diary {d} generated"),
                        Err(e) => eprintln!("auto-catchup: diary {d} failed: {e}"),
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api::get_status,
            api::data_root,
            api::read_image_as_data_url,
            api::set_data_root,
            api::reset_data_root,
            api::set_recording,
            api::get_config,
            api::save_config,
            api::list_day,
            api::list_range,
            api::get_diary,
            api::list_diary_dates,
            api::regenerate_diaries,
            api::force_regenerate_day,
            api::smart_regenerate_day,
            api::run_cleanup,
            api::retry_frame_summary,
            api::qa_search,
            api::qa_plan,
            api::qa_answer,
            api::qa_refine,
            api::qa_jump,
            api::days_with_slices,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
