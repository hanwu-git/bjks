use crate::indexer::cache::IndexCache;
use crate::indexer::mft_scanner::scan_volume_fast_with_progress;
use crate::indexer::{normalize_drive, VolumeInfo, CACHE_FILE, DEFAULT_SCAN_DRIVES};
use crate::logger::{app_data_dir, app_log};
use crate::models::{AppConfig, FileEntry, ScanProgress};
use crate::search::engine::SearchEngine;
use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 获取缓存文件路径
fn get_cache_path() -> PathBuf {
    app_data_dir().join(CACHE_FILE)
}

/// 当前 Unix 秒
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 缓存年龄（小时）：优先用库里记录的「索引建立时刻」，缺失时回退到文件修改时间。
/// 时间戳晚于当前系统时间（时钟回拨 / 文件时间异常）按「刚写入」处理，不因此判过期。
fn cache_age_hours(cache_path: &PathBuf, saved_at: Option<u64>) -> Option<u64> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let stamp = match saved_at {
        Some(t) => t,
        None => fs::metadata(cache_path)
            .ok()?
            .modified()
            .ok()?
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_secs(),
    };
    Some(now.saturating_sub(stamp) / 3600)
}

/// 缓存读取结果。把「没有缓存 / 库为空 / 读取失败」区分开：
/// 读取失败（被占用、临时错误）不该和「确实没有索引」一样静默触发全量重扫。
enum CacheLoad {
    Missing,
    Empty,
    Failed(String),
    Loaded {
        entries: Vec<FileEntry>,
        drives: Option<String>,
        saved_at: Option<u64>,
    },
}

/// 尝试从缓存加载索引与元数据；遇占用/临时错误先重试几次，避免一次抖动就整盘重扫
fn load_cache(cache_path: &PathBuf) -> CacheLoad {
    if !cache_path.exists() {
        return CacheLoad::Missing;
    }
    let mut last_err = String::from("未知错误");
    for attempt in 0..3u32 {
        if attempt > 0 {
            thread::sleep(Duration::from_millis(300));
        }
        let cache = match IndexCache::open(cache_path.to_str().unwrap_or_default()) {
            Ok(c) => c,
            Err(e) => {
                last_err = format!("打开缓存失败：{}", e);
                continue;
            }
        };
        match cache.load_all() {
            Ok(entries) if entries.is_empty() => return CacheLoad::Empty,
            Ok(entries) => {
                app_log(&format!("缓存加载成功：{} 个文件", entries.len()));
                return CacheLoad::Loaded {
                    entries,
                    drives: cache.get_meta("scan_drives"),
                    saved_at: cache
                        .get_meta("saved_at")
                        .and_then(|s| s.parse::<u64>().ok()),
                };
            }
            Err(e) => last_err = format!("缓存加载失败：{}", e),
        }
    }
    CacheLoad::Failed(last_err)
}

/// 保存索引到缓存，并记录本次索引使用的盘符与建立时刻
fn save_cache(entries: &[FileEntry], drives: &[String]) {
    let cache_path = get_cache_path();
    if let Ok(cache) = IndexCache::open(cache_path.to_str().unwrap_or_default()) {
        if let Err(e) = cache.create_table() {
            app_log(&format!("创建缓存表失败：{}", e));
        } else if let Err(e) = cache.save_entries(entries) {
            app_log(&format!("保存缓存失败：{}", e));
        } else if let Err(e) = cache.set_meta("scan_drives", &drives.join(",")) {
            app_log(&format!("记录缓存盘符失败：{}", e));
        } else if let Err(e) = cache.set_meta("saved_at", &now_secs().to_string()) {
            app_log(&format!("记录缓存建立时刻失败：{}", e));
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

/// 写入缓存的盘符清单：**以设置为准**，与启动比对时使用同一口径。
///
/// 旧实现这里传的是「枚举出的可访问卷」，而比对时用的是设置里的勾选列表。
/// 光驱（如 D:）没插盘时不可访问会被枚举过滤掉，于是缓存记成 "C:"、
/// 比对却是 "C:,D:"，每次启动都被判成「盘符变了」而全量重扫。
fn indexed_drives(config_drives: &[String]) -> Vec<String> {
    let source = if config_drives.is_empty() {
        DEFAULT_SCAN_DRIVES.join(",")
    } else {
        config_drives.join(",")
    };
    normalize_drive_list(&source)
}

/// 启动流程：优先加载缓存让窗口立即显示，需要时后台刷新
/// 返回 (索引条目, 是否正在后台刷新)
/// 缓存有效需同时满足：未超过设置的有效期（默认 24 小时），且缓存记录的盘符与当前勾选的盘符一致
pub fn load_or_scan(current_drives: &[String], expiry_hours: u64) -> (Vec<FileEntry>, bool) {
    let cache_path = get_cache_path();
    app_log(&format!("缓存路径: {:?}", cache_path));

    // 优先加载缓存（即使过期也加载，保证窗口快速显示）
    // 直接 move 出条目：旧写法对 27 万条索引再 clone 一份，两份同时存活到函数结束，
    // 白白多占约 50MB 峰值内存（字符串字段递归复制）。
    let (entries, cached_drives, saved_at) = match load_cache(&cache_path) {
        CacheLoad::Loaded {
            entries,
            drives,
            saved_at,
        } => (
            entries,
            normalize_drive_list(&drives.unwrap_or_default()),
            saved_at,
        ),
        CacheLoad::Empty => {
            app_log("缓存为空，将执行后台全量扫描");
            (Vec::new(), Vec::new(), None)
        }
        CacheLoad::Missing => {
            app_log("未找到缓存，将执行后台全量扫描");
            (Vec::new(), Vec::new(), None)
        }
        CacheLoad::Failed(err) => {
            app_log(&format!("缓存读取失败（已重试 3 次）：{}，将全量扫描兜底", err));
            (Vec::new(), Vec::new(), None)
        }
    };

    if entries.is_empty() {
        return (entries, true);
    }

    let age = cache_age_hours(&cache_path, saved_at);
    let cache_valid = age.map(|h| h < expiry_hours).unwrap_or(false);
    app_log(&format!(
        "缓存是否有效: {}（已建立 {} 小时，有效期 {} 小时）",
        cache_valid,
        age.map(|h| h.to_string()).unwrap_or_else(|| "未知".into()),
        expiry_hours
    ));

    if !cache_valid {
        app_log("缓存已过期，将在后台刷新索引");
        return (entries, true);
    }

    let current = indexed_drives(current_drives);
    if cached_drives.is_empty() {
        app_log("缓存未记录盘符信息，将在后台刷新以对齐当前勾选");
        return (entries, true);
    }
    if cached_drives != current {
        app_log(&format!(
            "缓存盘符 {:?} 与当前勾选 {:?} 不一致，将在后台刷新索引",
            cached_drives, current
        ));
        return (entries, true);
    }

    app_log("使用有效缓存，无需后台刷新");
    (entries, false)
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

        // 保存到缓存（记录本次索引使用的盘符，以设置为准，与启动比对同口径）
        let drives = indexed_drives(&config.scan_drives);
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

    // 保存新缓存（记录本次索引使用的盘符，以设置为准，与启动比对同口径）
    let drives = indexed_drives(&config.scan_drives);
    save_cache(&entries, &drives);

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexed_drives_follows_config() {
        // 关键回归：光驱没插盘时 enumerate_volumes 会把该盘过滤掉，
        // 但写入缓存的清单必须仍按设置记录，否则每次启动都误判「盘符变了」而全量重扫
        let drives = vec!["C:".to_string(), "D:".to_string()];
        assert_eq!(indexed_drives(&drives), vec!["C:", "D:"]);
        // 大小写、尾随反斜杠、乱序都要归一，保证与比对口径一致
        let messy = vec!["d:\\".to_string(), "c:".to_string()];
        assert_eq!(indexed_drives(&messy), vec!["C:", "D:"]);
        // 配置为空时与 enumerate_volumes 的默认盘保持一致
        assert_eq!(indexed_drives(&[]), vec!["C:", "D:"]);
    }

    #[test]
    fn test_cache_age_handles_clock_skew() {
        let dummy = PathBuf::from("not-exist.db");
        // 记录时刻晚于当前时间（时钟回拨 / 文件时间在未来）→ 按刚建立处理，不能下溢成过期
        let future = now_secs() + 86_400 * 30;
        assert_eq!(cache_age_hours(&dummy, Some(future)), Some(0));
        // 三天前的记录 → 约 72 小时
        let old = now_secs().saturating_sub(86_400 * 3);
        assert!(cache_age_hours(&dummy, Some(old)).unwrap() >= 71);
    }
}