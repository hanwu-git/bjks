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

/// 配置为空时默认参与索引的盘符
const DEFAULT_SCAN_DRIVES: [&str; 2] = ["C:", "D:"];

/// 判断盘符根目录是否可访问
fn is_ready(root: &str) -> bool {
    Path::new(root).is_dir() && fs::read_dir(root).is_ok()
}

/// 列出当前可访问的盘符（形如 "C:"），供设置界面选择
pub fn list_available_drives() -> Vec<String> {
    (b'A'..=b'Z')
        .map(|letter| format!("{}:", letter as char))
        .filter(|drive| is_ready(&format!("{}\\", drive)))
        .collect()
}

/// 枚举可访问的卷；scan_drives 为空时回退为默认盘符（C:、D:）
pub fn enumerate_volumes(scan_drives: &[String]) -> Vec<VolumeInfo> {
    let targets: Vec<String> = if scan_drives.is_empty() {
        DEFAULT_SCAN_DRIVES.iter().map(|d| d.to_string()).collect()
    } else {
        scan_drives
            .iter()
            .map(|drive| normalize_drive(drive))
            .filter(|drive| drive.len() > 1)
            .collect()
    };

    targets
        .into_iter()
        .filter_map(|drive| {
            let root = format!("{}\\", drive);
            if is_ready(&root) {
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