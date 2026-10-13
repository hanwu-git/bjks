//! 服务层：共享状态与系统操作（原 Tauri commands 模块的原生版本）

use crate::config::settings::{load_config, save_config};
use crate::indexer::enumerate_volumes;
use crate::indexer::startup::{force_rescan, start_background_scan};
use crate::logger::app_log;
use crate::models::{AppConfig, FileEntry, IndexStatus, ScanProgress};
use crate::search::engine::SearchEngine;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// 全局共享状态（搜索引擎 + 扫描进度），跨线程与 UI 共用
#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Mutex<SearchEngine>>,
    pub is_scanning: Arc<Mutex<bool>>,
    pub scanned_files: Arc<Mutex<usize>>,
    pub progress: Arc<Mutex<ScanProgress>>,
}

impl AppState {
    pub fn new(
        engine: SearchEngine,
        needs_refresh: bool,
        entry_count: usize,
        total_volumes: usize,
    ) -> Self {
        let mut progress = ScanProgress::default();
        progress.total_volumes = total_volumes;
        Self {
            engine: Arc::new(Mutex::new(engine)),
            is_scanning: Arc::new(Mutex::new(needs_refresh)),
            scanned_files: Arc::new(Mutex::new(entry_count)),
            progress: Arc::new(Mutex::new(progress)),
        }
    }

    /// 当前索引状态（原 get_index_status 命令逻辑）
    pub fn index_status(&self) -> IndexStatus {
        let is_scanning = *self.is_scanning.lock().unwrap();
        let scanned_files = *self.scanned_files.lock().unwrap();
        let index_count = self.engine.lock().unwrap().get_index_count();
        let progress = self.progress.lock().unwrap();

        // 扫描中按「已完成盘数 / 总盘数」计算真实百分比
        let progress_percent = if is_scanning {
            if progress.total_volumes > 0 {
                progress.completed_volumes as f32 / progress.total_volumes as f32 * 100.0
            } else {
                0.0
            }
        } else if index_count > 0 {
            100.0
        } else {
            0.0
        };

        IndexStatus {
            is_scanning,
            scanned_files,
            total_estimated: 0,
            progress_percent,
            total_volumes: progress.total_volumes,
            completed_volumes: progress.completed_volumes,
            current_volume: progress.current_volume.clone(),
        }
    }

    /// 启动后台扫描线程刷新索引
    pub fn spawn_background_scan(&self, volumes: Vec<crate::indexer::VolumeInfo>, config: AppConfig) {
        start_background_scan(
            volumes,
            config,
            Arc::clone(&self.engine),
            Arc::clone(&self.scanned_files),
            Arc::clone(&self.is_scanning),
            Arc::clone(&self.progress),
        );
    }

    /// 强制重建索引（同步阻塞，调用方须放在后台线程执行）
    /// 返回 false 表示没有可用的盘符
    pub fn rescan(&self, config: &AppConfig) -> bool {
        let volumes = enumerate_volumes(&config.scan_drives);
        if volumes.is_empty() {
            app_log("重新扫描中止：没有可用的盘符");
            return false;
        }
        app_log(&format!(
            "开始重新扫描，盘符: {:?}",
            volumes.iter().map(|v| v.drive_letter.clone()).collect::<Vec<_>>()
        ));

        *self.is_scanning.lock().unwrap() = true;
        let entries = force_rescan(
            &volumes,
            config,
            &self.progress,
            &self.scanned_files,
        );

        let count = entries.len();
        app_log(&format!("重新扫描完成：{} 个文件", count));
        *self.engine.lock().unwrap() = SearchEngine::new(entries);
        *self.scanned_files.lock().unwrap() = count;
        *self.is_scanning.lock().unwrap() = false;
        true
    }
}

/// 用系统默认程序打开文件：交给 explorer 直开，避免 cmd start 闪黑窗
pub fn open_file(path: &str) -> bool {
    Command::new("explorer").arg(path).spawn().is_ok()
}

/// 打开目录：文件则打开其所在目录，目录则直接打开
pub fn open_folder(path: &str) -> bool {
    let folder = if std::path::Path::new(path).is_file() {
        std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string())
    } else {
        path.to_string()
    };

    Command::new("explorer").arg(&folder).spawn().is_ok()
}

/// 列出当前可访问的盘符（形如 "C:"），供设置界面选择参与索引的盘
pub fn list_volumes() -> Vec<String> {
    crate::indexer::list_available_drives()
}

/// 读取配置（供 UI 层使用）
pub fn get_config() -> AppConfig {
    load_config()
}

/// 保存配置（供 UI 层使用）
pub fn save_settings(config: &AppConfig) -> bool {
    save_config(config).is_ok()
}

/// Unix 时间戳 → 本地时区 "yyyy-MM-dd HH:mm"（win32 API 转换，替代 chrono）
pub fn format_datetime(ts: i64) -> String {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{
        FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime,
    };
    if ts <= 0 {
        return String::new();
    }
    // Unix 秒 → Windows FILETIME（100 纳秒，自 1601-01-01 起）
    let ft = ((ts + 11_644_473_600) as u64).wrapping_mul(10_000_000);
    let filetime = FILETIME {
        dwLowDateTime: ft as u32,
        dwHighDateTime: (ft >> 32) as u32,
    };
    unsafe {
        let mut utc = SYSTEMTIME {
            wYear: 0,
            wMonth: 0,
            wDayOfWeek: 0,
            wDay: 0,
            wHour: 0,
            wMinute: 0,
            wSecond: 0,
            wMilliseconds: 0,
        };
        if FileTimeToSystemTime(&filetime, &mut utc) == 0 {
            return String::new();
        }
        let mut local = utc; // 时区转换失败时回退 UTC
        SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute
        )
    }
}

/// 字节数 → 人类可读大小（与旧版 JS formatSize 输出一致）
pub fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".to_string();
    }
    const K: f64 = 1024.0;
    let sizes = ["B", "KB", "MB", "GB", "TB"];
    let b = bytes as f64;
    let i = ((b.ln() / K.ln()).floor() as usize).min(sizes.len() - 1);
    let v = b / K.powi(i as i32);
    let s = format!("{:.2}", v)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string();
    format!("{} {}", s, sizes[i])
}

/// 扩展名 → 图标字符（NotoEmoji 黑白字形，egui 默认字体自带）
pub fn file_icon(entry: &FileEntry) -> &'static str {
    if entry.is_dir {
        return "\u{1F4C1}"; // 📁
    }
    match entry.extension.as_ref() {
        "xls" | "xlsx" | "csv" => "\u{1F4CA}",  // 📊
        "ppt" | "pptx" => "\u{1F4CB}",          // 📋
        "pdf" => "\u{1F4D5}",                   // 📕
        "txt" | "md" | "log" | "ini" | "json" | "xml" | "yml" | "yaml" | "conf" => "\u{1F4DD}", // 📝
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tiff" => "\u{1F5BC}", // 🖼
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "wma" | "m4a" => "\u{1F3B5}", // 🎵
        "mp4" | "avi" | "mkv" | "mov" | "wmv" | "flv" | "webm" => "\u{1F3AC}", // 🎬
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" => "\u{1F4E6}",     // 📦
        "exe" | "msi" | "bat" | "cmd" | "ps1" => "\u{2699}",                   // ⚙
        _ => "\u{1F4C4}", // 📄（含 doc/docx 及其他未知类型）
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(100), "100 B");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1048576), "1 MB");
    }

    #[test]
    fn test_format_datetime() {
        // 2024-01-01 00:00:00 UTC = 1704067200，北京时间应为 08:00
        let s = format_datetime(1704067200);
        assert!(s.starts_with("2024-01-01 08:"), "北京时间输出: {}", s);
        assert_eq!(format_datetime(0), "");
    }
}
