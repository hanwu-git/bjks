use crate::models::AppVersionInfo;

/// 程序说明文档：编译期嵌入二进制，确保绿色版仅需单个 exe 即可查看说明
const README: &str = include_str!("../../../程序说明.md");

#[tauri::command]
pub fn get_app_version() -> AppVersionInfo {
    AppVersionInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        name: env!("CARGO_PKG_NAME").to_string(),
    }
}

#[tauri::command]
pub fn get_readme() -> String {
    README.to_string()
}