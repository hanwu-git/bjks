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

/// 单个卷的增量更新状态：FRN → 完整路径 平表 + USN 游标
pub struct VolUsnState {
    pub frn_paths: HashMap<u64, Box<str>>,
    pub journal_id: u64,
    pub next_usn: u64,
}

/// 卷根目录固定文件记录号（NTFS $Root）
const ROOT_FRN: u64 = 5;

/// 把 FRN 树解析成完整路径平表（输入记录顺序无关，循环回溯到卷根）
fn build_paths(records: &[MftRecord], root_prefix: &str) -> HashMap<u64, Box<str>> {
    let by_frn: HashMap<u64, &MftRecord> =
        records.iter().map(|r| (r.frn, r)).collect();

    let mut paths: HashMap<u64, Box<str>> = HashMap::with_capacity(records.len());
    for rec in records {
        // 逐级向上拼路径（已解析的父级直接复用，避免重复回溯）
        let mut chain: Vec<&MftRecord> = Vec::new();
        let mut cur = rec;
        loop {
            // 已解析过的祖先直接复用；卷根（FRN=5）为终点，其自身不参与路径拼接
            if cur.frn == ROOT_FRN || paths.contains_key(&cur.frn) {
                break;
            }
            // 环路检测：真实 MFT 中根记录父指针指向自身，异常数据也可能成环，
            // 无此保护会无限回溯并把内存吃光
            if chain.iter().any(|n| n.frn == cur.frn) {
                break;
            }
            chain.push(cur);
            match by_frn.get(&cur.parent_frn) {
                Some(parent) => cur = parent,
                None => break, // 父记录不在本卷（理论不可达）
            }
        }
        // 从最顶层向下写路径
        let mut base = if chain.is_empty() {
            paths.get(&rec.frn).map(|p| p.to_string())
        } else {
            match paths.get(&chain.last().unwrap().parent_frn) {
                Some(p) => Some(p.to_string()),
                None => Some(root_prefix.to_string()), // 顶层直接挂在卷根下
            }
        };
        while let Some(node) = chain.pop() {
            let full = match base.take() {
                Some(dir) if dir.ends_with('\\') => format!("{}{}", dir, node.name),
                Some(dir) => format!("{}\\{}", dir, node.name),
                None => format!("{}{}", root_prefix, node.name),
            };
            paths.insert(node.frn, full.into_boxed_str());
            base = paths.get(&node.frn).map(|p| p.to_string());
        }
    }
    paths
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

    let mut records: Vec<MftRecord> = Vec::new();
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
            records.extend(parse_records(&buf, 8, returned as usize));
            med.start_file_reference_number =
                u64::from_ne_bytes(buf[0..8].try_into().unwrap());
        }
        CloseHandle(handle);
    }

    let root = format!("{}\\", drive.trim_end_matches('\\'));
    let frn_paths = build_paths(&records, &root);
    app_log(&format!(
        "卷 {} MFT 枚举完成：{} 条记录，路径解析 {} 条",
        drive,
        records.len(),
        frn_paths.len()
    ));
    Some(VolUsnState {
        frn_paths,
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

    for rec in records {
        let reason = rec.reason;
        if reason & USN_REASON_RENAME_OLD_NAME != 0 {
            // 旧名：优先用平表里的现成路径，其次拼 parent
            let old_path = state
                .frn_paths
                .get(&rec.frn)
                .map(|p| p.to_string())
                .unwrap_or_default();
            pending_old.insert(rec.frn, old_path);
        }
        if reason & USN_REASON_RENAME_NEW_NAME != 0 {
            let new_path = format!("{}{}", root, rec.name); // 临时，稍后按 parent 修正
            let old_path = pending_old.remove(&rec.frn);
            match old_path {
                Some(old) => {
                    // 改名/移动：更新平表路径
                    let real_new = resolve_path(state, rec, &root, &new_path);
                    if let Some(rp) = real_new {
                        if !old.is_empty() {
                            batch.renamed.push((old, rp.clone(), rec.is_dir));
                        }
                        state.frn_paths.insert(rec.frn, rp.into_boxed_str());
                        renamed_frn.insert(rec.frn);
                    }
                }
                None => {
                    // 无 OLD 的 NEW_NAME：从其他卷移入视为新增；本批已改过名的忽略
                    if !renamed_frn.contains(&rec.frn) {
                        let real_new = resolve_path(state, rec, &root, &new_path);
                        if let Some(rp) = real_new {
                            if seen_created.insert(rec.frn) {
                                batch.created.push((rec.frn, rp, rec.is_dir));
                            }
                        }
                    }
                }
            }
        } else if reason & USN_REASON_FILE_CREATE != 0 {
            if let Some(rp) = resolve_path(state, rec, &root, "") {
                // 新条目也要写入平表，否则它随后的改名/删除无法还原旧路径
                state.frn_paths.insert(rec.frn, rp.as_str().into());
                if seen_created.insert(rec.frn) {
                    batch.created.push((rec.frn, rp, rec.is_dir));
                }
            }
        } else if reason & USN_REASON_FILE_DELETE != 0 {
            // 删除：优先平表路径，其次拼旧名
            let path = state
                .frn_paths
                .get(&rec.frn)
                .map(|p| p.to_string())
                .unwrap_or_else(|| format!("{}{}", root, rec.name));
            batch.deleted.push(path);
            state.frn_paths.remove(&rec.frn);
        }
    }

    // 批尾仍未配对的 OLD_NAME = 纯删除
    for (frn, old_path) in pending_old {
        if !old_path.is_empty() {
            batch.deleted.push(old_path);
        }
        state.frn_paths.remove(&frn);
    }
    batch
}

/// 用平表把 (parent_frn, name) 解析为完整路径；parent 尚未知时返回 None（跳过，下轮全扫兜底）
fn resolve_path(
    state: &VolUsnState,
    rec: &MftRecord,
    root: &str,
    fallback: &str,
) -> Option<String> {
    // 父目录在平表中 → 直接拼
    if let Some(parent_path) = state.frn_paths.get(&rec.parent_frn) {
        let pp = parent_path.to_string();
        return Some(if pp.ends_with('\\') {
            format!("{}{}", pp, rec.name)
        } else {
            format!("{}\\{}", pp, rec.name)
        });
    }
    // 顶层（父是卷根 FRN=5，根不在平表）
    if rec.parent_frn == 5 {
        return Some(format!("{}{}", root, rec.name));
    }
    let _ = fallback;
    None
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

    fn new_state(paths: Vec<(u64, &str)>) -> VolUsnState {
        let mut frn_paths: HashMap<u64, Box<str>> = HashMap::new();
        for (frn, p) in paths {
            frn_paths.insert(frn, p.into());
        }
        VolUsnState {
            frn_paths,
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
        let mut state = new_state(vec![(5, "C:\\"), (42, "C:\\Users\\hanwu\\Documents")]);
        let rec = build_record(99, 42, USN_REASON_FILE_CREATE, "新建.txt", false);
        let recs = parse_records(&rec, 0, rec.len());
        let batch = coalesce("C:", &recs, &mut state);
        assert_eq!(batch.created.len(), 1, "CREATE 应产生一条新增");
        assert_eq!(batch.created[0].1, "C:\\Users\\hanwu\\Documents\\新建.txt");
    }

    #[test]
    fn test_coalesce_delete() {
        let mut state = new_state(vec![(5, "C:\\"), (42, "C:\\Docs"), (99, "C:\\Docs\\旧.txt")]);
        let rec = build_record(99, 42, USN_REASON_FILE_DELETE, "旧.txt", false);
        let recs = parse_records(&rec, 0, rec.len());
        let batch = coalesce("C:", &recs, &mut state);
        assert_eq!(batch.deleted.len(), 1, "DELETE 应产生一条删除");
        assert_eq!(batch.deleted[0], "C:\\Docs\\旧.txt");
        assert!(!state.frn_paths.contains_key(&99), "删除后应移除平表条目");
    }

    #[test]
    fn test_coalesce_rename_pair() {
        let mut state = new_state(vec![(5, "C:\\"), (42, "C:\\Docs"), (99, "C:\\Docs\\旧名.txt")]);
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
        let mut state = new_state(vec![(5, "C:\\"), (42, "C:\\Docs"), (99, "C:\\Docs\\x.txt")]);
        let old = build_record(99, 42, USN_REASON_RENAME_OLD_NAME, "x.txt", false);
        let recs = parse_records(&old, 0, old.len());
        let batch = coalesce("C:", &recs, &mut state);
        assert_eq!(batch.deleted.len(), 1, "未配对的 OLD 应视为删除");
        assert_eq!(batch.deleted[0], "C:\\Docs\\x.txt");
    }

    #[test]
    fn test_coalesce_create_dedup() {
        // 一次写入会产生多条 CREATE 记录（写盘 + 关闭），同一 FRN 只能算一次新增
        let mut state = new_state(vec![(5, "C:\\"), (42, "C:\\Docs")]);
        let mut buf = Vec::new();
        for _ in 0..3 {
            buf.extend(build_record(99, 42, USN_REASON_FILE_CREATE, "同文件.txt", false));
        }
        let recs = parse_records(&buf, 0, buf.len());
        let batch = coalesce("C:", &recs, &mut state);
        assert_eq!(batch.created.len(), 1, "同一 FRN 的重复 CREATE 应去重");
    }

    #[test]
    fn test_build_paths_tree() {
        let records = vec![
            MftRecord { frn: 5, parent_frn: 5, name: ".".into(), is_dir: true, reason: 0 },
            MftRecord { frn: 10, parent_frn: 5, name: "Users".into(), is_dir: true, reason: 0 },
            MftRecord { frn: 11, parent_frn: 10, name: "hanwu".into(), is_dir: true, reason: 0 },
            MftRecord { frn: 12, parent_frn: 11, name: "a.txt".into(), is_dir: false, reason: 0 },
        ];
        let paths = build_paths(&records, "C:\\");
        assert_eq!(paths.get(&12).map(|p| p.as_ref()), Some("C:\\Users\\hanwu\\a.txt"));
    }
}
