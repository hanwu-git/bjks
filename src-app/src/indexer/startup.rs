use crate::indexer::cache::IndexCache;
use crate::indexer::mft_scanner::scan_volume_fast_with_progress;
use crate::indexer::{normalize_drive, VolumeInfo};
use crate::logger::{app_data_dir, app_log};
use crate::models::{AppConfig, FileEntry, ScanProgress};
use crate::search::engine::SearchEngine;
use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_FILE: &str = "file_index_v2.db";
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

/// 尝试从缓存加载索引，同时取出元数据（本次索引使用的盘符）
fn load_cache_with_meta(cache_path: &PathBuf) -> Option<(Vec<FileEntry>, Option<String>)> {
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
                    Some((entries, cache.get_meta("scan_drives")))
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

/// 保存索引到缓存，并记录本次索引使用的盘符
fn save_cache(entries: &[FileEntry], drives: &[String]) {
    let cache_path = get_cache_path();
    if let Ok(cache) = IndexCache::open(cache_path.to_str().unwrap()) {
        if let Err(e) = cache.create_table() {
            app_log(&format!("创建缓存表失败：{}", e));
        } else if let Err(e) = cache.save_entries(entries) {
            app_log(&format!("保存缓存失败：{}", e));
        } else if let Err(e) = cache.set_meta("scan_drives", &drives.join(",")) {
            app_log(&format!("记录缓存盘符失败：{}", e));
        } else {
            app_log(&format!(
                "缓存已保存：{} 个文件，盘符 {}",
                entries.len(),
                drives.join(",")
            ));
        }
    }
}

/// 盘符列表规范化：按逗号拆分、统一为 "X:"、去重排序，便于比较
fn normalize_drive_list(drives: &str) -> Vec<String> {
    let mut list: Vec<String> = drives
        .split(',')
        .map(normalize_drive)
        .filter(|d| d.len() > 1)
        .collect();
    list.sort();
    list.dedup();
    list
}

/// 启动流程：优先加载缓存让窗口立即显示，需要时后台刷新
/// 返回 (索引条目, 是否正在后台刷新)
/// 缓存有效需同时满足：未超过 24 小时，且缓存记录的盘符与当前勾选的盘符一致
pub fn load_or_scan(current_drives: &[String]) -> (Vec<FileEntry>, bool) {
    let cache_path = get_cache_path();
    app_log(&format!("缓存路径: {:?}", cache_path));

    let cache_valid = is_cache_valid(&cache_path);
    app_log(&format!("缓存是否有效: {}", cache_valid));

    // 优先加载缓存（即使过期也加载，保证窗口快速显示）
    let loaded = load_cache_with_meta(&cache_path);
    let entries = loaded
        .as_ref()
        .map(|(e, _)| e.clone())
        .unwrap_or_default();

    if cache_valid && !entries.is_empty() {
        let cached = loaded
            .as_ref()
            .and_then(|(_, meta)| meta.clone())
            .unwrap_or_default();
        let cached_drives = normalize_drive_list(&cached);
        let current = normalize_drive_list(&current_drives.join(","));

        if !cached_drives.is_empty() && cached_drives == current {
            app_log("使用有效缓存，无需后台刷新");
            return (entries, false);
        }
        if cached_drives.is_empty() {
            app_log("缓存未记录盘符信息，将在后台刷新以对齐当前勾选");
        } else {
            app_log(&format!(
                "缓存盘符 {:?} 与当前勾选 {:?} 不一致，将在后台刷新索引",
                cached_drives, current
            ));
        }
    }

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

        all.append(&mut entries);

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

        // 保存到缓存（记录本次索引使用的盘符）
        let drives: Vec<String> = volumes
            .iter()
            .map(|v| v.drive_letter.trim_end_matches('\\').to_string())
            .collect();
        save_cache(&entries, &drives);

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

    // 保存新缓存（记录本次索引使用的盘符）
    let drives: Vec<String> = volumes
        .iter()
        .map(|v| v.drive_letter.trim_end_matches('\\').to_string())
        .collect();
    save_cache(&entries, &drives);

    entries
}