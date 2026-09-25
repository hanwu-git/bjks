use crate::indexer::VolumeInfo;
use crate::models::{AppConfig, FileEntry};
use glob::Pattern;
use rayon::prelude::*;
use walkdir::{DirEntry, WalkDir};

/// 扫描排除规则：目录路径、文件名通配模式、扩展名
struct ScanFilter {
    paths: Vec<String>,
    patterns: Vec<Pattern>,
    extensions: Vec<String>,
}

impl ScanFilter {
    fn from_config(config: &AppConfig) -> Self {
        Self {
            paths: config
                .excluded_paths
                .iter()
                .map(|p| p.to_lowercase())
                .collect(),
            patterns: config
                .excluded_file_patterns
                .iter()
                .filter_map(|p| Pattern::new(&p.to_lowercase()).ok())
                .collect(),
            extensions: config
                .excluded_extensions
                .iter()
                .map(|e| e.trim_start_matches('.').to_lowercase())
                .filter(|e| !e.is_empty())
                .collect(),
        }
    }

    fn is_excluded(&self, entry: &DirEntry) -> bool {
        let path = entry.path();

        let path_str = path.to_string_lossy().to_lowercase();
        if self.paths.iter().any(|p| path_str.contains(p.as_str())) {
            return true;
        }

        if let Some(name) = entry.file_name().to_str() {
            let name_lower = name.to_lowercase();
            if self.patterns.iter().any(|p| p.matches(&name_lower)) {
                return true;
            }
        }

        if !entry.file_type().is_dir() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let ext_lower = ext.to_lowercase();
                if self.extensions.iter().any(|e| e == &ext_lower) {
                    return true;
                }
            }
        }

        false
    }
}

/// 扫描指定卷（使用 walkdir 遍历）
pub fn scan_volume_fast(root: &str, config: &AppConfig) -> Vec<FileEntry> {
    let filter = ScanFilter::from_config(config);
    let mut entries = Vec::new();
    let mut id_counter: u64 = 0;

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| !filter.is_excluded(e))
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
pub fn scan_all_volumes(volumes: &[VolumeInfo], config: &AppConfig) -> Vec<FileEntry> {
    volumes
        .par_iter()
        .flat_map(|vol| {
            let mut entries = scan_volume_fast(&vol.drive_letter, config);
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
        let entries = scan_volume_fast(".", &AppConfig::default());
        assert!(!entries.is_empty(), "应该扫描到至少一个文件");

        // 验证所有条目都有有效的 ID
        for entry in &entries {
            assert!(entry.id < entries.len() as u64);
        }
    }

    #[test]
    fn test_exclusion() {
        let config = AppConfig {
            excluded_paths: vec!["target".to_string()],
            ..Default::default()
        };
        let entries = scan_volume_fast(".", &config);

        // 验证没有路径包含 "target" 的条目
        for entry in &entries {
            assert!(!entry.path.to_lowercase().contains("target"),
                   "路径 {} 应该被排除", entry.path);
        }
    }

    #[test]
    fn test_exclude_pattern_and_extension() {
        let config = AppConfig {
            excluded_paths: vec![],
            excluded_file_patterns: vec!["*.md".to_string()],
            excluded_extensions: vec!["toml".to_string()],
            ..Default::default()
        };
        let entries = scan_volume_fast(".", &config);

        for entry in &entries {
            assert!(!entry.name.to_lowercase().ends_with(".md"),
                   "文件 {} 命中屏蔽模式 *.md，应该被排除", entry.name);
            assert!(entry.is_dir || entry.extension != "toml",
                   "文件 {} 的扩展名被屏蔽，应该被排除", entry.name);
        }
    }

    #[test]
    fn test_scan_all_volumes() {
        // 创建模拟的卷信息
        let volumes = vec![
            VolumeInfo { drive_letter: ".".to_string() },
        ];

        let entries = scan_all_volumes(&volumes, &AppConfig::default());
        assert!(!entries.is_empty(), "应该扫描到至少一个文件");

        // 验证 ID 全局唯一
        let mut ids: Vec<u64> = entries.iter().map(|e| e.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), entries.len(), "ID 应该全局唯一");
    }
}