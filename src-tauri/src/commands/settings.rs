use crate::models::AppConfig;
use crate::config::settings::{load_config, save_config};
use std::process::Command;

#[tauri::command]
pub fn get_config() -> AppConfig {
    load_config()
}

#[tauri::command]
pub fn save_settings(config: AppConfig) -> bool {
    save_config(&config).is_ok()
}

#[tauri::command]
pub fn add_exclude_path(path: String) -> bool {
    let mut config = load_config();
    if !config.excluded_paths.contains(&path) {
        config.excluded_paths.push(path);
        return save_config(&config).is_ok();
    }
    true
}

#[tauri::command]
pub fn remove_exclude_path(path: String) -> bool {
    let mut config = load_config();
    config.excluded_paths.retain(|p| p != &path);
    save_config(&config).is_ok()
}

#[tauri::command]
pub fn rescan() -> bool {
    // TODO: Implement rescan logic
    true
}

#[tauri::command]
pub fn open_file(path: String) -> bool {
    Command::new("cmd")
        .args(&["/C", "start", "", &path])
        .spawn()
        .is_ok()
}

#[tauri::command]
pub fn open_folder(path: String) -> bool {
    let folder = if std::path::Path::new(&path).is_file() {
        std::path::Path::new(&path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(path)
    } else {
        path
    };
    
    Command::new("explorer")
        .arg(&folder)
        .spawn()
        .is_ok()
}
