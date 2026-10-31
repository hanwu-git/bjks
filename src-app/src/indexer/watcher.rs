//! 运行中索引自动更新：定时读取各 NTFS 卷的 USN 变更日志，增量更新内存索引与缓存
//!
//! 周期由设置「自动更新间隔（分钟）」控制（0=关闭，每轮重读配置即时生效）。
//! 非 NTFS 卷 / 权限不足 / 游标失效的盘自动跳过，等下次全量扫描兜底。

use super::usn::{self, VolUsnState};
use super::CACHE_FILE;
use crate::logger::{app_data_dir, app_log};
use crate::models::{AppConfig, FileEntry};
use crate::service::AppState;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub type FrnMaps = Arc<Mutex<HashMap<String, VolUsnState>>>;

/// 缓存文件路径（与 startup.rs 保持一致）
pub fn cache_path() -> PathBuf {
    app_data_dir().join(CACHE_FILE)
}

/// 新增文件的元数据：查实际属性（大小/修改时间）；文件已不存在返回 None
fn stat_entry(path: &str) -> Option<(u64, i64)> {
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileAttributesExW, WIN32_FILE_ATTRIBUTE_DATA,
    };
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut data: WIN32_FILE_ATTRIBUTE_DATA = std::mem::zeroed();
        if GetFileAttributesExW(wide.as_ptr(), 0, &mut data as *mut _ as *mut _) != 0 {
            let size = ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64;
            let ft = ((data.ftLastWriteTime.dwHighDateTime as u64) << 32)
                | data.ftLastWriteTime.dwLowDateTime as u64;
            // FILETIME（100ns 自 1601）→ Unix 秒
            let unix = (ft / 10_000_000) as i64 - 11_644_473_600;
            Some((size, unix))
        } else {
            None
        }
    }
}

fn extension_of(name: &str) -> Box<str> {
    match name.rfind('.') {
        Some(pos) if pos + 1 < name.len() => name[pos + 1..].to_lowercase().into_boxed_str(),
        _ => "".into(),
    }
}

/// 解析缓存 meta 里的游标串 "journal_id:next_usn"
fn parse_cursor(s: &str) -> Option<(u64, u64)> {
    let (id, next) = s.split_once(':')?;
    Some((id.parse().ok()?, next.parse().ok()?))
}

/// 初始化一个卷的增量状态：MFT 全枚举建路径平表 + 从缓存 meta 恢复游标
/// 返回 None 表示该盘不支持增量（非 NTFS / 权限不足）
fn init_volume(drive: &str) -> Option<VolUsnState> {
    let mut state = usn::enum_mft(drive)?;

    // 从缓存 meta 恢复上次游标：journal id 匹配才续接（补缓存生成后的变更）
    let meta_key = format!("usn_{}", drive.trim_end_matches(':'));
    match crate::indexer::cache::IndexCache::open(cache_path().to_str().unwrap_or_default()) {
        Ok(cache) => match cache.get_meta(&meta_key) {
            Some(saved) => match parse_cursor(&saved) {
                Some((sid, snext)) => {
                    if sid == state.journal_id {
                        app_log(&format!(
                            "卷 {} 恢复 USN 游标 {}（journal id 匹配，将从上次位置补变更）",
                            drive, snext
                        ));
                        state.next_usn = snext;
                    } else {
                        app_log(&format!(
                            "卷 {} USN 日志已重建（journal id 变化），以当前位置为增量起点，历史变更待全扫兜底",
                            drive
                        ));
                    }
                }
                None => {
                    app_log(&format!("卷 {} 缓存游标格式异常（{}），以当前位置为增量起点", drive, saved));
                }
            },
            None => {
                app_log(&format!("卷 {} 缓存无游标记录（{}），以当前位置为增量起点", drive, meta_key));
            }
        },
        Err(e) => {
            app_log(&format!("卷 {} 打开缓存读取游标失败（{}），以当前位置为增量起点", drive, e));
        }
    }
    Some(state)
}

/// 把一批变更写回缓存（独立短连接，失败仅记日志不影响内存索引）
fn persist_changes(
    drive: &str,
    batch: &usn::UsnBatch,
    renames_for_db: &[(String, String, bool, String, Box<str>)],
    created_entries: &[FileEntry],
) {
    let path = cache_path();
    let Ok(cache) = crate::indexer::cache::IndexCache::open(path.to_str().unwrap_or_default())
    else {
        return;
    };
    if let Err(e) = cache.apply_batch(
        &batch.deleted,
        created_entries,
        renames_for_db,
    ) {
        app_log(&format!("增量变更写缓存失败：{}", e));
        return;
    }
    let meta_key = format!("usn_{}", drive.trim_end_matches(':'));
    let _ = cache.set_meta(
        &meta_key,
        &format!("{}:{}", batch.journal_id(), batch.next_usn),
    );
}

/// 启动后台自动更新线程（程序运行期间按设置周期增量更新索引）
pub fn spawn_auto_update(state: AppState, maps: FrnMaps) {
    thread::spawn(move || {
        app_log("索引自动更新线程启动（USN 增量模式）");
        // 本进程内不再尝试的盘（无权限/非 NTFS/游标失效），避免每轮刷错误日志
        let mut disabled_drives: std::collections::HashSet<String> = std::collections::HashSet::new();
        loop {
            // 每轮重读配置，设置改动即时生效
            let minutes = crate::config::settings::load_config().auto_update_minutes;
            if minutes == 0 {
                thread::sleep(Duration::from_secs(60));
                continue;
            }
            thread::sleep(Duration::from_secs(minutes as u64 * 60));

            // 全量扫描进行中跳过本轮（扫描完成后映射会被重建）
            if *state.is_scanning.lock().unwrap() {
                app_log("全量扫描进行中，本轮增量更新跳过");
                continue;
            }

            let cfg: AppConfig = crate::config::settings::load_config();
            let started = Instant::now();
            let mut total_changed = 0usize;

            for drive in &cfg.scan_drives {
                let key = drive.trim_end_matches('\\').to_string();

                // 已判定不可增量的盘直接跳过（重启程序后重新尝试）
                if disabled_drives.contains(&key) {
                    continue;
                }

                // 该盘映射未就位 → 初始化（含 MFT 枚举，秒级）
                let need_init = !maps.lock().unwrap().contains_key(&key);
                if need_init {
                    match init_volume(&key) {
                        Some(s) => {
                            maps.lock().unwrap().insert(key.clone(), s);
                        }
                        None => {
                            // 非 NTFS / 权限不足（打开卷需管理员权限）：本进程内不再重试
                            disabled_drives.insert(key.clone());
                            continue;
                        }
                    }
                }

                // 读变更 → 归并 → 应用到引擎与缓存
                let batch = {
                    let mut m = maps.lock().unwrap();
                    let st = match m.get_mut(&key) {
                        Some(s) => s,
                        None => continue,
                    };
                    match usn::read_changes(&key, st) {
                        Some(records) => {
                            // 不再丢弃积压：单批上限 65536 条的读取是增量且廉价的，
                            // 归并 + 内存更新 + 缓存写回实测各在 1 秒内完成。
                            // 曾经的「超过 5000 条就整批跳过、只推进游标」会把电脑
                            // 正常使用几分钟产生的变更全部丢掉，索引永远追不上磁盘。
                            if !records.is_empty() {
                                app_log(&format!("卷 {} 读到 {} 条 USN 记录", key, records.len()));
                            }
                            usn::coalesce(&key, &records, st)
                        }
                        None => {
                            // 游标失效：移除状态，本进程内不再重试（重启或重建索引可恢复）
                            m.remove(&key);
                            disabled_drives.insert(key.clone());
                            app_log(&format!(
                                "卷 {} USN 游标失效，已停止该盘增量（重启或手动重建索引可恢复）",
                                key
                            ));
                            continue;
                        }
                    }
                };

                let journal_id = batch.journal_id();
                if batch.is_empty() {
                    // 无变更也更新缓存里的游标（轻量 set_meta）
                    let meta_key = format!("usn_{}", key.trim_end_matches(':'));
                    if let Ok(cache) = crate::indexer::cache::IndexCache::open(
                        cache_path().to_str().unwrap_or_default(),
                    ) {
                        let _ = cache.set_meta(&meta_key, &format!("{}:{}", journal_id, batch.next_usn));
                    }
                    continue;
                }

                // 组装引擎变更（屏蔽规则与全量扫描保持一致：
                // 否则运行中产生的 C:\Windows 等系统文件会被增量灌进索引，搜索被污染）
                let filter = crate::indexer::mft_scanner::ScanFilter::from_config(&cfg);
                let mut add: Vec<FileEntry> = Vec::new();
                for (_frn, path, is_dir) in &batch.created {
                    if filter.excludes_path(path, *is_dir) {
                        continue;
                    }
                    // 历史重放里的临时文件可能已被删除，取不到属性就跳过，避免幽灵条目
                    let (size, modified) = match stat_entry(path) {
                        Some(v) => v,
                        None => continue,
                    };
                    let name = path.rsplit('\\').next().unwrap_or(path);
                    add.push(FileEntry {
                        name: name.into(),
                        path: path.clone().into_boxed_str(),
                        extension: if *is_dir { "".into() } else { extension_of(name) },
                        size,
                        modified,
                        is_dir: *is_dir,
                    });
                }
                let remove: HashSet<String> = batch.deleted.iter().cloned().collect();
                let rename: HashMap<String, (String, String, bool)> = batch
                    .renamed
                    .iter()
                    .filter(|(_, new, is_dir)| !filter.excludes_path(new, *is_dir))
                    .map(|(old, new, is_dir)| {
                        let new_dir = new
                            .rsplit_once('\\')
                            .map(|(d, _)| d.to_string())
                            .unwrap_or_default();
                        let new_name = new.rsplit('\\').next().unwrap_or(new).to_string();
                        (old.clone(), (new_dir, new_name, *is_dir))
                    })
                    .collect();

                let changed = state.engine.lock().unwrap().apply_changes(add.clone(), &remove, &rename);
                total_changed += changed;

                // 缓存写回：改名前先取出旧条目的新路径信息供 UPDATE 使用
                let renames_for_db: Vec<(String, String, bool, String, Box<str>)> = batch
                    .renamed
                    .iter()
                    .filter(|(_, new, is_dir)| !filter.excludes_path(new, *is_dir))
                    .map(|(old, new, is_dir)| {
                        let name = new.rsplit('\\').next().unwrap_or(new);
                        (old.clone(), new.clone(), *is_dir, name.to_string(), extension_of(name))
                    })
                    .collect();
                persist_changes(&key, &batch, &renames_for_db, &add);
                app_log(&format!(
                    "卷 {} 增量完成：新增 {} 删除 {} 改名 {}（索引生效 {} 条），缓存已写回",
                    key,
                    add.len(),
                    batch.deleted.len(),
                    rename.len(),
                    changed
                ));

                // 更新内存映射游标（read_changes 已推进 state.next_usn，batch.next_usn 相同）
                let mut m = maps.lock().unwrap();
                if let Some(st) = m.get_mut(&key) {
                    st.next_usn = batch.next_usn;
                }
            }

            if total_changed > 0 {
                app_log(&format!(
                    "索引自动更新完成：{} 处变更，耗时 {:?}",
                    total_changed,
                    started.elapsed()
                ));
            }
        }
    });
}
