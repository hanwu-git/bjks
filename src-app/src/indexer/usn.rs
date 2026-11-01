//! USN 变更日志与 MFT 枚举：NTFS 增量索引更新的底层封装
//!
//! - `enum_mft`：一次性枚举卷上全部文件记录（FRN → 父目录 + 文件名），用于把
//!   USN 变更记录还原成完整路径
//! - `read_changes`：从上次游标读取 USN 变更日志（新增/删除/改名），开销极小
//! - 游标 (usn_journal_id, next_usn) 持久化在缓存 meta 表，跨启动衔接

use crate::logger::app_log;
use std::collections::{HashMap, HashSet};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_HANDLE_EOF, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

// ---------- 自定义 IOCTL 与结构体（windows-sys 未封装的部分） ----------

const FSCTL_QUERY_USN_JOURNAL: u32 = 0x0009_00f4; // CTL_CODE(9, 61, METHOD_BUFFERED, FILE_ANY_ACCESS)（winioctl.h，无 FSCTL_GET_USN_JOURNAL_DATA）
const FSCTL_READ_USN_JOURNAL: u32 = 0x0009_00bb; // CTL_CODE(9, 46, METHOD_NEITHER, FILE_ANY_ACCESS)
const FSCTL_ENUM_USN_DATA: u32 = 0x0009_00b3; // CTL_CODE(9, 44, METHOD_NEITHER, FILE_ANY_ACCESS)

/// USN 变更原因标志（winioctl.h 权威值）
/// 注意：FILE_CREATE/DELETE/RENAME 的位远高于 0x1~0x8（后者是数据写/扩展/截断），
/// 早期实现误用 0x1/0x2/0x4/0x8 导致所有记录都被丢弃。
const USN_REASON_FILE_CREATE: u32 = 0x0000_0100;
const USN_REASON_FILE_DELETE: u32 = 0x0000_0200;
const USN_REASON_RENAME_OLD_NAME: u32 = 0x0000_1000;
const USN_REASON_RENAME_NEW_NAME: u32 = 0x0000_2000;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;

/// USN_RECORD_V2 头部长度（RecordLength/Major/Minor/FRN/ParentFRN/Usn/TimeStamp/
/// Reason/SourceInfo/SecurityId/FileAttributes/FileNameLength/FileNameOffset = 60 字节）
const USN_RECORD_HEADER_LEN: usize = 60;

/// 解析一段 USN_RECORD_V2 记录流（buf[start..end]），跳过 RecordLength=0 的坏记录。
/// 枚举 MFT 与读取变更日志共用同一解析器，避免两处偏移量再次不一致。
fn parse_records(buf: &[u8], start: usize, end: usize) -> Vec<MftRecord> {
    let mut out = Vec::new();
    let mut offset = start;
    while offset + USN_RECORD_HEADER_LEN <= end {
        let rec_len = u32::from_ne_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
        if rec_len < USN_RECORD_HEADER_LEN || offset + rec_len > end {
            break;
        }
        // USN_RECORD_V2 布局（winioctl.h）：FRN@8、ParentFRN@16、Reason@40、
        // FileAttributes@52、FileNameLength@56、FileNameOffset@58、FileName 紧随其后
        let frn = u64::from_ne_bytes(buf[offset + 8..offset + 16].try_into().unwrap());
        let parent = u64::from_ne_bytes(buf[offset + 16..offset + 24].try_into().unwrap());
        let reason = u32::from_ne_bytes(buf[offset + 40..offset + 44].try_into().unwrap());
        let attrs = u32::from_ne_bytes(buf[offset + 52..offset + 56].try_into().unwrap());
        let name_len = u16::from_ne_bytes(buf[offset + 56..offset + 58].try_into().unwrap()) as usize;
        let name_off = u16::from_ne_bytes(buf[offset + 58..offset + 60].try_into().unwrap()) as usize;
        if name_off >= USN_RECORD_HEADER_LEN && offset + name_off + name_len <= end {
            let u16s: Vec<u16> = buf[offset + name_off..offset + name_off + name_len]
                .chunks_exact(2)
                .map(|c| u16::from_ne_bytes(c.try_into().unwrap()))
                .collect();
            out.push(MftRecord {
                frn,
                parent_frn: parent,
                name: String::from_utf16_lossy(&u16s),
                is_dir: attrs & FILE_ATTRIBUTE_DIRECTORY != 0,
                reason,
            });
        }
        offset += rec_len; // RecordLength 已按 8 字节对齐
    }
    out
}

/// MFT 枚举请求参数（MFT_ENUM_DATA_V0）
#[repr(C)]
struct MftEnumDataV0 {
    start_file_reference_number: u64,
    low_usn: u64,
    high_usn: u64,
}

/// USN 日志读取请求参数（USN_JOURNAL_DATA_V0）
#[repr(C)]
struct UsnJournalDataV0 {
    start_usn: i64,
    reason_mask: u32,
    return_only_on_close: u32,
    timeout: u64,
    bytes_to_wait_for: u64,
    usn_journal_id: u64,
}

/// USN 日志状态查询结果（USN_JOURNAL_DATA_V0，7 字段共 56 字节）
#[repr(C)]
#[derive(Default)]
struct UsnJournalStateV0 {
    usn_journal_id: u64,
    first_usn: i64,
    next_usn: i64,
    lowest_valid_usn: i64,
    max_usn: i64,
    maximum_size: u64,
    allocation_delta: u64,
}

/// MFT 文件记录（USN_RECORD_V2，MFT 枚举与变更日志同格式）
#[derive(Clone, Debug)]
pub struct MftRecord {
    pub frn: u64,
    pub parent_frn: u64,
    pub name: String,
    pub is_dir: bool,
    /// 仅变更日志有意义：USN 变更原因标志
    pub reason: u32,
}

/// 目录节点：只记父 FRN 与自身名，不存完整路径。
///
/// 关键设计：USN 记录本身就带 (parent_frn, name)，所以只要知道每个目录挂在谁下面，
/// 任何路径都能现场拼出。因此全卷 71.8 万条记录里只需保存约 10 万条目录，
/// 文件条目完全不进表——沿用「全量平表」会额外吃 120MB+。
/// 附带好处：目录改名只需改它自己的 name，子目录路径自动跟随（旧实现要靠前缀重写，易漏）。
#[derive(Clone, Debug)]
pub struct DirNode {
    pub parent_frn: u64,
    pub name: Box<str>,
}

/// 单个卷的增量更新状态：目录树 + USN 游标
pub struct VolUsnState {
    pub dirs: HashMap<u64, DirNode>,
    pub journal_id: u64,
    pub next_usn: u64,
}

/// 卷根目录固定文件记录号（NTFS $Root）
const ROOT_FRN: u64 = 5;

/// NTFS 文件引用号 = 高 16 位序列号 + 低 48 位记录号。
/// 真实枚举里的根记录是 `0x0005_0000_0000_0005`（序列 5 + 记录 5），
/// 只比较裸 `5` 会让父链在根处断裂（整个路径都拼不出来）。
const FRN_INDEX_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

/// 是否为卷根（忽略序列号）
fn is_root_frn(frn: u64) -> bool {
    frn & FRN_INDEX_MASK == ROOT_FRN
}

/// 由记录集合建目录树（只保留目录，卷根自身不入表）
fn build_dirs(records: Vec<MftRecord>) -> HashMap<u64, DirNode> {
    records
        .into_iter()
        .filter(|r| r.is_dir && !is_root_frn(r.frn))
        .map(|r| {
            (
                r.frn,
                DirNode {
                    parent_frn: r.parent_frn,
                    name: r.name.into_boxed_str(),
                },
            )
        })
        .collect()
}

/// 拼上级路径与名字（上级路径若已带尾随分隔符则不再补）
fn join_path(dir: &str, name: &str) -> String {
    if dir.ends_with('\\') {
        format!("{}{}", dir, name)
    } else {
        format!("{}\\{}", dir, name)
    }
}

/// 从目录树向上回溯，拼出目录完整路径。
/// 卷根返回 `C:\`（带尾随分隔符），其余目录不带尾随分隔符。
/// 返回 None：父链断裂或异常成环（无此保护会把内存吃光 / 死循环）。
fn dir_path(dirs: &HashMap<u64, DirNode>, frn: u64, root: &str) -> Option<String> {
    if is_root_frn(frn) {
        return Some(root.to_string());
    }
    let mut names: Vec<&str> = Vec::new();
    let mut seen: HashSet<u64> = HashSet::new();
    let mut cur = frn;
    loop {
        if is_root_frn(cur) {
            break;
        }
        if !seen.insert(cur) {
            return None; // 成环
        }
        let node = dirs.get(&cur)?; // 父链断裂
        names.push(&node.name);
        cur = node.parent_frn;
        if names.len() > 256 {
            return None; // 深度异常
        }
    }
    let mut out = String::with_capacity(root.len() + names.iter().map(|n| n.len() + 1).sum::<usize>());
    out.push_str(root);
    for name in names.iter().rev() {
        out.push_str(name);
        out.push('\\');
    }
    out.pop(); // 去掉末尾分隔符，与历史平表格式一致（根是唯一例外）
    Some(out)
}

/// 打开卷句柄（\\.\C: 形式，需要管理权限查询 USN；失败返回 None 并记日志）
fn open_volume(drive: &str) -> Option<HANDLE> {
    let wide: Vec<u16> = format!("\\\\.\\{}", drive.trim_end_matches('\\'))
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let handle = CreateFileW(
            wide.as_ptr(),
            0x80000000 | 0x40000000, // GENERIC_READ | GENERIC_WRITE（USN 查询需要写权限位）
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            0, // hTemplateFile
        );
        if handle == 0 || handle == -1 {
            let err = GetLastError();
            let hint = if err == 5 {
                "拒绝访问：USN 增量更新需要以管理员身份运行，该盘将由全量扫描兜底"
            } else {
                "跳过增量更新"
            };
            app_log(&format!(
                "打开卷 {} 失败（GetLastError={}），{}",
                drive, err, hint
            ));
            None
        } else {
            Some(handle)
        }
    }
}

/// 查询 USN 日志状态（journal id + 当前 next_usn）
pub fn query_journal(drive: &str) -> Option<(u64, u64)> {
    let handle = open_volume(drive)?;
    let result = unsafe {
        let mut out = UsnJournalStateV0::default();
        let ok = DeviceIoControl(
            handle,
            FSCTL_QUERY_USN_JOURNAL,
            std::ptr::null_mut(),
            0,
            &mut out as *mut _ as *mut _,
            std::mem::size_of::<UsnJournalStateV0>() as u32,
            &mut 0u32,
            std::ptr::null_mut(),
        );
        if ok == 0 {
            app_log(&format!(
                "查询 {} USN 日志失败（GetLastError={}，非 NTFS 卷或权限不足）",
                drive,
                GetLastError()
            ));
            None
        } else {
            Some((out.usn_journal_id, out.next_usn as u64))
        }
    };
    unsafe { CloseHandle(handle) };
    result
}

/// 枚举 MFT 建 FRN → 完整路径平表（全卷一次，百万级文件秒级完成）
pub fn enum_mft(drive: &str) -> Option<VolUsnState> {
    let (journal_id, next_usn) = query_journal(drive)?;
    let handle = open_volume(drive)?;

    // 边枚举边丢弃文件记录：只保留目录，临时内存从 ~70MB 降到 ~10MB
    let mut dir_records: Vec<MftRecord> = Vec::new();
    let mut total_records: usize = 0;
    unsafe {
        let mut med = MftEnumDataV0 {
            start_file_reference_number: 0,
            low_usn: 0,
            // USN 为有符号数（LARGE_INTEGER），传 u64::MAX 会变成 -1 被内核视为空区间（枚举 0 条）
            high_usn: i64::MAX as u64,
        };
        let mut buf = vec![0u8; 1 << 20]; // 1MB 输出缓冲
        loop {
            let mut returned = 0u32;
            let ok = DeviceIoControl(
                handle,
                FSCTL_ENUM_USN_DATA,
                &mut med as *mut _ as *mut _,
                std::mem::size_of::<MftEnumDataV0>() as u32,
                buf.as_mut_ptr() as *mut _,
                buf.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            );
            if ok == 0 {
                let err = GetLastError();
                if err != ERROR_HANDLE_EOF {
                    app_log(&format!(
                        "枚举 {} MFT 失败（GetLastError={}），该盘不做增量更新",
                        drive, err
                    ));
                    CloseHandle(handle);
                    return None;
                }
                break;
            }
            if returned < 8 {
                break;
            }
            // 输出前 8 字节是续枚举的 start_file_reference_number，其后为记录流
            let parsed = parse_records(&buf, 8, returned as usize);
            total_records += parsed.len();
            dir_records.extend(
                parsed
                    .into_iter()
                    .filter(|r| r.is_dir && !is_root_frn(r.frn)),
            );
            med.start_file_reference_number =
                u64::from_ne_bytes(buf[0..8].try_into().unwrap());
        }
        CloseHandle(handle);
    }

    let dirs = build_dirs(dir_records);
    app_log(&format!(
        "卷 {} MFT 枚举完成：总记录 {} 条，目录树 {} 条",
        drive,
        total_records,
        dirs.len()
    ));
    Some(VolUsnState {
        dirs,
        journal_id,
        next_usn,
    })
}

/// 从游标读取一批 USN 变更（上限 64K 条防止单轮过长），返回 (记录, 新游标)
/// 返回 None 表示游标失效（journal id 变化 / 日志截断），该盘需要全扫兜底
pub fn read_changes(drive: &str, state: &mut VolUsnState) -> Option<Vec<MftRecord>> {
    let handle = open_volume(drive)?;
    let mut out: Vec<MftRecord> = Vec::new();
    unsafe {
        let mut req = UsnJournalDataV0 {
            start_usn: state.next_usn as i64,
            reason_mask: 0xFFFF_FFFF,
            return_only_on_close: 0,
            timeout: 0,
            bytes_to_wait_for: 0,
            usn_journal_id: state.journal_id,
        };
        let mut buf = vec![0u8; 1 << 20];
        let mut laps: u32 = 0;
        loop {
            laps += 1;
            if laps > 500 {
                // 防御：单次读取不应超过 500 圈（每圈最多 65536 条），超出视为异常强制收尾
                app_log(&format!("读取 {} USN 变更超过 500 圈，强制收尾（已收集 {} 条）", drive, out.len()));
                break;
            }
            let mut returned = 0u32;
            let ok = DeviceIoControl(
                handle,
                FSCTL_READ_USN_JOURNAL,
                &mut req as *mut _ as *mut _,
                std::mem::size_of::<UsnJournalDataV0>() as u32,
                buf.as_mut_ptr() as *mut _,
                buf.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            );
            if ok == 0 {
                let err = GetLastError();
                if err != ERROR_HANDLE_EOF {
                    app_log(&format!(
                        "读取 {} USN 变更失败（GetLastError={}），游标失效",
                        drive, err
                    ));
                    CloseHandle(handle);
                    return None; // journal id 变化或日志截断，需要全扫兜底
                }
                break;
            }
            if returned < 8 {
                break;
            }
            // 只有 8 字节游标头 = 本轮已读到日志末尾（首圈也一样），结束
            if returned == 8 {
                break;
            }
            state.next_usn = u64::from_ne_bytes(buf[0..8].try_into().unwrap());
            out.extend(parse_records(&buf, 8, returned as usize));
            if out.len() >= 65_536 {
                break; // 本批到此为止，剩余下轮处理（next_usn 已推进）
            }
            req.start_usn = state.next_usn as i64;
        }
        CloseHandle(handle);
    }
    Some(out)
}

/// 从 USN 记录解析出一条变更对索引的实际影响（供 watcher 使用）
#[derive(Debug, Default)]
pub struct UsnBatch {
    /// (frn, 完整路径, 是否目录)：新增
    pub created: Vec<(u64, String, bool)>,
    /// 完整路径：删除
    pub deleted: Vec<String>,
    /// 旧路径 → 新路径：改名/移动（目录改名为整树前缀重写）
    pub renamed: Vec<(String, String, bool)>,
    /// 新游标（next_usn）
    pub next_usn: u64,
    /// 读取时校验通过的 journal id（持久化游标用）
    pub journal_id: u64,
}

impl UsnBatch {
    pub fn is_empty(&self) -> bool {
        self.created.is_empty() && self.deleted.is_empty() && self.renamed.is_empty()
    }

    pub fn journal_id(&self) -> u64 {
        self.journal_id
    }
}

/// 把一批原始 USN 记录归并成 UsnBatch（同批内 RENAME_OLD/NEW 配对）
pub fn coalesce(drive: &str, records: &[MftRecord], state: &mut VolUsnState) -> UsnBatch {
    let root = format!("{}\\", drive.trim_end_matches('\\'));
    let mut batch = UsnBatch::default();
    batch.next_usn = state.next_usn;
    batch.journal_id = state.journal_id;

    // 同批内的 RENAME_OLD_NAME 暂存：frn → 旧路径（无法还原时为空串）
    let mut pending_old: HashMap<u64, String> = HashMap::new();
    // 同一个文件在一次写入过程中会产生多条 CREATE 记录（写入/关闭各带一次），
    // 按 FRN 去重，避免索引里出现重复条目
    let mut seen_created: HashSet<u64> = HashSet::new();
    // 本批内已配对改名的 FRN：其后的重复 NEW_NAME 记录（如 CLOSE 事件）不应再当新增
    let mut renamed_frn: HashSet<u64> = HashSet::new();
    // 批内父目录路径缓存；目录树一旦变动（建/改名/删）立即清空，避免用到过期路径
    let mut path_cache: HashMap<u64, Box<str>> = HashMap::new();

    for rec in records {
        let reason = rec.reason;
        if reason & USN_REASON_RENAME_OLD_NAME != 0 {
            // 旧路径 = 旧父目录路径 + 旧名，两项都由本条记录自带，无需查历史
            let old_path =
                resolve_path(&state.dirs, &mut path_cache, rec.parent_frn, &rec.name, &root)
                    .unwrap_or_default();
            pending_old.insert(rec.frn, old_path);
        }
        if reason & USN_REASON_RENAME_NEW_NAME != 0 {
            let old_path = pending_old.remove(&rec.frn);
            match old_path {
                Some(old) => {
                    // 改名/移动：目录只更新自身的父与名，子目录路径自动跟随
                    if let Some(new_path) =
                        resolve_path(&state.dirs, &mut path_cache, rec.parent_frn, &rec.name, &root)
                    {
                        if !old.is_empty() {
                            batch.renamed.push((old, new_path, rec.is_dir));
                        }
                        if rec.is_dir {
                            state.dirs.insert(
                                rec.frn,
                                DirNode {
                                    parent_frn: rec.parent_frn,
                                    name: rec.name.as_str().into(),
                                },
                            );
                            path_cache.clear();
                        }
                        renamed_frn.insert(rec.frn);
                    }
                }
                None => {
                    // 无 OLD 的 NEW_NAME：从其他卷移入视为新增；本批已改过名的忽略
                    if !renamed_frn.contains(&rec.frn) {
                        if let Some(new_path) = resolve_path(
                            &state.dirs,
                            &mut path_cache,
                            rec.parent_frn,
                            &rec.name,
                            &root,
                        ) {
                            if rec.is_dir {
                                state.dirs.insert(
                                    rec.frn,
                                    DirNode {
                                        parent_frn: rec.parent_frn,
                                        name: rec.name.as_str().into(),
                                    },
                                );
                                path_cache.clear();
                            }
                            if seen_created.insert(rec.frn) {
                                batch.created.push((rec.frn, new_path, rec.is_dir));
                            }
                        }
                    }
                }
            }
        } else if reason & USN_REASON_FILE_CREATE != 0 {
            if let Some(new_path) =
                resolve_path(&state.dirs, &mut path_cache, rec.parent_frn, &rec.name, &root)
            {
                // 新建目录要进目录树，否则它下面的条目无法解析路径
                if rec.is_dir {
                    state.dirs.insert(
                        rec.frn,
                        DirNode {
                            parent_frn: rec.parent_frn,
                            name: rec.name.as_str().into(),
                        },
                    );
                    path_cache.clear();
                }
                if seen_created.insert(rec.frn) {
                    batch.created.push((rec.frn, new_path, rec.is_dir));
                }
            }
        } else if reason & USN_REASON_FILE_DELETE != 0 {
            // 删除：路径同样由本条记录的 (parent_frn, name) 现场拼出
            let path = resolve_path(&state.dirs, &mut path_cache, rec.parent_frn, &rec.name, &root)
                .unwrap_or_else(|| format!("{}{}", root, rec.name));
            batch.deleted.push(path);
            if rec.is_dir {
                state.dirs.remove(&rec.frn);
                path_cache.clear();
            }
        }
    }

    // 批尾仍未配对的 OLD_NAME = 纯删除
    for (frn, old_path) in pending_old {
        if !old_path.is_empty() {
            batch.deleted.push(old_path);
        }
        state.dirs.remove(&frn);
    }
    batch
}

/// 用目录树把 (parent_frn, name) 拼成完整路径；父链断裂时返回 None（跳过，下轮全扫兜底）
fn resolve_path(
    dirs: &HashMap<u64, DirNode>,
    cache: &mut HashMap<u64, Box<str>>,
    parent_frn: u64,
    name: &str,
    root: &str,
) -> Option<String> {
    if is_root_frn(parent_frn) {
        return Some(join_path(root, name));
    }
    let parent_path = match cache.get(&parent_frn) {
        Some(p) => p.to_string(),
        None => {
            let p = dir_path(dirs, parent_frn, root)?;
            cache.insert(parent_frn, p.as_str().into());
            p
        }
    };
    Some(join_path(&parent_path, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 按 USN_RECORD_V2 官方布局合成一条记录（供离线测试，不依赖 NTFS/管理员权限）
    fn build_record(frn: u64, parent: u64, reason: u32, name: &str, is_dir: bool) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let name_bytes = units.len() * 2;
        let name_off = USN_RECORD_HEADER_LEN;
        let rec_len = (name_off + name_bytes + 7) & !7; // RecordLength 按 8 字节对齐
        let mut b = vec![0u8; rec_len];
        b[0..4].copy_from_slice(&(rec_len as u32).to_ne_bytes());
        b[4..6].copy_from_slice(&2u16.to_ne_bytes()); // MajorVersion = 2
        b[6..8].copy_from_slice(&0u16.to_ne_bytes());
        b[8..16].copy_from_slice(&frn.to_ne_bytes());
        b[16..24].copy_from_slice(&parent.to_ne_bytes());
        b[24..32].copy_from_slice(&100i64.to_ne_bytes()); // Usn
        b[32..40].copy_from_slice(&0i64.to_ne_bytes()); // TimeStamp
        b[40..44].copy_from_slice(&reason.to_ne_bytes());
        b[44..48].copy_from_slice(&0u32.to_ne_bytes()); // SourceInfo
        b[48..52].copy_from_slice(&0u32.to_ne_bytes()); // SecurityId
        let attrs: u32 = if is_dir { FILE_ATTRIBUTE_DIRECTORY } else { 0 };
        b[52..56].copy_from_slice(&attrs.to_ne_bytes());
        b[56..58].copy_from_slice(&(name_bytes as u16).to_ne_bytes());
        b[58..60].copy_from_slice(&(name_off as u16).to_ne_bytes());
        for (i, u) in units.iter().enumerate() {
            let o = name_off + i * 2;
            b[o..o + 2].copy_from_slice(&u.to_ne_bytes());
        }
        b
    }

    /// 构造目录树状态：(FRN, 父 FRN, 目录名)，与 enumerate MFT 后的内存结构一致
    fn new_state(dirs: Vec<(u64, u64, &str)>) -> VolUsnState {
        let mut map: HashMap<u64, DirNode> = HashMap::new();
        for (frn, parent_frn, name) in dirs {
            map.insert(
                frn,
                DirNode {
                    parent_frn,
                    name: name.into(),
                },
            );
        }
        VolUsnState {
            dirs: map,
            journal_id: 1,
            next_usn: 0,
        }
    }

    #[test]
    fn test_parse_record_offsets() {
        // 缓冲区前 8 字节为续读游标，与真实 DeviceIoControl 输出一致
        let mut buf = vec![0u8; 8];
        buf.extend(build_record(42, 5, USN_REASON_FILE_CREATE, "报告.docx", false));
        buf.extend(build_record(7, 42, USN_REASON_FILE_CREATE, "子目录", true));

        let recs = parse_records(&buf, 8, buf.len());
        assert_eq!(recs.len(), 2, "应解析出 2 条记录");
        assert_eq!(recs[0].frn, 42);
        assert_eq!(recs[0].parent_frn, 5);
        assert_eq!(recs[0].name, "报告.docx");
        assert!(!recs[0].is_dir);
        assert_eq!(recs[0].reason, 0x0000_0100);
        assert!(recs[1].is_dir, "目录记录应识别 is_dir");
    }

    #[test]
    fn test_reason_constants_match_winioctl() {
        // 权威值来自 winioctl.h，误写成 0x1/0x2/0x4/0x8 会导致增删改全部漏判
        assert_eq!(USN_REASON_FILE_CREATE, 0x100);
        assert_eq!(USN_REASON_FILE_DELETE, 0x200);
        assert_eq!(USN_REASON_RENAME_OLD_NAME, 0x1000);
        assert_eq!(USN_REASON_RENAME_NEW_NAME, 0x2000);
    }

    #[test]
    fn test_coalesce_create() {
        let mut state = new_state(vec![
            (10, 5, "Users"),
            (11, 10, "hanwu"),
            (42, 11, "Documents"),
        ]);
        let rec = build_record(99, 42, USN_REASON_FILE_CREATE, "新建.txt", false);
        let recs = parse_records(&rec, 0, rec.len());
        let batch = coalesce("C:", &recs, &mut state);
        assert_eq!(batch.created.len(), 1, "CREATE 应产生一条新增");
        assert_eq!(batch.created[0].1, "C:\\Users\\hanwu\\Documents\\新建.txt");
        assert!(!state.dirs.contains_key(&99), "普通文件不进目录树（省内存的关键）");
    }

    #[test]
    fn test_coalesce_delete() {
        let mut state = new_state(vec![(42, 5, "Docs")]);
        let rec = build_record(99, 42, USN_REASON_FILE_DELETE, "旧.txt", false);
        let recs = parse_records(&rec, 0, rec.len());
        let batch = coalesce("C:", &recs, &mut state);
        assert_eq!(batch.deleted.len(), 1, "DELETE 应产生一条删除");
        assert_eq!(batch.deleted[0], "C:\\Docs\\旧.txt");
        assert!(state.dirs.contains_key(&42), "删除文件不应影响目录树");
    }

    #[test]
    fn test_coalesce_rename_pair() {
        let mut state = new_state(vec![(42, 5, "Docs")]);
        let old = build_record(99, 42, USN_REASON_RENAME_OLD_NAME, "旧名.txt", false);
        let new = build_record(99, 42, USN_REASON_RENAME_NEW_NAME, "新名.txt", false);
        let mut buf = old.clone();
        buf.extend(new);
        let recs = parse_records(&buf, 0, buf.len());
        let batch = coalesce("C:", &recs, &mut state);
        assert_eq!(batch.renamed.len(), 1, "OLD/NEW 应配对为一次改名");
        assert_eq!(batch.renamed[0].0, "C:\\Docs\\旧名.txt");
        assert_eq!(batch.renamed[0].1, "C:\\Docs\\新名.txt");
    }

    #[test]
    fn test_coalesce_rename_old_only_is_delete() {
        let mut state = new_state(vec![(42, 5, "Docs")]);
        let old = build_record(99, 42, USN_REASON_RENAME_OLD_NAME, "x.txt", false);
        let recs = parse_records(&old, 0, old.len());
        let batch = coalesce("C:", &recs, &mut state);
        assert_eq!(batch.deleted.len(), 1, "未配对的 OLD 应视为删除");
        assert_eq!(batch.deleted[0], "C:\\Docs\\x.txt");
    }

    #[test]
    fn test_coalesce_create_dedup() {
        // 一次写入会产生多条 CREATE 记录（写盘 + 关闭），同一 FRN 只能算一次新增
        let mut state = new_state(vec![(42, 5, "Docs")]);
        let mut buf = Vec::new();
        for _ in 0..3 {
            buf.extend(build_record(99, 42, USN_REASON_FILE_CREATE, "同文件.txt", false));
        }
        let recs = parse_records(&buf, 0, buf.len());
        let batch = coalesce("C:", &recs, &mut state);
        assert_eq!(batch.created.len(), 1, "同一 FRN 的重复 CREATE 应去重");
    }

    #[test]
    fn test_dir_tree_path() {
        // 真实枚举里的根记录 FRN 带序列号（0x0005_0000_0000_0005），不能只认裸 5
        const ROOT_SERIAL: u64 = 0x0005_0000_0000_0005;
        let records = vec![
            MftRecord { frn: ROOT_SERIAL, parent_frn: ROOT_SERIAL, name: ".".into(), is_dir: true, reason: 0 },
            MftRecord { frn: 10, parent_frn: ROOT_SERIAL, name: "Users".into(), is_dir: true, reason: 0 },
            MftRecord { frn: 11, parent_frn: 10, name: "hanwu".into(), is_dir: true, reason: 0 },
            MftRecord { frn: 12, parent_frn: 11, name: "a.txt".into(), is_dir: false, reason: 0 },
        ];
        let dirs = build_dirs(records);
        assert_eq!(dirs.len(), 2, "只有目录进树，卷根与文件都丢弃");
        assert_eq!(dir_path(&dirs, 11, "C:\\").as_deref(), Some("C:\\Users\\hanwu"));
        let mut cache = HashMap::new();
        assert_eq!(
            resolve_path(&dirs, &mut cache, 11, "a.txt", "C:\\").as_deref(),
            Some("C:\\Users\\hanwu\\a.txt")
        );
        // 挂直接卷根下的条目也必须拼对（旧实现会在根处断裂）
        assert_eq!(
            resolve_path(&dirs, &mut cache, ROOT_SERIAL, "根下文件.txt", "C:\\").as_deref(),
            Some("C:\\根下文件.txt")
        );
    }

    #[test]
    fn test_dir_rename_children_follow() {
        // 目录改名后，子目录只记父 FRN，路径应自动跟随（旧实现需要前缀重写，易漏）
        let mut state = new_state(vec![(10, 5, "Old"), (11, 10, "Child")]);
        let old = build_record(10, 5, USN_REASON_RENAME_OLD_NAME, "Old", true);
        let new = build_record(10, 5, USN_REASON_RENAME_NEW_NAME, "New", true);
        let mut buf = old.clone();
        buf.extend(new);
        let recs = parse_records(&buf, 0, buf.len());
        let batch = coalesce("C:", &recs, &mut state);
        assert_eq!(batch.renamed.len(), 1);
        assert_eq!(batch.renamed[0].0, "C:\\Old");
        assert_eq!(batch.renamed[0].1, "C:\\New");

        // 改名之后，子目录下的文件路径必须走新名字
        let rec = build_record(99, 11, USN_REASON_FILE_CREATE, "x.txt", false);
        let recs = parse_records(&rec, 0, rec.len());
        let batch = coalesce("C:", &recs, &mut state);
        assert_eq!(batch.created[0].1, "C:\\New\\Child\\x.txt");
    }

}
