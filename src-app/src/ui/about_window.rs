//! 关于弹窗与程序说明文档弹窗

use super::theme;
use crate::logger::app_log;
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
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.viewport_rect().center())
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
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.viewport_rect().center())
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(README).size(12.0)).wrap());
                    });
            });
        self.show_readme = open;
    }

    /// 更新提示弹窗：仅当探测到新版本、未被忽略（本次会话或持久化忽略）时出现
    pub fn update_window(&mut self, ctx: &egui::Context) {
        if self.update_dismissed {
            return;
        }
        let Some(info) = self.update_available.lock().unwrap().clone() else {
            return;
        };
        // 用户已忽略该版本（配置持久化）：同版本号不再提示
        if info.version == self.ignored_update {
            return;
        }
        let c = theme::theme_colors(self.theme_id);
        egui::Window::new(RichText::new("发现新版本").strong())
            .resizable(false)
            .collapsible(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.viewport_rect().center())
            .show(ctx, |ui| {
                ui.label(format!("最新版本：v{}", info.version));
                ui.label(
                    RichText::new(format!("当前版本：v{}", env!("CARGO_PKG_VERSION")))
                        .color(c.text_sec),
                );
                if let Some(notes) = &info.notes {
                    if !notes.is_empty() {
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .max_height(140.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(RichText::new(notes).size(12.0)).wrap());
                            });
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("打开下载页").color(c.primary_text),
                            )
                            .fill(c.primary),
                        )
                        .clicked()
                    {
                        crate::update::open_in_browser(&info.url);
                        self.update_dismissed = true;
                    }
                    // 忽略此版本：写入配置，重启后同版本号也不再提示
                    if ui.button("忽略此版本").clicked() {
                        let mut cfg = service::get_config();
                        cfg.ignored_update = info.version.clone();
                        if service::save_settings(&cfg) {
                            app_log(&format!("更新提示：已忽略版本 v{}，同版本不再提示", info.version));
                        } else {
                            app_log("更新提示：忽略版本写入配置失败，仅本次会话生效");
                        }
                        self.ignored_update = info.version.clone();
                        self.update_dismissed = true;
                    }
                    // 下次再问：仅本次会话不再提示
                    if ui.button("下次再问").clicked() {
                        self.update_dismissed = true;
                    }
                });
            });
    }
}
