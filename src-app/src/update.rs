//! 更新检查：启动 10 秒后经 WinHTTP 探测 GitHub 最新 Release。
//! 网络不通 / 超时 / 解析失败一律静默（只写日志），绝不弹错误框；
//! 仅当远端版本号大于当前版本时才置位共享状态，由 UI 弹出提示。

use crate::logger::app_log;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Release 查询接口与下载页（仓库原始地址）
const API_HOST: &str = "api.github.com";
const API_PATH: &str = "/repos/hanwu-git/bjks/releases/latest";
const DOWNLOAD_URL: &str = "https://github.com/hanwu-git/bjks/releases/latest";

/// 探测到的更新信息（ui 层读取后弹窗提示）
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    /// 新版本号（去掉前导 v）
    pub version: String,
    /// 下载页地址
    pub url: String,
    /// Release 说明（截断后的纯文本）
    pub notes: Option<String>,
}

/// 延迟 10 秒后台探测，不占用启动时间；结果写入共享状态并请求重绘。
/// 返回的共享句柄由 BjksApp 持有，每帧轮询。
pub fn start_delayed_check(ctx: crate::egui::Context) -> Arc<Mutex<Option<UpdateInfo>>> {
    let shared = Arc::new(Mutex::new(None));
    let shared_for_thread = shared.clone();
    std::thread::spawn(move || {
        // 等窗口与索引就绪后再探测
        std::thread::sleep(Duration::from_secs(10));
        match check_latest() {
            Ok(Some(info)) => {
                app_log(&format!(
                    "更新检查：发现新版本 v{}（当前 v{}），已提示用户",
                    info.version,
                    env!("CARGO_PKG_VERSION")
                ));
                *shared_for_thread.lock().unwrap() = Some(info);
                ctx.request_repaint();
            }
            Ok(None) => app_log("更新检查：已是最新版本"),
            // 网不通等情况：只落日志，不打扰用户
            Err(e) => app_log(&format!("更新检查：不可用（{}）", e)),
        }
    });
    shared
}

/// 探测并比较版本；Ok(None) 表示无新版本，Err 表示本次检查不可用
pub fn check_latest() -> Result<Option<UpdateInfo>, String> {
    let json = fetch_latest_json()?;
    let mut rel: Release = serde_json::from_str(json.trim())
        .map_err(|e| format!("响应解析失败（{} 字节）: {}", json.len(), e))?;
    if !is_newer(env!("CARGO_PKG_VERSION"), &rel.tag_name) {
        return Ok(None);
    }
    // 说明文本截断到 600 字符，避免弹窗过长
    let notes = rel.body.take().map(|b| {
        let b = b.trim();
        let mut s: String = b.chars().take(600).collect();
        if b.chars().count() > 600 {
            s.push_str("……");
        }
        s
    });
    let url = if rel.html_url.trim().is_empty() {
        DOWNLOAD_URL.to_string()
    } else {
        rel.html_url.trim().to_string()
    };
    Ok(Some(UpdateInfo {
        version: parse_ver_str(&rel.tag_name).to_string(),
        url,
        notes,
    }))
}

/// 用系统默认浏览器打开下载页
pub fn open_in_browser(url: &str) {
    unsafe {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        let verb = wide("open");
        let file = wide(url);
        ShellExecuteW(
            0,
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        );
    }
}

// ---- 版本比较 ----

/// "v3.5.4" -> "3.5.4"
fn parse_ver_str(s: &str) -> &str {
    s.trim().trim_start_matches(['v', 'V'])
}

/// "3.5.4" -> [3, 5, 4]；非法/带后缀返回 None
fn parse_ver(s: &str) -> Option<[u32; 3]> {
    let s = parse_ver_str(s);
    let mut out = [0u32; 3];
    let mut it = s.split('.');
    for slot in out.iter_mut() {
        *slot = it.next()?.trim().parse().ok()?;
    }
    Some(out)
}

/// 远端版本是否比当前新（比较失败视为不更新）
pub fn is_newer(current: &str, remote: &str) -> bool {
    match (parse_ver(current), parse_ver(remote)) {
        (Some(c), Some(r)) => r > c,
        _ => false,
    }
}

// ---- WinHTTP 请求（系统自带，不引第三方 HTTP 依赖） ----

const WINHTTP_ACCESS_TYPE_DEFAULT_PROXY: u32 = 0;
const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
const INTERNET_DEFAULT_HTTPS_PORT: u16 = 443;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: Option<String>,
}

/// GET https://api.github.com{API_PATH}，返回响应体文本
fn fetch_latest_json() -> Result<String, String> {
    use windows_sys::Win32::Networking::WinHttp::{WinHttpCloseHandle, WinHttpConnect, WinHttpOpen};

    // windows-sys 未导出该类型别名，按 WinHTTP 原始定义自定义
    type HINTERNET = *mut core::ffi::c_void;

    unsafe {
        let agent = wide("BJKS-Updater");
        let h_session: HINTERNET = WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        );
        if h_session.is_null() {
            return Err("WinHttpOpen 失败".to_string());
        }

        let outcome: Result<String, String>;
        let host = wide(API_HOST);
        let h_conn: HINTERNET =
            WinHttpConnect(h_session, host.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0);
        if h_conn.is_null() {
            outcome = Err("WinHttpConnect 失败".to_string());
        } else {
            outcome = run_request(h_conn);
        }

        if !h_conn.is_null() {
            WinHttpCloseHandle(h_conn);
        }
        WinHttpCloseHandle(h_session);
        return outcome;
    }

    unsafe fn run_request(h_conn: HINTERNET) -> Result<String, String> {
        use windows_sys::Win32::Networking::WinHttp::{
            WinHttpCloseHandle, WinHttpOpenRequest, WinHttpQueryDataAvailable,
            WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
            WinHttpSetTimeouts,
        };

        let verb = wide("GET");
        let path = wide(API_PATH);
        let headers = wide(
            "User-Agent: BJKS-Updater\r\nAccept: application/vnd.github+json\r\n",
        );

        let h_req = WinHttpOpenRequest(
            h_conn,
            verb.as_ptr(),
            path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        );
        if h_req.is_null() {
            return Err("WinHttpOpenRequest 失败".to_string());
        }
        let result = send_and_read(h_req, headers.as_ptr());
        WinHttpCloseHandle(h_req);
        return result;

        unsafe fn send_and_read(h_req: HINTERNET, headers: *const u16) -> Result<String, String> {
            // 域名解析 5s / 连接 5s / 发送 5s / 接收 10s，全部在后台线程内
            if WinHttpSetTimeouts(h_req, 5000, 5000, 5000, 10000) == 0 {
                return Err("WinHttpSetTimeouts 失败".to_string());
            }
            if WinHttpSendRequest(h_req, headers, u32::MAX, std::ptr::null_mut(), 0, 0, 0) == 0 {
                return Err("请求发送失败（网络不可达？）".to_string());
            }
            if WinHttpReceiveResponse(h_req, std::ptr::null_mut()) == 0 {
                return Err("接收响应失败".to_string());
            }

            // 状态码不单独校验：GitHub 出错时响应里不会有 tag_name，JSON 解析会失败并静默跳过
            let mut body = Vec::new();
            loop {
                let mut avail: u32 = 0;
                if WinHttpQueryDataAvailable(h_req, &mut avail) == 0 {
                    let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
                    return Err(format!("读取响应长度失败（GetLastError={}）", err));
                }
                if avail == 0 {
                    break;
                }
                let mut buf = vec![0u8; avail as usize];
                let mut read: u32 = 0;
                if WinHttpReadData(h_req, buf.as_mut_ptr() as *mut _, avail, &mut read) == 0 {
                    let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
                    return Err(format!("读取响应内容失败（GetLastError={}）", err));
                }
                body.extend_from_slice(&buf[..read as usize]);
            }
            String::from_utf8(body).map_err(|_| "响应不是 UTF-8".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ver() {
        assert_eq!(parse_ver("3.5.3"), Some([3, 5, 3]));
        assert_eq!(parse_ver("v3.5.4"), Some([3, 5, 4]));
        assert_eq!(parse_ver("V4.0.0"), Some([4, 0, 0]));
        assert_eq!(parse_ver("abc"), None);
        assert_eq!(parse_ver("3.5"), None);
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("3.5.3", "v3.5.4"));
        assert!(is_newer("3.5.3", "4.0.0"));
        assert!(is_newer("3.5.9", "3.6.0"));
        assert!(!is_newer("3.5.3", "v3.5.3"));
        assert!(!is_newer("3.5.4", "3.5.3"));
        assert!(!is_newer("3.5.3", "garbage"));
    }
}
