use crate::config::settings::load_config;
use crate::logger::app_log;
use crate::models::{IndexStatus, ScanProgress, SearchRequest, SearchResponse};
use crate::search::engine::SearchEngine;
use std::sync::{Arc, Mutex};
use tauri::State;

pub struct AppState {
    pub engine: Arc<Mutex<SearchEngine>>,
    pub is_scanning: Arc<Mutex<bool>>,
    pub scanned_files: Arc<Mutex<usize>>,
    pub progress: Arc<Mutex<ScanProgress>>,
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
    let progress = state.progress.lock().unwrap();

    // 扫描中按「已完成盘数 / 总盘数」计算真实百分比
    let progress_percent = if is_scanning {
        if progress.total_volumes > 0 {
            progress.completed_volumes as f32 / progress.total_volumes as f32 * 100.0
        } else {
            0.0
        }
    } else if index_count > 0 {
        100.0
    } else {
        0.0
    };

    IndexStatus {
        is_scanning,
        scanned_files,
        total_estimated: 0,
        progress_percent,
        total_volumes: progress.total_volumes,
        completed_volumes: progress.completed_volumes,
        current_volume: progress.current_volume.clone(),
    }
}