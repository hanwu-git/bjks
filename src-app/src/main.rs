#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod indexer;
mod logger;
mod models;
mod search;
mod service;
mod ui;

use config::settings::load_config;
pub use eframe::egui;
use indexer::enumerate_volumes;
use indexer::startup::load_or_scan;
use logger::app_log;
use search::engine::SearchEngine;
use service::AppState;
use ui::BjksApp;

/// 设置 Mesa 软件渲染驱动选择（仅当随包分发的 Mesa DLL 被加载时才生效，走系统 OpenGL 时被忽略）
/// 必须早于 eframe 启动：默认 d3d12 驱动在无 GPU 环境下初始化会直接退出
fn set_render_env() {
    std::env::set_var("GALLIUM_DRIVER", "llvmpipe");
}

/// 记录随包渲染库状态到日志（仅日志，不做提示或阻断；是否真正需要由 eframe 启动结果决定）
fn log_mesa_status() {
    let has_mesa = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .map(|d| d.join("opengl32.dll").exists() && d.join("libgallium_wgl.dll").exists())
        .unwrap_or(false);
    app_log(&format!(
        "随包 Mesa 渲染库{}（缺失时走系统 OpenGL，无可用 GPU 的机器需完整版压缩包）",
        if has_mesa { "已就位" } else { "未找到" }
    ));
}

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
    set_render_env();
    app_log("=== 应用启动（v2 原生版，无 WebView2） ===");
    log_mesa_status();

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

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!("本机快搜 v{}", env!("CARGO_PKG_VERSION")))
        .with_inner_size([w, h])
        .with_min_inner_size([900.0, 600.0]);
    if let Some(ico) = icon {
        viewport = viewport.with_icon(std::sync::Arc::new(ico));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    // 6. 启动 eframe 应用
    app_log("启动 eframe 应用...");
    if let Err(e) = eframe::run_native(
        "BJKS",
        options,
        Box::new(move |cc| {
            ui::theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(BjksApp::new(cc, state)))
        }),
    ) {
        // GL 初始化失败（无显卡/远程桌面/虚拟机且无随包渲染库）才提示，独显机器不再误报弹窗
        app_log(&format!("eframe 启动失败: {}", e));
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                MessageBoxW, MB_ICONERROR, MB_SETFOREGROUND,
            };
            let title: Vec<u16> = "本机快搜"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let text: Vec<u16> = format!(
                "图形界面初始化失败：{}\n\n\
本机可能没有可用的 OpenGL（无显卡 / 远程桌面 / 虚拟机）。\n\
请下载完整版压缩包（BJKS-full.zip），将 BJKS.exe 与 opengl32.dll、libgallium_wgl.dll 放在同一目录后运行。",
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
