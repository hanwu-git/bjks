use crate::indexer::cache::IndexCache;
use crate::indexer::mft_scanner::scan_volume_fast_with_progress;
use crate::indexer::VolumeInfo;
use crate::logger::{app_data_dir, app_log};
use crate::models::{AppConfig, FileEntry, ScanProgress};
use crate::search::engine::SearchEngine;
use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_FILE: &str = "file_index.db";
const CACHE_EXPIRY_HOURS: u64 = 24;

/// 获取缓存文件路径
fn get_cache_path() -> PathBuf {
    app_data_dir().join(CACHE_FILE)
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
pub fn load_or_scan() -> (Vec<FileEntry>, bool) {
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
    if entries.is_empty() {
        app_log("缓存为空，将执行后台全量扫描");
    } else {
        app_log("缓存已过期，将在后台刷新索引");
    }

    (entries, true)
}

/// 顺序扫描多个卷，并持续上报进度（按盘推进，便于给出真实百分比）
fn scan_volumes_with_progress(
    volumes: &[VolumeInfo],
    config: &AppConfig,
    progress: &Arc<Mutex<ScanProgress>>,
    scanned_files: &Arc<Mutex<usize>>,
) -> Vec<FileEntry> {
    {
        let mut p = progress.lock().unwrap();
        p.total_volumes = volumes.len();
        p.completed_volumes = 0;
        p.current_volume.clear();
        p.scanned_files = 0;
    }
    *scanned_files.lock().unwrap() = 0;

    let mut all: Vec<FileEntry> = Vec::new();
    let scanned = Cell::new(0usize);

    for vol in volumes {
        app_log(&format!("开始扫描卷：{}", vol.drive_letter));
        progress.lock().unwrap().current_volume = vol.drive_letter.clone();

        let on_progress = |count: usize| {
            let abs = scanned.get() + count;
            if let Ok(mut p) = progress.lock() {
                p.scanned_files = abs;
            }
            if let Ok(mut s) = scanned_files.lock() {
                *s = abs;
            }
        };

        let mut entries =
            scan_volume_fast_with_progress(&vol.drive_letter, config, &on_progress);
        scanned.set(scanned.get() + entries.len());

        // 重新编号，保证跨卷 ID 全局唯一
        let start_id = all.len() as u64;
        for (idx, entry) in entries.iter_mut().enumerate() {
            entry.id = start_id + idx as u64;
        }
        all.extend(entries);

        let mut p = progress.lock().unwrap();
        p.completed_volumes += 1;
        p.scanned_files = all.len();
        app_log(&format!(
            "卷 {} 扫描完成，累计 {} 个条目",
            vol.drive_letter,
            all.len()
        ));
    }

    *scanned_files.lock().unwrap() = all.len();
    all
}

/// 后台扫描并更新搜索引擎
pub fn start_background_scan(
    volumes: Vec<VolumeInfo>,
    config: AppConfig,
    engine: Arc<Mutex<SearchEngine>>,
    scanned_files: Arc<Mutex<usize>>,
    is_scanning: Arc<Mutex<bool>>,
    progress: Arc<Mutex<ScanProgress>>,
) {
    thread::spawn(move || {
        app_log("后台扫描线程启动");
        *is_scanning.lock().unwrap() = true;

        let entries = scan_volumes_with_progress(&volumes, &config, &progress, &scanned_files);
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

/// 强制重新扫描：删除旧缓存并全量重建（同步执行，会阻塞调用者）
pub fn force_rescan(
    volumes: &[VolumeInfo],
    config: &AppConfig,
    progress: &Arc<Mutex<ScanProgress>>,
    scanned_files: &Arc<Mutex<usize>>,
) -> Vec<FileEntry> {
    let cache_path = get_cache_path();

    // 删除旧缓存
    if cache_path.exists() {
        fs::remove_file(&cache_path).ok();
        app_log("已删除旧索引缓存");
    }

    // 执行全量扫描
    let entries = scan_volumes_with_progress(volumes, config, progress, scanned_files);

    // 保存新缓存
    save_cache(&entries);

    entries
}