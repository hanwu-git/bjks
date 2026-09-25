use glob::Pattern;
use crate::models::{AppConfig, FileEntry};

pub struct ExcludeFilter {
    path_prefixes: Vec<String>,
    file_patterns: Vec<Pattern>,
    extensions: Vec<String>,
}

impl ExcludeFilter {
    pub fn from_config(config: &AppConfig) -> Self {
        let file_patterns = config.excluded_file_patterns.iter()
            .filter_map(|p| Pattern::new(p).ok())
            .collect();
        
        Self {
            path_prefixes: config.excluded_paths.clone(),
            file_patterns,
            extensions: config.excluded_extensions.clone(),
        }
    }
    
    pub fn is_excluded(&self, entry: &FileEntry) -> bool {
        // 1. 路径前缀匹配
        if self.path_prefixes.iter().any(|p| {
            entry.parent_path.starts_with(p)
        }) { return true; }
        
        // 2. 文件名 glob 匹配
        if self.file_patterns.iter().any(|p| p.matches(&entry.name)) {
            return true;
        }
        
        // 3. 扩展名匹配
        if self.extensions.iter().any(|e| entry.extension == *e) {
            return true;
        }
        
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_entry(name: &str, path: &str, ext: &str) -> FileEntry {
        FileEntry {
            id: 1,
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            path: path.to_string(),
            parent_path: std::path::Path::new(path).parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            extension: ext.to_string(),
            size: 1024,
            modified: 1700000000,
            is_dir: false,
        }
    }

    #[test]
    fn test_exclude_by_path() {
        let config = AppConfig {
            excluded_paths: vec!["C:\\Windows".to_string()],
            excluded_file_patterns: vec![],
            excluded_extensions: vec![],
            scan_drives: vec![],
            max_results: 1000,
        };
        
        let filter = ExcludeFilter::from_config(&config);
        
        // 应该被排除
        let entry1 = create_test_entry("test.txt", "C:\\Windows\\System32\\test.txt", "txt");
        assert!(filter.is_excluded(&entry1));
        
        // 不应该被排除
        let entry2 = create_test_entry("test.txt", "D:\\Documents\\test.txt", "txt");
        assert!(!filter.is_excluded(&entry2));
    }

    #[test]
    fn test_exclude_by_file_pattern() {
        let config = AppConfig {
            excluded_paths: vec![],
            excluded_file_patterns: vec!["*.tmp".to_string(), "Thumbs.db".to_string()],
            excluded_extensions: vec![],
            scan_drives: vec![],
            max_results: 1000,
        };
        
        let filter = ExcludeFilter::from_config(&config);
        
        // 应该被排除
        let entry1 = create_test_entry("temp.tmp", "C:\\test\\temp.tmp", "tmp");
        assert!(filter.is_excluded(&entry1));
        
        let entry2 = create_test_entry("Thumbs.db", "D:\\photos\\Thumbs.db", "db");
        assert!(filter.is_excluded(&entry2));
        
        // 不应该被排除
        let entry3 = create_test_entry("document.txt", "C:\\test\\document.txt", "txt");
        assert!(!filter.is_excluded(&entry3));
    }

    #[test]
    fn test_exclude_by_extension() {
        let config = AppConfig {
            excluded_paths: vec![],
            excluded_file_patterns: vec![],
            excluded_extensions: vec!["log".to_string(), "bak".to_string()],
            scan_drives: vec![],
            max_results: 1000,
        };
        
        let filter = ExcludeFilter::from_config(&config);
        
        // 应该被排除
        let entry1 = create_test_entry("app.log", "C:\\test\\app.log", "log");
        assert!(filter.is_excluded(&entry1));
        
        let entry2 = create_test_entry("backup.bak", "D:\\backup.bak", "bak");
        assert!(filter.is_excluded(&entry2));
        
        // 不应该被排除
        let entry3 = create_test_entry("document.txt", "C:\\test\\document.txt", "txt");
        assert!(!filter.is_excluded(&entry3));
    }

    #[test]
    fn test_multiple_rules_combined() {
        let config = AppConfig {
            excluded_paths: vec!["C:\\Windows".to_string()],
            excluded_file_patterns: vec!["*.tmp".to_string()],
            excluded_extensions: vec!["log".to_string()],
            scan_drives: vec![],
            max_results: 1000,
        };
        
        let filter = ExcludeFilter::from_config(&config);
        
        // 路径匹配 - 应该被排除
        let entry1 = create_test_entry("test.txt", "C:\\Windows\\test.txt", "txt");
        assert!(filter.is_excluded(&entry1));
        
        // 文件名模式匹配 - 应该被排除
        let entry2 = create_test_entry("temp.tmp", "D:\\test\\temp.tmp", "tmp");
        assert!(filter.is_excluded(&entry2));
        
        // 扩展名匹配 - 应该被排除
        let entry3 = create_test_entry("app.log", "E:\\logs\\app.log", "log");
        assert!(filter.is_excluded(&entry3));
        
        // 都不匹配 - 不应该被排除
        let entry4 = create_test_entry("document.txt", "F:\\docs\\document.txt", "txt");
        assert!(!filter.is_excluded(&entry4));
    }

    #[test]
    fn test_empty_config() {
        let config = AppConfig::default();
        let filter = ExcludeFilter::from_config(&config);
        
        // 默认配置应该排除 C:\Windows
        let entry1 = create_test_entry("test.txt", "C:\\Windows\\test.txt", "txt");
        assert!(filter.is_excluded(&entry1));
        
        // 默认配置应该排除 Thumbs.db
        let entry2 = create_test_entry("Thumbs.db", "D:\\photos\\Thumbs.db", "db");
        assert!(filter.is_excluded(&entry2));
        
        // 其他文件不应该被排除
        let entry3 = create_test_entry("document.txt", "E:\\docs\\document.txt", "txt");
        assert!(!filter.is_excluded(&entry3));
    }
}
