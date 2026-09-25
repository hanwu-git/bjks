use crate::models::AppVersionInfo;
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

#[tauri::command]
pub fn get_app_version() -> AppVersionInfo {
    AppVersionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: env!("CARGO_PKG_NAME").to_string(),
    }
}

#[tauri::command]
pub fn get_readme(app_handle: tauri::AppHandle) -> String {
    // 开发环境：直接从项目根目录读取
    let dev_path = PathBuf::from("../src/程序说明.md");
    if dev_path.exists() {
        return fs::read_to_string(dev_path).unwrap_or_else(|_| "无法读取说明文档".to_string());
    }

    // 发布环境：从打包资源目录读取
    if let Ok(resource_dir) = app_handle.path().resource_dir() {
        let resource_path = resource_dir.join("程序说明.md");
        if resource_path.exists() {
            return fs::read_to_string(resource_path).unwrap_or_else(|_| "无法读取说明文档".to_string());
        }
    }

    "说明文档未找到".to_string()
}
