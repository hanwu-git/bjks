use notify::{Watcher, RecursiveMode, Event, EventKind};
use std::sync::{Arc, Mutex};
use std::path::Path;
use std::time::UNIX_EPOCH;
use std::fs;
use crate::models::FileEntry;

pub struct FileWatcher {
    _watcher: notify::RecommendedWatcher,
}

impl FileWatcher {
    pub fn start(
        paths: &[String],
        index: Arc<Mutex<Vec<FileEntry>>>,
    ) -> Result<Self, String> {
        let index_clone = Arc::clone(&index);
        
        let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
            if let Ok(event) = res {
                match event.kind {
                    EventKind::Create(_) => {
                        for path in event.paths {
                            if let Some(entry) = Self::path_to_entry(&path) {
                                if let Ok(mut idx) = index_clone.lock() {
                                    // 检查是否已存在
                                    if !idx.iter().any(|e| e.path == entry.path) {
                                        let new_id = idx.iter().map(|e| e.id).max().unwrap_or(0) + 1;
                                        let mut new_entry = entry;
                                        new_entry.id = new_id;
                                        idx.push(new_entry);
                                    }
                                }
                            }
                        }
                    }
                    EventKind::Remove(_) => {
                        for path in event.paths {
                            // 提取文件名用于匹配
                            let file_name = path.file_name()
                                .map(|n| n.to_string_lossy().to_string());
                            
                            if let Some(name) = file_name {
                                if let Ok(mut idx) = index_clone.lock() {
                                    idx.retain(|e| e.name != name);
                                }
                            }
                        }
                    }
                    EventKind::Modify(_) => {
                        for path in event.paths {
                            if let Some(new_entry) = Self::path_to_entry(&path) {
                                if let Ok(mut idx) = index_clone.lock() {
                                    if let Some(existing) = idx.iter_mut().find(|e| e.path == new_entry.path) {
                                        existing.size = new_entry.size;
                                        existing.modified = new_entry.modified;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }).map_err(|e| e.to_string())?;
        
        // 监控各卷根目录
        for path in paths {
            watcher.watch(Path::new(path), RecursiveMode::Recursive)
                .map_err(|e| e.to_string())?;
        }
        
        Ok(Self { _watcher: watcher })
    }
    
    fn path_to_entry(path: &Path) -> Option<FileEntry> {
        let metadata = path.metadata().ok()?;
        let name = path.file_name()?.to_string_lossy().to_string();
        let name_lower = name.to_lowercase();
        let path_str = path.to_string_lossy().to_string();
        let parent_path = path.parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let extension = path.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let size = metadata.len();
        let modified = metadata.modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let is_dir = metadata.is_dir();
        
        Some(FileEntry {
            id: 0, // 后续分配
            name,
            name_lower,
            path: path_str,
            parent_path,
            extension,
            size,
            modified,
            is_dir,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_watcher_creation() {
        let index = Arc::new(Mutex::new(Vec::new()));
        let test_dir = "test_watch_dir";
        
        // 创建测试目录
        fs::create_dir_all(test_dir).ok();
        
        // 启动监控
        let result = FileWatcher::start(&[test_dir.to_string()], index);
        assert!(result.is_ok(), "应该能成功创建监控器");
        
        // 清理
        fs::remove_dir_all(test_dir).ok();
    }

    #[test]
    fn test_file_creation_detection() {
        let index = Arc::new(Mutex::new(Vec::new()));
        let test_dir = "test_watch_create";
        
        fs::create_dir_all(test_dir).ok();
        
        let _watcher = FileWatcher::start(&[test_dir.to_string()], Arc::clone(&index))
            .expect("启动监控失败");
        
        // 创建新文件
        let test_file = format!("{}\\test.txt", test_dir);
        fs::write(&test_file, "test content").expect("创建文件失败");
        
        // 等待事件处理
        thread::sleep(Duration::from_millis(500));
        
        // 检查索引
        let idx = index.lock().unwrap();
        assert!(idx.iter().any(|e| e.path.contains("test.txt")), 
                "新创建的文件应该被添加到索引");
        
        // 清理
        drop(_watcher);
        fs::remove_dir_all(test_dir).ok();
    }

    #[test]
    fn test_file_deletion_detection() {
        let index = Arc::new(Mutex::new(Vec::new()));
        let test_dir = "test_watch_delete";
        
        fs::create_dir_all(test_dir).ok();
        
        // 先添加一个文件到索引
        let test_file = format!("{}\\existing.txt", test_dir);
        fs::write(&test_file, "existing content").expect("创建文件失败");
        
        // 获取文件的绝对路径用于比较
        let abs_path = fs::canonicalize(&test_file).unwrap_or_else(|_| test_file.clone().into());
        let path_str = abs_path.to_string_lossy().to_string();
        
        let entry = FileEntry {
            id: 1,
            name: "existing.txt".to_string(),
            name_lower: "existing.txt".to_string(),
            path: path_str.clone(),
            parent_path: test_dir.to_string(),
            extension: "txt".to_string(),
            size: 16,
            modified: 0,
            is_dir: false,
        };
        
        index.lock().unwrap().push(entry);
        
        let _watcher = FileWatcher::start(&[test_dir.to_string()], Arc::clone(&index))
            .expect("启动监控失败");
        
        // 删除文件
        fs::remove_file(&test_file).expect("删除文件失败");
        
        // 等待事件处理（增加等待时间）
        thread::sleep(Duration::from_millis(1000));
        
        // 检查索引 - 使用文件名而不是完整路径比较
        let idx = index.lock().unwrap();
        let has_file = idx.iter().any(|e| e.name == "existing.txt");
        assert!(!has_file, "删除的文件应该从索引中移除");
        
        // 清理
        drop(_watcher);
        fs::remove_dir_all(test_dir).ok();
    }
}
