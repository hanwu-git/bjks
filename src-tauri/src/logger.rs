use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// Windows 本地时间（替代 chrono，仅取本机时间即可满足日志需求）
#[cfg(windows)]
mod local_time {
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(lp_system_time: *mut SystemTime);
    }

    /// 返回格式化的本地时间字符串，如 2026-10-10 09:30:12.345
    pub fn now_string() -> String {
        let mut st = SystemTime::default();
        unsafe {
            GetLocalTime(&mut st);
        }
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            st.year, st.month, st.day, st.hour, st.minute, st.second, st.milliseconds
        )
    }
}

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
    let timestamp = local_time::now_string();
    writeln!(file, "[{}] {}", timestamp, msg).ok();
}