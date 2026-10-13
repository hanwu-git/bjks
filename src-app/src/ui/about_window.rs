//! 关于弹窗与程序说明文档弹窗

use super::theme;
use crate::service;
use crate::egui;
use crate::ui::BjksApp;
use egui::RichText;

/// 程序说明文档：编译期嵌入二进制，确保绿色版仅需单个 exe 即可查看说明
const README: &str = include_str!("../../../程序说明.md");

impl BjksApp {
    pub fn about_window(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let c = theme::theme_colors(self.theme_id);
        let mut open = self.show_about;
        egui::Window::new(RichText::new("关于 BJKS").strong())
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(RichText::new("BJKS（本机快搜）").strong());
                ui.label(format!(
                    "版本号：{} v{}",
                    env!("CARGO_PKG_NAME"),
                    env!("CARGO_PKG_VERSION")
                ));
                ui.label(
                    RichText::new(
                        "基于 Rust 原生 GUI（egui）开发的 Windows 本地文件快速搜索工具，无 WebView2 依赖。",
                    )
                    .color(c.text_sec),
                );
                ui.separator();
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("升级说明").color(c.primary_text),
                        )
                        .fill(c.primary),
                    )
                    .clicked()
                {
                    self.show_about = false;
                    self.show_readme = true;
                }
            });
        self.show_about = open;
    }

    pub fn readme_window(&mut self, ctx: &egui::Context) {
        if !self.show_readme {
            return;
        }
        let _ = service::get_config;
        let mut open = self.show_readme;
        egui::Window::new(RichText::new("升级说明 / 程序说明").strong())
            .open(&mut open)
            .resizable(true)
            .collapsible(false)
            .default_width(640.0)
            .default_height(500.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(README).size(12.0)).wrap());
                    });
            });
        self.show_readme = open;
    }
}
