use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// 应用数据目录（%LOCALAPPDATA%\本机快搜）
pub fn app_data_dir() -> PathBuf {
    let dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("本机快搜");
    fs::create_dir_all(&dir).ok();
    dir
}

/// 应用日志：追加写入数据目录下的 app.log
pub fn app_log(msg: &str) {
    let path = app_data_dir().join("app.log");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap_or_else(|_| panic!("无法打开日志文件: {:?}", path));
    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    writeln!(file, "[{}] {}", timestamp, msg).ok();
}