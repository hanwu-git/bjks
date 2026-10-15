//! 系统文件图标：通过 Shell API（SHGetFileInfoW）获取与资源管理器一致的系统关联彩色小图标，
//! 经 GDI 取出 32 位像素后转为 RGBA，供 egui 作为纹理显示。全部为系统内置 API，无需外部文件。

use windows_sys::Win32::Graphics::Gdi::{
    DeleteObject, DIB_RGB_COLORS, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, HBITMAP, RGBQUAD,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES,
};
use windows_sys::Win32::UI::Shell::{
    SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

/// 取系统关联小图标，返回 (RGBA 像素, 宽, 高)。
/// `key`：扩展名（小写、带点，如 `.pdf`）；目录图标由 `is_dir` 决定；空串表示无扩展名文件。
/// 使用 SHGFI_USEFILEATTRIBUTES：按扩展名+属性取图标，文件无需真实存在，不访问磁盘。
pub fn load_system_icon(key: &str, is_dir: bool) -> Option<(Vec<u8>, usize, usize)> {
    unsafe {
        // 构造一个仅用于推断图标的文件名（不会真正访问它）
        let mut probe_wide: Vec<u16> = if is_dir {
            "Folder".encode_utf16().collect()
        } else if key.is_empty() {
            "file".encode_utf16().collect()
        } else {
            format!("file{}", key).encode_utf16().collect()
        };
        probe_wide.push(0);

        let attrs: FILE_FLAGS_AND_ATTRIBUTES = if is_dir {
            FILE_ATTRIBUTE_DIRECTORY
        } else {
            FILE_ATTRIBUTE_NORMAL
        };

        let mut info: SHFILEINFOW = std::mem::zeroed();
        let hr = SHGetFileInfoW(
            probe_wide.as_ptr(),
            attrs,
            &mut info,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES,
        );
        if hr == 0 || info.hIcon == 0 {
            return None;
        }

        let rgba = hicon_to_rgba(info.hIcon);
        DestroyIcon(info.hIcon);
        rgba
    }
}

/// HICON → 32 位 RGBA（top-down）。现代 Windows 的小图标颜色位图自带 alpha 通道。
unsafe fn hicon_to_rgba(hicon: isize) -> Option<(Vec<u8>, usize, usize)> {
    let mut ii: ICONINFO = std::mem::zeroed();
    if GetIconInfo(hicon, &mut ii) == 0 {
        return None;
    }

    // 取颜色位图尺寸
    let mut bm: BITMAP = std::mem::zeroed();
    let ok_color = ii.hbmColor != 0
        && GetObjectW(
            ii.hbmColor,
            std::mem::size_of::<BITMAP>() as i32,
            &mut bm as *mut _ as *mut _,
        ) != 0;

    if !ok_color {
        // 极旧风格的 mask-only 图标：没有颜色位图，无法取彩色，直接放弃回退 emoji
        if ii.hbmMask != 0 {
            DeleteObject(ii.hbmMask);
        }
        return None;
    }

    let w = bm.bmWidth.max(1) as i32;
    let h = bm.bmHeight.max(1) as i32;

    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // 负值 = top-down，避免行序翻转
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0, // BI_RGB
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [RGBQUAD {
            rgbBlue: 0,
            rgbGreen: 0,
            rgbRed: 0,
            rgbReserved: 0,
        }],
    };

    let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
    let hdc = GetDC(0);
    let scanned = GetDIBits(
        hdc,
        ii.hbmColor as HBITMAP,
        0,
        h as u32,
        buf.as_mut_ptr() as *mut _,
        &mut bmi,
        DIB_RGB_COLORS,
    );
    ReleaseDC(0, hdc);

    DeleteObject(ii.hbmColor);
    if ii.hbmMask != 0 {
        DeleteObject(ii.hbmMask);
    }

    if scanned == 0 {
        return None;
    }

    // BGRA → RGBA
    let mut any_alpha = false;
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
        if px[3] != 0 {
            any_alpha = true;
        }
    }
    // 个别系统图标 alpha 通道为空（视为全不透明，避免显示成全透明）
    if !any_alpha {
        for px in buf.chunks_exact_mut(4) {
            px[3] = 255;
        }
    }

    Some((buf, w as usize, h as usize))
}
