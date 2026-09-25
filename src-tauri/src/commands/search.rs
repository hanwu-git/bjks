use crate::config::settings::load_config;
use crate::logger::app_log;
use crate::models::{IndexStatus, SearchRequest, SearchResponse};
use crate::search::engine::SearchEngine;
use std::sync::{Arc, Mutex};
use tauri::State;

pub struct AppState {
    pub engine: Arc<Mutex<SearchEngine>>,
    pub is_scanning: Arc<Mutex<bool>>,
    pub scanned_files: Arc<Mutex<usize>>,
}

#[tauri::command]
pub fn search(mut request: SearchRequest, state: State<AppState>) -> SearchResponse {
    // 结果数上限由配置控制
    let max_results = load_config().max_results;
    if max_results > 0 && request.limit > max_results {
        request.limit = max_results;
    }

    let engine = state.engine.lock().unwrap();
    let response = engine.search(&request);
    app_log(&format!(
        "搜索完成: query={}, total={}, query_time_ms={}",
        request.query, response.total, response.query_time_ms
    ));
    response
}

#[tauri::command]
pub fn get_index_status(state: State<AppState>) -> IndexStatus {
    let is_scanning = *state.is_scanning.lock().unwrap();
    let scanned_files = *state.scanned_files.lock().unwrap();
    let index_count = state.engine.lock().unwrap().get_index_count();

    IndexStatus {
        is_scanning,
        scanned_files,
        total_estimated: 0,
        progress_percent: if index_count > 0 { 100.0 } else { 0.0 },
    }
}