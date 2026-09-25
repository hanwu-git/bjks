use crate::indexer::cache::IndexCache;
use crate::indexer::mft_scanner::scan_all_volumes;
use crate::indexer::VolumeInfo;
use crate::models::FileEntry;
use crate::search::engine::SearchEngine;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

fn app_log(msg: &str) {
    let app_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("本机快搜");
    fs::create_dir_all(&app_dir).ok();
    let path = app_dir.join("app.log");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap_or_else(|_| panic!("无法打开日志文件: {:?}", path));
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    writeln!(file, "[{}] {}", timestamp, msg).ok();
}

const CACHE_FILE: &str = "file_index.db";
const CACHE_EXPIRY_HOURS: u64 = 24;

/// 获取缓存文件路径
fn get_cache_path() -> PathBuf {
    let app_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("本机快搜");
    fs::create_dir_all(&app_dir).ok();
    app_dir.join(CACHE_FILE)
}

/// 检查缓存是否有效
fn is_cache_valid(cache_path: &PathBuf) -> bool {
    if !cache_path.exists() {
        return false;
    }

    if let Ok(metadata) = fs::metadata(cache_path) {
        if let Ok(modified) = metadata.modified() {
            if let Ok(duration) = modified.duration_since(UNIX_EPOCH) {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let cache_age_hours = (now - duration.as_secs()) / 3600;
                return cache_age_hours < CACHE_EXPIRY_HOURS;
            }
        }
    }
    false
}

/// 尝试从缓存加载索引
fn load_cache(cache_path: &PathBuf) -> Option<Vec<FileEntry>> {
    if !cache_path.exists() {
        return None;
    }
    match IndexCache::open(cache_path.to_str().unwrap()) {
        Ok(cache) => match cache.load_all() {
            Ok(entries) => {
                app_log(&format!("缓存加载成功：{} 个文件", entries.len()));
                if entries.is_empty() {
                    app_log("缓存为空");
                    None
                } else {
                    Some(entries)
                }
            }
            Err(e) => {
                app_log(&format!("缓存加载失败：{}", e));
                None
            }
        },
        Err(e) => {
            app_log(&format!("打开缓存失败：{}", e));
            None
        }
    }
}

/// 保存索引到缓存
fn save_cache(entries: &[FileEntry]) {
    let cache_path = get_cache_path();
    if let Ok(cache) = IndexCache::open(cache_path.to_str().unwrap()) {
        if let Err(e) = cache.create_table() {
            app_log(&format!("创建缓存表失败：{}", e));
        } else if let Err(e) = cache.save_entries(entries) {
            app_log(&format!("保存缓存失败：{}", e));
        } else {
            app_log(&format!("缓存已保存：{} 个文件", entries.len()));
        }
    }
}

/// 启动流程：优先加载缓存让窗口立即显示，需要时后台刷新
/// 返回 (索引条目, 是否正在后台刷新)
pub fn load_or_scan(_volumes: &[VolumeInfo], _excludes: &[String]) -> (Vec<FileEntry>, bool) {
    let cache_path = get_cache_path();
    app_log(&format!("缓存路径: {:?}", cache_path));

    let cache_valid = is_cache_valid(&cache_path);
    app_log(&format!("缓存是否有效: {}", cache_valid));

    // 优先加载缓存（即使过期也加载，保证窗口快速显示）
    let entries = load_cache(&cache_path).unwrap_or_default();

    // 如果缓存有效且非空，直接返回
    if cache_valid && !entries.is_empty() {
        app_log("使用有效缓存，无需后台刷新");
        return (entries, false);
    }

    // 缓存过期或为空，需要后台刷新
    let needs_refresh = true;
    if entries.is_empty() {
        app_log("缓存为空，将执行后台全量扫描");
    } else {
        app_log("缓存已过期，将在后台刷新索引");
    }

    (entries, needs_refresh)
}

/// 后台扫描并更新搜索引擎
pub fn start_background_scan(
    volumes: Vec<VolumeInfo>,
    excludes: Vec<String>,
    engine: Arc<Mutex<SearchEngine>>,
    scanned_files: Arc<Mutex<usize>>,
    is_scanning: Arc<Mutex<bool>>,
) {
    thread::spawn(move || {
        app_log("后台扫描线程启动");
        *is_scanning.lock().unwrap() = true;

        let entries = scan_all_volumes(&volumes, &excludes);
        let count = entries.len();
        app_log(&format!("后台扫描完成：{} 个文件", count));

        // 保存到缓存
        save_cache(&entries);

        // 更新搜索引擎和状态
        *engine.lock().unwrap() = SearchEngine::new(entries);
        *scanned_files.lock().unwrap() = count;
        *is_scanning.lock().unwrap() = false;

        app_log("后台索引更新完成");
    });
}

/// 强制重新扫描（同步执行，会阻塞调用者）
pub fn force_rescan(volumes: &[VolumeInfo], excludes: &[String]) -> Vec<FileEntry> {
    let cache_path = get_cache_path();

    // 删除旧缓存
    if cache_path.exists() {
        fs::remove_file(&cache_path).ok();
    }

    // 执行全量扫描
    let entries = scan_all_volumes(volumes, excludes);

    // 保存新缓存
    save_cache(&entries);

    entries
}
