#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod models;
mod indexer;
mod search;
mod config;
mod commands;
mod logger;

use commands::search::AppState;
use config::settings::{load_config, save_config};
use indexer::enumerate_volumes;
use indexer::startup::{load_or_scan, start_background_scan};
use logger::app_log;
use models::ScanProgress;
use search::engine::SearchEngine;
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

/// 显示并聚焦主窗口（托盘左键单击、托盘菜单「显示主窗口」共用）
fn show_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn main() {
    app_log("=== 应用启动 ===");

    // 1. 加载配置
    let config = load_config();
    app_log(&format!("配置加载完成，排除路径: {:?}", config.excluded_paths));

    // 记录上次窗口尺寸（config 后续会被移动到后台扫描线程，需提前取出）
    let saved_width = config.window_width;
    let saved_height = config.window_height;

    // 窗口尺寸变化时暂存最新值，由后台线程节流写回配置
    let pending_size: Arc<Mutex<Option<(u32, u32)>>> = Arc::new(Mutex::new(None));
    let pending_for_event = Arc::clone(&pending_size);

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

    // 5. 创建共享状态（progress 预置总盘数，便于前端立即显示第 X/Y 个盘）
    let progress = Arc::new(Mutex::new(ScanProgress {
        total_volumes: volumes.len(),
        ..Default::default()
    }));

    let app_state = AppState {
        engine: Arc::new(Mutex::new(engine)),
        is_scanning: Arc::new(Mutex::new(needs_refresh)),
        scanned_files: Arc::new(Mutex::new(entry_count)),
        progress: Arc::clone(&progress),
    };

    // 6. 如果缓存过期或为空，启动后台扫描线程刷新索引
    if needs_refresh {
        start_background_scan(
            volumes,
            config,
            Arc::clone(&app_state.engine),
            Arc::clone(&app_state.scanned_files),
            Arc::clone(&app_state.is_scanning),
            progress,
        );
    }
    // 注意：config 已被移动到后台扫描线程，窗口相关配置需在移动前取出
    // （saved_width / saved_height 在上方已提前取出）

    // 后台线程：节流写回窗口尺寸（每 800ms 检查一次，避免拖拽时频繁写盘）
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(800));
        let size = pending_size.lock().unwrap().take();
        if let Some((w, h)) = size {
            let mut config = load_config();
            if config.window_width != w || config.window_height != h {
                config.window_width = w;
                config.window_height = h;
                if save_config(&config).is_ok() {
                    app_log(&format!("已记录窗口尺寸: {}x{}", w, h));
                }
            }
        }
    });

    // 7. 启动 Tauri 应用
    app_log("启动 Tauri 应用...");
    tauri::Builder::default()
        .manage(app_state)
        .setup(move |app| {
            // 恢复上次关闭时的窗口尺寸（0 表示未记录，沿用配置默认）
            if let Some(window) = app.get_webview_window("main") {
                if saved_width > 0 && saved_height > 0 {
                    let _ = window.set_size(tauri::PhysicalSize::new(saved_width, saved_height));
                    app_log(&format!("恢复窗口尺寸: {}x{}", saved_width, saved_height));
                }
            }

            // 创建托盘图标与右键菜单
            let show_item = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出程序", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            let mut tray_builder = TrayIconBuilder::new()
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("本机快搜")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    // 左键单击托盘图标：恢复主窗口
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon().cloned() {
                tray_builder = tray_builder.icon(icon);
            }
            tray_builder.build(app)?;
            app_log("托盘图标已创建");
            Ok(())
        })
        .on_window_event(move |window, event| match event {
            // 窗口尺寸变化：记录最新尺寸，交由后台线程节流写回
            tauri::WindowEvent::Resized(size) => {
                if let Ok(mut pending) = pending_for_event.lock() {
                    *pending = Some((size.width, size.height));
                }
            }
            // 关闭请求：按配置选择最小化到托盘或直接退出
            tauri::WindowEvent::CloseRequested { api, .. } => {
                let config = load_config();
                if config.close_action != "exit" {
                    api.prevent_close();
                    let _ = window.hide();
                    app_log("窗口关闭请求：已最小化到托盘");
                } else {
                    app_log("窗口关闭请求：直接退出程序");
                }
            }
            _ => {}
        })
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
            commands::settings::list_volumes,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}