#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod models;
mod indexer;
mod search;
mod config;
mod commands;
mod logger;

use commands::search::AppState;
use config::settings::load_config;
use indexer::enumerate_volumes;
use indexer::startup::{load_or_scan, start_background_scan};
use logger::app_log;
use search::engine::SearchEngine;
use std::sync::{Arc, Mutex};

fn main() {
    app_log("=== 应用启动 ===");

    // 1. 加载配置
    let config = load_config();
    app_log(&format!("配置加载完成，排除路径: {:?}", config.excluded_paths));

    // 2. 枚举要扫描的卷（受 scan_drives 配置约束）
    let volumes = enumerate_volumes(&config.scan_drives);
    app_log(&format!(
        "发现 {} 个卷: {:?}",
        volumes.len(),
        volumes.iter().map(|v| &v.drive_letter).collect::<Vec<_>>()
    ));

    // 3. 加载缓存（优先让窗口快速显示），需要时后台刷新
    app_log("开始加载缓存或全量扫描...");
    let (entries, needs_refresh) = load_or_scan();
    let entry_count = entries.len();
    app_log(&format!("初始索引包含 {} 个文件", entry_count));

    // 4. 创建搜索引擎
    let engine = SearchEngine::new(entries);

    // 5. 创建共享状态
    let app_state = AppState {
        engine: Arc::new(Mutex::new(engine)),
        is_scanning: Arc::new(Mutex::new(needs_refresh)),
        scanned_files: Arc::new(Mutex::new(entry_count)),
    };

    // 6. 如果缓存过期或为空，启动后台扫描线程刷新索引
    if needs_refresh {
        start_background_scan(
            volumes,
            config,
            Arc::clone(&app_state.engine),
            Arc::clone(&app_state.scanned_files),
            Arc::clone(&app_state.is_scanning),
        );
    }

    // 7. 启动 Tauri 应用
    app_log("启动 Tauri 应用...");
    tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::about::get_app_version,
            commands::about::get_readme,
            commands::search::search,
            commands::search::get_index_status,
            commands::settings::get_config,
            commands::settings::save_settings,
            commands::settings::add_exclude_item,
            commands::settings::remove_exclude_item,
            commands::settings::rescan,
            commands::settings::open_file,
            commands::settings::open_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}