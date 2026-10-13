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

/// 检查 exe 同目录是否存在 Mesa 软件渲染 DLL（随完整版压缩包分发，不再内嵌进 exe）
/// OpenGL 是启动期静态导入，DLL 必须与 exe 同目录才会被优先加载；缺失时弹窗提示
fn check_mesa_dlls() {
    // 强制使用 llvmpipe 纯软件渲染：默认的 d3d12 驱动在无 GPU 环境下初始化会直接退出
    std::env::set_var("GALLIUM_DRIVER", "llvmpipe");

    let exe_dir = match std::env::current_exe() {
        Ok(p) => p.parent().map(|d| d.to_path_buf()).unwrap_or_default(),
        Err(e) => {
            app_log(&format!("Mesa DLL：无法定位 exe 目录: {}", e));
            return;
        }
    };
    let missing = ["opengl32.dll", "libgallium_wgl.dll"]
        .iter()
        .any(|name| !exe_dir.join(name).exists());
    if !missing {
        return;
    }
    app_log("警告：exe 目录缺少 Mesa 软件渲染 DLL（opengl32.dll / libgallium_wgl.dll）");
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONWARNING, MB_SETFOREGROUND,
        };
        let title: Vec<u16> = "本机快搜".encode_utf16().chain(std::iter::once(0)).collect();
        let text: Vec<u16> = "未找到 Mesa 软件渲染库（opengl32.dll / libgallium_wgl.dll）。\n\n\
无显卡或远程桌面环境需要这些文件才能显示界面，请下载完整版压缩包，解压到同一目录后运行。\n\n\
点击「确定」继续启动（配有独立显卡的电脑可正常运行）。"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        MessageBoxW(
            0, // HWND：无属主窗口
            text.as_ptr(),
            title.as_ptr(),
            MB_ICONWARNING | MB_SETFOREGROUND,
        );
    }
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
    check_mesa_dlls();
    app_log("=== 应用启动（v2 原生版，无 WebView2） ===");

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
    eframe::run_native(
        "BJKS",
        options,
        Box::new(move |cc| {
            ui::theme::install_fonts(&cc.egui_ctx);
            Ok(Box::new(BjksApp::new(cc, state)))
        }),
    )
    .expect("eframe 启动失败");
}
