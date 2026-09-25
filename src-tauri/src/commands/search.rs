use crate::models::{SearchRequest, SearchResponse, IndexStatus};
use crate::search::engine::SearchEngine;
use std::sync::{Arc, Mutex};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use tauri::State;

pub struct AppState {
    pub engine: Arc<Mutex<SearchEngine>>,
    pub is_scanning: Arc<Mutex<bool>>,
    pub scanned_files: Arc<Mutex<usize>>,
}

fn app_log(msg: &str) {
    let app_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("本机快搜");
    fs::create_dir_all(&app_dir).ok();
    let path = app_dir.join("app.log");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap_or_else(|_| panic!("无法打开日志文件: {:?}", path));
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    writeln!(file, "[{}] {}", timestamp, msg).ok();
}

#[tauri::command]
pub fn search(request: SearchRequest, state: State<AppState>) -> SearchResponse {
    app_log(&format!("收到搜索请求: query={}, sort_by={:?}, asc={}", request.query, request.sort_by, request.sort_asc));
    let engine = state.engine.lock().unwrap();
    let response = engine.search(&request);
    app_log(&format!("搜索完成: total={}, query_time_ms={}", response.total, response.query_time_ms));
    response
}

#[tauri::command]
pub fn get_index_status(state: State<AppState>) -> IndexStatus {
    let is_scanning = *state.is_scanning.lock().unwrap();
    let scanned_files = *state.scanned_files.lock().unwrap();
    let engine = state.engine.lock().unwrap();
    let index_count = engine.get_index_count();
    
    // 直接写入单独的诊断文件
    let diag_path = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("本机快搜")
        .join("cmd_diag.log");
    fs::create_dir_all(diag_path.parent().unwrap()).ok();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&diag_path)
        .unwrap();
    writeln!(file, "[{}] get_index_status: is_scanning={}, scanned_files={}, index_count={}", 
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        is_scanning, scanned_files, index_count).ok();
    
    app_log(&format!("get_index_status: is_scanning={}, scanned_files={}, index_count={}", is_scanning, scanned_files, index_count));
    
    IndexStatus {
        is_scanning,
        scanned_files,
        total_estimated: 0,
        progress_percent: if index_count > 0 { 100.0 } else { 0.0 },
    }
}
