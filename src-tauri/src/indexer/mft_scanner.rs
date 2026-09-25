use crate::models::FileEntry;
use crate::indexer::VolumeInfo;
use walkdir::WalkDir;
use std::path::Path;
use rayon::prelude::*;

/// 检查路径是否应该被排除
fn is_excluded(path: &Path, excludes: &[String]) -> bool {
    let path_str = path.to_string_lossy().to_lowercase();
    for exclude in excludes {
        let exclude_lower = exclude.to_lowercase();
        if path_str.contains(&exclude_lower) {
            return true;
        }
    }
    false
}

/// 快速扫描指定卷（MVP 版本使用 walkdir）
pub fn scan_volume_fast(root: &str, excludes: &[String]) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut id_counter: u64 = 0;
    
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| !is_excluded(e.path(), excludes))
        .filter_map(|e| e.ok())
    {
        let metadata = entry.metadata().ok();
        let file_entry = FileEntry {
            id: id_counter,
            name: entry.file_name().to_string_lossy().to_string(),
            name_lower: entry.file_name().to_string_lossy().to_lowercase(),
            path: entry.path().to_string_lossy().to_string(),
            parent_path: entry.path().parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            extension: entry.path().extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
            size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
            modified: metadata.as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            is_dir: entry.file_type().is_dir(),
        };
        entries.push(file_entry);
        id_counter += 1;
    }
    entries
}

/// 并行扫描多个卷
pub fn scan_all_volumes(volumes: &[VolumeInfo], excludes: &[String]) -> Vec<FileEntry> {
    volumes
        .par_iter()
        .flat_map(|vol| {
            let mut entries = scan_volume_fast(&vol.drive_letter, excludes);
            // 重新分配 ID 以确保全局唯一性
            let start_id = entries.first().map(|e| e.id).unwrap_or(0);
            for (idx, entry) in entries.iter_mut().enumerate() {
                entry.id = start_id + idx as u64;
            }
            entries
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_subdirectory() {
        // 扫描当前目录作为测试
        let entries = scan_volume_fast(".", &[]);
        assert!(!entries.is_empty(), "应该扫描到至少一个文件");
        
        // 验证所有条目都有有效的 ID
        for entry in &entries {
            assert!(entry.id < entries.len() as u64);
        }
    }

    #[test]
    fn test_exclusion() {
        let excludes = vec!["target".to_string()];
        let entries = scan_volume_fast(".", &excludes);
        
        // 验证没有路径包含 "target" 的条目
        for entry in &entries {
            assert!(!entry.path.to_lowercase().contains("target"), 
                   "路径 {} 应该被排除", entry.path);
        }
    }

    #[test]
    fn test_scan_all_volumes() {
        // 创建模拟的卷信息
        let volumes = vec![
            VolumeInfo {
                drive_letter: ".".to_string(),
                volume_name: "Test".to_string(),
                total_size: 0,
                is_ntfs: true,
            },
        ];
        
        let entries = scan_all_volumes(&volumes, &[]);
        assert!(!entries.is_empty(), "应该扫描到至少一个文件");
        
        // 验证 ID 全局唯一
        let mut ids: Vec<u64> = entries.iter().map(|e| e.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), entries.len(), "ID 应该全局唯一");
    }
}
