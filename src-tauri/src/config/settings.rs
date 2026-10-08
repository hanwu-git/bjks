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
    use std::sync::Mutex;

    /// 三个测试共用同一个配置文件，通过锁串行化避免相互覆盖
    static CONFIG_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_save_and_load_config() {
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
        // 创建测试配置
        let config = AppConfig {
            excluded_paths: vec!["C:\\Test".to_string()],
            excluded_file_patterns: vec!["*.test".to_string()],
            excluded_extensions: vec!["test".to_string()],
            scan_drives: vec!["C:".to_string()],
            max_results: 500,
            ..AppConfig::default()
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
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
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
    fn test_old_config_missing_new_fields() {
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
        let path = get_config_path();

        // 模拟旧版（1.0.2）配置文件：缺少 window_width/window_height/close_action
        let legacy = serde_json::json!({
            "excluded_paths": ["C:\\Windows"],
            "excluded_file_patterns": ["*.tmp"],
            "excluded_extensions": [],
            "scan_drives": ["C:"],
            "max_results": 1000
        });
        fs::write(&path, legacy.to_string()).expect("写入旧配置失败");

        let config = load_config();
        assert_eq!(config.window_width, 0, "旧配置窗口宽度应为 0（未记录）");
        assert_eq!(config.window_height, 0, "旧配置窗口高度应为 0（未记录）");
        assert_eq!(config.close_action, "minimize", "旧配置关闭行为应回退为最小化到托盘");
        assert_eq!(config.max_results, 1000);

        fs::remove_file(path).ok();
    }

    #[test]
    fn test_config_file_is_json() {
        let _guard = CONFIG_TEST_LOCK.lock().unwrap();
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
