#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod indexer;
mod logger;
mod models;
mod search;
mod service;
mod sysicon;
mod ui;

use config::settings::load_config;
pub use egui;
use egui_software_backend::{SoftwareBackendAppConfiguration, run_app_with_software_backend};
use indexer::enumerate_volumes;
use indexer::startup::load_or_scan;
use logger::app_log;
use search::engine::SearchEngine;
use service::AppState;
use ui::BjksApp;

/// 从嵌入的 ico 加载应用图标（窗口 + 托盘共用）
pub fn load_app_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../icons/icon.ico");
    match image::load_from_memory(bytes) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            Some(egui::IconData {
                width: rgba.width(),
                height: rgba.height(),
                rgba: rgba.into_raw(),
            })
        }
        Err(e) => {
            app_log(&format!("图标加载失败: {}", e));
            None
        }
    }
}

fn main() {
    app_log("=== 应用启动（v3 纯 CPU 软件渲染版，无外部 DLL / 无 GPU 依赖） ===");

    // 1. 加载配置
    let config = load_config();
    app_log(&format!("配置加载完成，盘符: {:?}", config.scan_drives));

    // 2. 枚举卷 + 优先加载缓存（让窗口快速显示），需要时后台刷新
    let volumes = enumerate_volumes(&config.scan_drives);
    app_log(&format!(
        "发现 {} 个卷: {:?}",
        volumes.len(),
        volumes.iter().map(|v| &v.drive_letter).collect::<Vec<_>>()
    ));
    let (entries, needs_refresh) = load_or_scan(&config.scan_drives);
    let entry_count = entries.len();
    app_log(&format!("初始索引包含 {} 个文件", entry_count));

    // 3. 共享状态（progress 预置总盘数，便于立即显示第 X/Y 个盘）
    let state = AppState::new(
        SearchEngine::new(entries),
        needs_refresh,
        entry_count,
        volumes.len(),
    );
    if needs_refresh {
        state.spawn_background_scan(volumes, config.clone());
    }

    // 4. 窗口图标
    let icon = load_app_icon();

    // 5. 恢复上次关闭时的窗口尺寸（0 表示未记录，沿用默认）
    let (w, h) = if config.window_width > 0 && config.window_height > 0 {
        (config.window_width as f32, config.window_height as f32)
    } else {
        (1200.0, 800.0)
    };

    let mut settings = SoftwareBackendAppConfiguration::new()
        .title(Some(format!("本机快搜 v{}", env!("CARGO_PKG_VERSION"))))
        .inner_size(Some(egui::vec2(w, h)))
        .min_inner_size(Some(egui::vec2(900.0, 600.0)));
    if let Some(ico) = icon {
        settings = settings.icon(Some(std::sync::Arc::new(ico)));
    }

    // 6. 启动软件渲染应用（softbuffer 经系统 GDI 呈现，理论上任何 Windows 都可运行）
    app_log("启动软件渲染应用...");
    // 工厂类型为 FnMut（编译器按可能多次调用检查），用 RefCell<Option> 保证 state 只被取走一次
    let state_slot = std::rc::Rc::new(std::cell::RefCell::new(Some(state)));
    if let Err(e) = run_app_with_software_backend(settings, move |ctx| {
        let state = state_slot
            .borrow_mut()
            .take()
            .expect("应用工厂被重复调用");
        ui::theme::install_fonts(&ctx);
        BjksApp::new(ctx, state)
    }) {
        app_log(&format!("应用运行失败: {}", e));
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                MessageBoxW, MB_ICONERROR, MB_SETFOREGROUND,
            };
            let title: Vec<u16> = "本机快搜"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let text: Vec<u16> = format!(
                "程序窗口初始化失败：{}\n\n\
请尝试重启程序；若反复出现，请重启电脑后重试，或到 GitHub 下载历史版本。",
                e
            )
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
            MessageBoxW(0, text.as_ptr(), title.as_ptr(), MB_ICONERROR | MB_SETFOREGROUND);
        }
        std::process::exit(1);
    }
}
