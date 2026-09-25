#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod models;
mod indexer;
mod search;
mod config;
mod commands;

use commands::search::AppState;
use search::engine::SearchEngine;
use std::sync::{Arc, Mutex};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use indexer::VolumeInfo;
use indexer::startup::load_or_scan;
use indexer::watcher::FileWatcher;
use config::settings::load_config;

fn log_file() -> PathBuf {
    let app_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("本机快搜");
    fs::create_dir_all(&app_dir).ok();
    app_dir.join("app.log")
}

fn app_log(msg: &str) {
    let path = log_file();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap_or_else(|_| panic!("无法打开日志文件: {:?}", path));
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    writeln!(file, "[{}] {}", timestamp, msg).ok();
}

/// 枚举系统中的 NTFS 卷
fn enumerate_ntfs_volumes() -> Vec<VolumeInfo> {
    let mut volumes = Vec::new();
    
    // 遍历 A-Z 盘符
    for letter in b'A'..=b'Z' {
        let drive = format!("{}:\\", letter as char);
        let path = std::path::Path::new(&drive);
        
        // 检查驱动器是否就绪
        if path.exists() && path.is_dir() {
            // 尝试读取根目录来验证可访问性
            if fs::read_dir(&drive).is_ok() {
                // 获取卷标（简化处理）
                let volume_name = format!("{}:", letter as char);
                
                volumes.push(VolumeInfo {
                    drive_letter: drive,
                    volume_name,
                    total_size: 0,
                    is_ntfs: true, // MVP 阶段假设所有可访问驱动器都是 NTFS
                });
            }
        }
    }
    
    volumes
}

fn main() {
    app_log("=== 应用启动 ===");
    
    // 1. 加载配置
    let config = load_config();
    app_log(&format!("配置加载完成，排除路径: {:?}", config.excluded_paths));
    
    // 2. 枚举 NTFS 卷
    let volumes = enumerate_ntfs_volumes();
    app_log(&format!("发现 {} 个卷: {:?}", volumes.len(), volumes.iter().map(|v| &v.drive_letter).collect::<Vec<_>>()));
    
    // 3. 加载缓存（优先让窗口快速显示），需要时后台刷新
    let excludes = config.excluded_paths.clone();
    app_log("开始加载缓存或全量扫描...");
    let (entries, needs_refresh) = load_or_scan(&volumes, &excludes);
    let entry_count = entries.len();
    app_log(&format!("初始索引包含 {} 个文件", entry_count));

    // 4. 创建搜索引擎
    let engine = SearchEngine::new(entries);

    // 5. 创建共享状态
    let index = Arc::new(Mutex::new(Vec::new())); // 用于文件监控的索引引用
    let app_state = AppState {
        engine: Arc::new(Mutex::new(engine)),
        is_scanning: Arc::new(Mutex::new(needs_refresh)),
        scanned_files: Arc::new(Mutex::new(entry_count)),
    };

    // 6. 如果缓存过期或为空，启动后台扫描线程刷新索引
    if needs_refresh {
        let engine_arc = Arc::clone(&app_state.engine);
        let scanned_arc = Arc::clone(&app_state.scanned_files);
        let scanning_arc = Arc::clone(&app_state.is_scanning);
        let vols = volumes.clone();
        let exs = excludes.clone();
        indexer::startup::start_background_scan(vols, exs, engine_arc, scanned_arc, scanning_arc);
    }
    
    // 7. 启动文件监控（后台线程）
    let monitor_paths: Vec<String> = volumes.iter().map(|v| v.drive_letter.clone()).collect();
    let watcher_index = Arc::clone(&index);
    
    let _watcher = match FileWatcher::start(&monitor_paths, watcher_index) {
        Ok(w) => {
            app_log(&format!("文件监控已启动，监控 {} 个路径", monitor_paths.len()));
            Some(w)
        }
        Err(e) => {
            app_log(&format!("文件监控启动失败: {}", e));
            None
        }
    };
    
    // 8. 启动 Tauri 应用
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
            commands::settings::add_exclude_path,
            commands::settings::remove_exclude_path,
            commands::settings::rescan,
            commands::settings::open_file,
            commands::settings::open_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
