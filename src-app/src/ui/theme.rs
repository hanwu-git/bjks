//! 三主题配色映射：将旧版 CSS 变量映射为 egui Visuals
//! a 青岚（浅色）/ b 暗夜（深色）/ c 暖沙（暖色）

use crate::egui;
use egui::Color32;

#[derive(Clone, Copy)]
pub struct ThemeColors {
    pub bg: Color32,           // 窗口底
    pub panel: Color32,        // 面板/栏位底
    pub panel_alt: Color32,    // 表头/次级面板底
    #[allow(dead_code)]
    pub muted_bg: Color32,     // 次级按钮悬停底
    pub input_bg: Color32,     // 输入框底
    pub text: Color32,         // 主文字
    pub text_sec: Color32,     // 次级文字
    pub text_faint: Color32,   // 弱文字
    pub primary: Color32,      // 主色
    #[allow(dead_code)]
    pub primary_hover: Color32,
    pub primary_text: Color32, // 主色上的文字
    pub zebra: Color32,        // 斑马纹
    pub row_hover: Color32,    // 行悬停
    pub row_selected: Color32, // 行选中
    pub border: Color32,       // 常规边线
    pub border_strong: Color32,
    pub danger: Color32,
    pub btn_hover: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub fn theme_colors(id: char) -> ThemeColors {
    match id {
        'b' => ThemeColors {
            bg: rgb(0x15, 0x16, 0x1E),
            panel: rgb(0x22, 0x23, 0x2E),
            panel_alt: rgb(0x26, 0x28, 0x38),
            muted_bg: rgb(0x2A, 0x2B, 0x38),
            input_bg: rgb(0x1A, 0x1B, 0x24),
            text: rgb(0xE2, 0xE4, 0xEA),
            text_sec: rgb(0x98, 0xA0, 0xAE),
            text_faint: rgb(0x6B, 0x72, 0x80),
            primary: rgb(0x7A, 0xA2, 0xF7),
            primary_hover: rgb(0x8F, 0xB0, 0xF9),
            primary_text: rgb(0x15, 0x16, 0x1E),
            zebra: rgb(0x1C, 0x1D, 0x27),
            row_hover: rgb(0x2A, 0x2C, 0x3D),
            row_selected: rgb(0x31, 0x41, 0x5E),
            border: rgb(0x33, 0x35, 0x4A),
            border_strong: rgb(0x3D, 0x40, 0x59),
            danger: rgb(0xF8, 0x71, 0x71),
            btn_hover: rgb(0x2E, 0x30, 0x40),
        },
        'c' => ThemeColors {
            bg: rgb(0xFA, 0xF9, 0xF6),
            panel: rgb(0xFF, 0xFF, 0xFF),
            panel_alt: rgb(0xF5, 0xF0, 0xE8),
            muted_bg: rgb(0xF3, 0xEF, 0xE7),
            input_bg: rgb(0xFF, 0xFF, 0xFF),
            text: rgb(0x43, 0x35, 0x22),
            text_sec: rgb(0x8A, 0x7D, 0x6B),
            text_faint: rgb(0xA8, 0x9C, 0x8A),
            primary: rgb(0xC2, 0x41, 0x0C),
            primary_hover: rgb(0xA6, 0x37, 0x09),
            primary_text: rgb(0xFF, 0xFF, 0xFF),
            zebra: rgb(0xF5, 0xF2, 0xEC),
            row_hover: rgb(0xFA, 0xEF, 0xE3),
            row_selected: rgb(0xF9, 0xE2, 0xC8),
            border: rgb(0xEA, 0xE4, 0xD9),
            border_strong: rgb(0xDD, 0xD4, 0xC4),
            danger: rgb(0xDC, 0x26, 0x26),
            btn_hover: rgb(0xF0, 0xEB, 0xE1),
        },
        // 默认方案 A 青岚
        _ => ThemeColors {
            bg: rgb(0xF8, 0xFA, 0xFC),
            panel: rgb(0xFF, 0xFF, 0xFF),
            panel_alt: rgb(0xEE, 0xF2, 0xF7),
            muted_bg: rgb(0xF1, 0xF3, 0xF7),
            input_bg: rgb(0xFF, 0xFF, 0xFF),
            text: rgb(0x1E, 0x29, 0x3B),
            text_sec: rgb(0x64, 0x74, 0x8B),
            text_faint: rgb(0x94, 0xA3, 0xB8),
            primary: rgb(0x0F, 0x76, 0x6E),
            primary_hover: rgb(0x0D, 0x5F, 0x58),
            primary_text: rgb(0xFF, 0xFF, 0xFF),
            zebra: rgb(0xF1, 0xF5, 0xF9),
            row_hover: rgb(0xE6, 0xF4, 0xF2),
            row_selected: rgb(0xCD, 0xEA, 0xE6),
            border: rgb(0xE2, 0xE8, 0xF0),
            border_strong: rgb(0xCB, 0xD5, 0xE1),
            danger: rgb(0xDC, 0x26, 0x26),
            btn_hover: rgb(0xE8, 0xED, 0xF3),
        },
    }
}

/// 应用主题到 egui 上下文
pub fn apply(ctx: &egui::Context, id: char) {
    let c = theme_colors(id);
    let dark = id == 'b';
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    v.panel_fill = c.panel;
    v.window_fill = c.panel;
    v.extreme_bg_color = c.input_bg;
    v.faint_bg_color = c.zebra;
    v.override_text_color = Some(c.text);
    v.selection.bg_fill = c.row_selected;
    v.selection.stroke = egui::Stroke::new(1.0_f32, c.primary);
    v.window_stroke = egui::Stroke::new(1.0_f32, c.border_strong);
    v.widgets.noninteractive.bg_fill = c.panel_alt;
    v.widgets.noninteractive.weak_bg_fill = c.panel_alt;
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, c.text_sec);
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, c.border);
    v.widgets.inactive.weak_bg_fill = c.panel;
    v.widgets.inactive.bg_fill = c.panel;
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0_f32, c.text);
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, c.border_strong);
    v.widgets.hovered.weak_bg_fill = c.btn_hover;
    v.widgets.hovered.bg_fill = c.btn_hover;
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0_f32, c.text);
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, c.primary);
    v.widgets.active.weak_bg_fill = c.row_hover;
    v.widgets.active.bg_fill = c.row_hover;
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0_f32, c.primary);

    let mut style = (*ctx.style()).clone();
    style.visuals = v;
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    ctx.set_style(style);
}

/// 全局字体安装：中文字体运行时加载（微软雅黑等系统字体），egui 默认字体不含 CJK
pub fn install_fonts(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        "msyh.ttc", // 微软雅黑
        "msyh.ttf",
        "simhei.ttf", // 黑体
        "simsun.ttc", // 宋体
    ];

    // 不带 default_fonts feature：系统雅黑覆盖拉丁+中文，置于字体链首位
    let mut fonts = egui::FontDefinitions::default();
    for name in CANDIDATES {
        let path = std::path::Path::new(r"C:\Windows\Fonts").join(name);
        match std::fs::read(&path) {
            Ok(bytes) => {
                fonts
                    .font_data
                    .insert("cjk".to_string(), egui::FontData::from_owned(bytes).into());
                fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default()
                    .insert(0, "cjk".into());
                fonts
                    .families
                    .entry(egui::FontFamily::Monospace)
                    .or_default()
                    .insert(0, "cjk".into());
                crate::logger::app_log(&format!("已加载系统字体: {}", path.display()));
                break;
            }
            Err(_) => continue,
        }
    }

    // emoji 字形兜底（egui 光栅化为黑白轮廓；彩色文件图标走系统 Shell 图标纹理）
    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\seguiemj.ttf") {
        fonts
            .font_data
            .insert("emoji".to_string(), egui::FontData::from_owned(bytes).into());
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .push("emoji".into());
    }

    ctx.set_fonts(fonts);
}
