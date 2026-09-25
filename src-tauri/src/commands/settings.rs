use crate::commands::search::AppState;
use crate::config::settings::{load_config, save_config};
use crate::indexer::enumerate_volumes;
use crate::indexer::startup::force_rescan;
use crate::logger::app_log;
use crate::models::AppConfig;
use crate::search::engine::SearchEngine;
use std::process::Command;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub fn get_config() -> AppConfig {
    load_config()
}

#[tauri::command]
pub fn save_settings(config: AppConfig) -> bool {
    save_config(&config).is_ok()
}

/// 按类别增删屏蔽项，kind 取 path / pattern / extension
#[tauri::command]
pub fn add_exclude_item(kind: String, value: String) -> bool {
    let value = value.trim().to_string();
    if value.is_empty() {
        return false;
    }
    let mut config = load_config();
    let list = match exclude_list_mut(&mut config, &kind) {
        Some(list) => list,
        None => return false,
    };
    if !list.contains(&value) {
        list.push(value);
    }
    save_config(&config).is_ok()
}

#[tauri::command]
pub fn remove_exclude_item(kind: String, value: String) -> bool {
    let mut config = load_config();
    let list = match exclude_list_mut(&mut config, &kind) {
        Some(list) => list,
        None => return false,
    };
    list.retain(|item| item != &value);
    save_config(&config).is_ok()
}

fn exclude_list_mut<'a>(config: &'a mut AppConfig, kind: &str) -> Option<&'a mut Vec<String>> {
    match kind {
        "path" => Some(&mut config.excluded_paths),
        "pattern" => Some(&mut config.excluded_file_patterns),
        "extension" => Some(&mut config.excluded_extensions),
        _ => None,
    }
}

/// 重新扫描：删除缓存并全量重建索引（在阻塞线程池中执行，不冻结界面）
#[tauri::command]
pub async fn rescan(state: State<'_, AppState>) -> Result<bool, String> {
    let config = load_config();
    let volumes = enumerate_volumes(&config.scan_drives);
    if volumes.is_empty() {
        app_log("重新扫描中止：没有可用的盘符");
        return Ok(false);
    }
    app_log(&format!(
        "开始重新扫描，盘符: {:?}",
        volumes.iter().map(|v| v.drive_letter.clone()).collect::<Vec<_>>()
    ));

    let engine = Arc::clone(&state.engine);
    let scanned_files = Arc::clone(&state.scanned_files);
    let is_scanning = Arc::clone(&state.is_scanning);
    *is_scanning.lock().unwrap() = true;

    let entries = tauri::async_runtime::spawn_blocking(move || force_rescan(&volumes, &config))
        .await
        .map_err(|e| e.to_string())?;

    let count = entries.len();
    app_log(&format!("重新扫描完成：{} 个文件", count));
    *engine.lock().unwrap() = SearchEngine::new(entries);
    *scanned_files.lock().unwrap() = count;
    *is_scanning.lock().unwrap() = false;

    Ok(true)
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