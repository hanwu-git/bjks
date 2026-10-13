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

/// Mesa 软件渲染 DLL（llvmpipe），保证无 GPU / 远程桌面 / 虚拟机环境下也能正常显示界面
const MESA_OPENGL32: &[u8] = include_bytes!("../dlls/opengl32.dll");
const MESA_GALLIUM: &[u8] = include_bytes!("../dlls/libgallium_wgl.dll");

/// 把内嵌的 Mesa DLL 释放到 exe 同目录（OpenGL 是启动期静态导入，必须放 exe 旁才会被优先加载）
/// 返回 true 表示本次是首次释放（当前进程已绑定系统 GL，需要重启自身才能加载 Mesa）
fn ensure_mesa_dlls() -> bool {
    let exe_dir = match std::env::current_exe() {
        Ok(p) => p.parent().map(|d| d.to_path_buf()).unwrap_or_default(),
        Err(e) => {
            app_log(&format!("Mesa DLL：无法定位 exe 目录: {}", e));
            return false;
        }
    };
    // 强制使用 llvmpipe 纯软件渲染：默认的 d3d12 驱动在无 GPU 环境下初始化会直接退出
    std::env::set_var("GALLIUM_DRIVER", "llvmpipe");

    let dlls: [(&str, &[u8]); 2] = [
        ("opengl32.dll", MESA_OPENGL32),
        ("libgallium_wgl.dll", MESA_GALLIUM),
    ];
    let mut released = false;
    for (name, bytes) in dlls {
        let target = exe_dir.join(name);
        if target.exists() {
            continue;
        }
        released = true;
        let tmp = exe_dir.join(format!("{}.tmp", name));
        match std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &target)) {
            Ok(_) => app_log(&format!("Mesa DLL 已释放: {}", target.display())),
            Err(e) => app_log(&format!(
                "Mesa DLL 释放失败（{}，需保证 exe 目录可写）: {}",
                target.display(),
                e
            )),
        }
    }
    released
}

/// 首次运行时 Mesa DLL 刚释放、当前进程无法回退加载，需重启自身一次（用户无感）
fn restart_self() {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    app_log("首次释放 Mesa DLL，重启进程以加载软件渲染……");
    let _ = std::process::Command::new(&exe)
        .args(std::env::args().skip(1))
        .spawn();
    std::process::exit(0);
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
    if ensure_mesa_dlls() {
        restart_self();
    }
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
