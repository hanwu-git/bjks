use crate::models::{AppConfig, FileEntry};
use walkdir::{DirEntry, WalkDir};

/// 简易通配符匹配：支持 `*`（任意长度）与 `?`（单个字节）。
/// 调用方保证 pattern 与 text 均已转为小写，故此处按字节比较即可。
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut match_idx = 0usize;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            match_idx = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            match_idx += 1;
            ti = match_idx;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

/// 扫描排除规则：目录路径、文件名通配模式、扩展名
struct ScanFilter {
    paths: Vec<String>,
    patterns: Vec<String>,
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
                .map(|p| p.to_lowercase())
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
            if self.patterns.iter().any(|p| wildcard_match(p, &name_lower)) {
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

    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| !filter.is_excluded(e))
        .filter_map(|e| e.ok())
    {
        let metadata = entry.metadata().ok();
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let file_entry = FileEntry {
            name: file_name.into_boxed_str(),
            path: entry.path().to_string_lossy().into_owned().into_boxed_str(),
            extension: entry.path().extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default()
                .into_boxed_str(),
            size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
            modified: metadata.as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            is_dir: entry.file_type().is_dir(),
        };
        entries.push(file_entry);

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

        // 验证所有条目都有有效的文件名
        for entry in &entries {
            assert!(!entry.name.is_empty(), "文件名不应为空");
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
            assert!(entry.is_dir || entry.extension.as_ref() != "toml",
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

    #[test]
    fn test_wildcard_match() {
        assert!(wildcard_match("*.tmp", "a.tmp"));
        assert!(wildcard_match("*.tmp", "abc.tmp"));
        assert!(!wildcard_match("*.tmp", "abc.txt"));
        assert!(wildcard_match("thumbs.db", "thumbs.db"));
        assert!(!wildcard_match("thumbs.db", "thumbs.dbx"));
        assert!(wildcard_match("a?c", "abc"));
        assert!(!wildcard_match("a?c", "ac"));
        assert!(wildcard_match("*", "anything"));
    }
}