pub mod mft_scanner;
pub mod cache;
pub mod startup;

use serde::{Serialize, Deserialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeInfo {
    pub drive_letter: String,
}

/// 枚举可访问的卷；scan_drives 为空表示扫描所有就绪盘符
pub fn enumerate_volumes(scan_drives: &[String]) -> Vec<VolumeInfo> {
    let targets: Vec<String> = if scan_drives.is_empty() {
        (b'A'..=b'Z').map(|letter| format!("{}:", letter as char)).collect()
    } else {
        scan_drives.iter().map(|drive| normalize_drive(drive)).collect()
    };

    targets
        .into_iter()
        .filter_map(|drive| {
            let root = format!("{}\\", drive);
            if Path::new(&root).is_dir() && fs::read_dir(&root).is_ok() {
                Some(VolumeInfo { drive_letter: root })
            } else {
                None
            }
        })
        .collect()
}

/// 规范化盘符输入：接受 "C"、"C:"、"C:\" 等形式，统一为 "C:"
fn normalize_drive(input: &str) -> String {
    let letter = input
        .trim()
        .trim_end_matches(|c| c == '\\' || c == '/')
        .trim_end_matches(':');
    format!("{}:", letter.to_ascii_uppercase())
}