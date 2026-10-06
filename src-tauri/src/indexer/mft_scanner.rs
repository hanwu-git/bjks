use crate::models::{AppConfig, FileEntry};
use glob::Pattern;
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

/// 进度回调的文件数间隔
const PROGRESS_INTERVAL: usize = 500;

/// 扫描指定卷，并通过回调持续上报已扫描的文件数
pub fn scan_volume_fast_with_progress(
    root: &str,
    config: &AppConfig,
    on_progress: &dyn Fn(usize),
) -> Vec<FileEntry> {
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

        if entries.len() % PROGRESS_INTERVAL == 0 {
            on_progress(entries.len());
        }
    }

    on_progress(entries.len());
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// 测试用：不带进度回调的扫描
    fn scan_volume_fast(root: &str, config: &AppConfig) -> Vec<FileEntry> {
        scan_volume_fast_with_progress(root, config, &|_| {})
    }

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
    fn test_scan_with_progress_callback() {
        // 进度回调应至少被调用一次，且最终数量与结果一致
        let total = Cell::new(0usize);
        let entries = scan_volume_fast_with_progress(".", &AppConfig::default(), &|count| {
            total.set(count);
        });

        assert!(!entries.is_empty(), "应该扫描到至少一个文件");
        assert_eq!(total.get(), entries.len(), "最终进度应等于扫描条目总数");
    }
}