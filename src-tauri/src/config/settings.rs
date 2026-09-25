use crate::models::AppConfig;
use std::fs;
use std::path::PathBuf;

const CONFIG_FILE: &str = "config.json";

/// 获取配置文件路径
fn get_config_path() -> PathBuf {
    let app_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("本机快搜");
    fs::create_dir_all(&app_dir).ok();
    app_dir.join(CONFIG_FILE)
}

/// 加载配置
pub fn load_config() -> AppConfig {
    let path = get_config_path();
    if path.exists() {
        let content = fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        AppConfig::default()
    }
}

/// 保存配置
pub fn save_config(config: &AppConfig) -> Result<(), String> {
    let path = get_config_path();
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| e.to_string())?;
    fs::write(&path, content)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_save_and_load_config() {
        // 创建测试配置
        let config = AppConfig {
            excluded_paths: vec!["C:\\Test".to_string()],
            excluded_file_patterns: vec!["*.test".to_string()],
            excluded_extensions: vec!["test".to_string()],
            scan_drives: vec!["C:".to_string()],
            max_results: 500,
        };

        // 保存配置
        save_config(&config).expect("保存配置失败");

        // 加载配置
        let loaded = load_config();

        // 验证配置一致
        assert_eq!(loaded.excluded_paths, config.excluded_paths);
        assert_eq!(loaded.excluded_file_patterns, config.excluded_file_patterns);
        assert_eq!(loaded.excluded_extensions, config.excluded_extensions);
        assert_eq!(loaded.scan_drives, config.scan_drives);
        assert_eq!(loaded.max_results, config.max_results);

        // 清理测试文件
        let path = get_config_path();
        if path.exists() {
            fs::remove_file(path).ok();
        }
    }

    #[test]
    fn test_load_default_config() {
        // 删除配置文件（如果存在）
        let path = get_config_path();
        if path.exists() {
            fs::remove_file(&path).ok();
        }

        // 加载配置应该返回默认值
        let config = load_config();
        let default = AppConfig::default();

        assert_eq!(config.excluded_paths, default.excluded_paths);
        assert_eq!(config.excluded_file_patterns, default.excluded_file_patterns);
        assert_eq!(config.excluded_extensions, default.excluded_extensions);
        assert_eq!(config.scan_drives, default.scan_drives);
        assert_eq!(config.max_results, default.max_results);
    }

    #[test]
    fn test_config_file_is_json() {
        // 保存配置
        let config = AppConfig::default();
        save_config(&config).expect("保存配置失败");

        // 读取文件内容
        let path = get_config_path();
        let content = fs::read_to_string(&path).expect("读取配置文件失败");

        // 验证是有效的 JSON
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&content);
        assert!(parsed.is_ok(), "配置文件不是有效的 JSON 格式");

        // 验证格式可读（pretty print）
        assert!(content.contains('\n'), "配置文件应该使用 pretty print 格式");

        // 清理
        fs::remove_file(path).ok();
    }
}
