//! 设置窗口：关闭行为 / 界面配色 / 索引盘符 / 屏蔽项管理 / 重建索引

use super::theme;
use crate::indexer::normalize_drive;
use crate::logger::app_log;
use crate::service;
use crate::egui;
use crate::ui::BjksApp;
use egui::RichText;

impl BjksApp {
    pub fn open_settings(&mut self) {
        self.show_settings = true;
    }

    pub fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        // 打开时载入配置副本
        if self.settings_edit.is_none() {
            self.settings_edit = Some(service::get_config());
            self.drives_cache = service::list_volumes();
            self.theme_before = self.theme_id;
            self.rescan_confirm = false;
            self.exclude_input_path.clear();
            self.exclude_input_pattern.clear();
            self.exclude_input_extension.clear();
            self.settings_error = None;
        }

        let mut open_flag = self.show_settings;
        egui::Window::new(RichText::new("⚙ 设置").strong())
            .open(&mut open_flag)
            .resizable(false)
            .collapsible(false)
            .default_width(460.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.screen_rect().center())
            .show(ctx, |ui| {
                self.settings_content(ui);
            });
        self.show_settings = open_flag;

        // 关闭（未保存）时还原打开前的配色预览
        if !self.show_settings {
            self.settings_edit = None;
            if self.theme_id != self.theme_before {
                self.theme_id = self.theme_before;
                theme::apply(ctx, self.theme_id);
            }
        }
    }

    fn settings_content(&mut self, ui: &mut egui::Ui) {
        let c = theme::theme_colors(self.theme_id);

        // ---- 顶部操作栏：固定悬浮在滚动内容上方，任何情况下按钮都可见 ----
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(RichText::new("保存").color(c.primary_text))
                        .fill(c.primary),
                )
                .clicked()
            {
                self.save_settings_handler();
            }

            // 重建索引（二次确认）
            let (label, fill, text_color) = if self.rescan_confirm {
                ("确认重建？", c.danger, egui::Color32::WHITE)
            } else {
                ("重建索引", c.btn_hover, c.text)
            };
            if ui
                .add(
                    egui::Button::new(RichText::new(label).color(text_color)).fill(fill),
                )
                .clicked()
            {
                if self.rescan_confirm {
                    self.rescan_confirm = false;
                    self.start_rescan();
                    // 关闭设置窗口（还原主题预览逻辑由关闭分支处理）
                    self.theme_before = self.theme_id;
                    self.settings_edit = None;
                    self.show_settings = false;
                } else {
                    self.rescan_confirm = true;
                }
            }
            // 错误提示（与按钮同行，悬停看全文）
            if let Some(err) = self.settings_error.as_ref() {
                ui.label(RichText::new(err).small().color(c.danger))
                    .on_hover_text(err);
            }
        });
        if self.rescan_confirm {
            ui.label(
                RichText::new(
                    "重建索引将清空现有索引并重新扫描磁盘，可能需要几分钟。再次点击按钮确认。",
                )
                .small()
                .color(c.danger),
            );
        }
        ui.separator();

        // ---- 内容区：高度限制在屏幕内（预留标题栏/顶栏/边距），超出部分内部滚动 ----
        let max_h = (ui.ctx().screen_rect().height() - 180.0).max(240.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height(max_h)
            .show(ui, |ui| {
                // ---- 关闭窗口时 ----
                ui.heading(RichText::new("🪟 关闭窗口时").size(14.0));
                if let Some(cfg) = self.settings_edit.as_mut() {
                    ui.horizontal(|ui| {
                        ui.radio_value(
                            &mut cfg.close_action,
                            "minimize".to_string(),
                            "最小化到托盘",
                        );
                        ui.radio_value(
                            &mut cfg.close_action,
                            "exit".to_string(),
                            "直接退出程序",
                        );
                    });
                }
                ui.label(
                    RichText::new(
                        "选择「最小化到托盘」后，点击关闭仅隐藏窗口，可从托盘图标左键单击恢复，或右键菜单退出程序。",
                    )
                    .small()
                    .color(c.text_sec),
                );
                ui.separator();

                // ---- 界面配色（即时预览） ----
                ui.heading(RichText::new("🎨 界面配色").size(14.0));
                if let Some(cfg) = self.settings_edit.as_mut() {
                    ui.horizontal_wrapped(|ui| {
                        for (val, label) in [
                            ("a", "青岚（青绿主色 · 浅色）"),
                            ("b", "暗夜（柔和蓝主色 · 深色）"),
                            ("c", "暖沙（琥珀主色 · 暖色）"),
                        ] {
                            ui.radio_value(&mut cfg.theme, val.to_string(), label);
                        }
                    });
                }
                // 主题选择变化后立即预览
                if let Some(cfg) = self.settings_edit.as_ref() {
                    let id = cfg.theme.chars().next().unwrap_or('a');
                    if id != self.theme_id {
                        self.theme_id = id;
                        theme::apply(ui.ctx(), self.theme_id);
                    }
                }
                ui.label(
                    RichText::new("选择界面配色方案，切换后即时预览，点「保存」后记住。")
                        .small()
                        .color(c.text_sec),
                );
                ui.separator();

                // ---- 索引盘符 ----
                ui.heading(RichText::new("💽 索引盘符").size(14.0));
                let drives = self.drives_cache.clone();
                if let Some(cfg) = self.settings_edit.as_mut() {
                    ui.horizontal_wrapped(|ui| {
                        for drive in drives {
                            let norm = normalize_drive(drive.as_str());
                            let mut checked = cfg
                                .scan_drives
                                .iter()
                                .any(|d| normalize_drive(d) == norm);
                            if ui.checkbox(&mut checked, drive.as_str()).changed() {
                                if checked {
                                    if !cfg
                                        .scan_drives
                                        .iter()
                                        .any(|d| normalize_drive(d) == norm)
                                    {
                                        cfg.scan_drives.push(norm.clone());
                                    }
                                } else {
                                    cfg.scan_drives
                                        .retain(|d| normalize_drive(d) != norm);
                                }
                            }
                        }
                    });
                }
                ui.label(
                    RichText::new(
                        "只有勾选的盘符参与索引，默认 C 盘和 D 盘。修改后保存，再点「重建索引」生效。",
                    )
                    .small()
                    .color(c.text_sec),
                );

                // ---- 屏蔽项三组 ----
                self.exclude_section(
                    ui,
                    "path",
                    "📂 屏蔽目录",
                    "请输入要屏蔽的目录路径（如 C:\\Windows）",
                );
                self.exclude_section(
                    ui,
                    "pattern",
                    "📄 屏蔽文件模式",
                    "请输入要屏蔽的文件模式（如 *.tmp）",
                );
                self.exclude_section(
                    ui,
                    "extension",
                    "🏷 屏蔽扩展名",
                    "请输入要屏蔽的扩展名（如 log）",
                );
            });
    }

    /// 屏蔽项分组：列表 + 删除按钮 + 添加输入框（借用安全：输入值先取出，用后写回）
    fn exclude_section(&mut self, ui: &mut egui::Ui, kind: &str, title: &str, hint: &str) {
        let c = theme::theme_colors(self.theme_id);

        // 取出输入缓冲（避免与 self 借用冲突）
        let mut input_val = match kind {
            "path" => std::mem::take(&mut self.exclude_input_path),
            "pattern" => std::mem::take(&mut self.exclude_input_pattern),
            _ => std::mem::take(&mut self.exclude_input_extension),
        };

        // 克隆当前列表用于渲染
        let list: Vec<String> = self
            .settings_edit
            .as_ref()
            .map(|cfg| match kind {
                "path" => cfg.excluded_paths.clone(),
                "pattern" => cfg.excluded_file_patterns.clone(),
                "extension" => cfg.excluded_extensions.clone(),
                _ => Vec::new(),
            })
            .unwrap_or_default();

        ui.separator();
        ui.heading(RichText::new(title).size(14.0));
        let mut remove_idx: Option<usize> = None;
        let mut add_value: Option<String> = None;

        egui::ScrollArea::vertical()
            .id_salt(format!("exclude_scroll_{}", kind))
            .max_height(100.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if list.is_empty() {
                    ui.label(RichText::new("（无）").color(c.text_faint));
                }
                for (i, value) in list.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(value.as_str());
                        if ui
                            .add(egui::Button::new(
                                RichText::new("删除").color(c.danger),
                            ))
                            .clicked()
                        {
                            remove_idx = Some(i);
                        }
                    });
                }
            });
        ui.horizontal(|ui| {
            let resp =
                ui.add_sized([240.0, 22.0], egui::TextEdit::singleline(&mut input_val).hint_text(hint));
            if ui.button("+ 添加").clicked()
                || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                let value = input_val.trim().to_string();
                if !value.is_empty() {
                    add_value = Some(value);
                    input_val.clear();
                }
            }
        });

        // 输入缓冲写回
        match kind {
            "path" => self.exclude_input_path = input_val,
            "pattern" => self.exclude_input_pattern = input_val,
            _ => self.exclude_input_extension = input_val,
        }

        // 写回设置副本
        if remove_idx.is_some() || add_value.is_some() {
            if let Some(cfg) = self.settings_edit.as_mut() {
                let target = match kind {
                    "path" => Some(&mut cfg.excluded_paths),
                    "pattern" => Some(&mut cfg.excluded_file_patterns),
                    "extension" => Some(&mut cfg.excluded_extensions),
                    _ => None,
                };
                if let Some(list) = target {
                    if let Some(i) = remove_idx {
                        if i < list.len() {
                            list.remove(i);
                        }
                    }
                    if let Some(value) = add_value {
                        if !list.contains(&value) {
                            list.push(value);
                        }
                    }
                }
            }
        }
    }

    /// 保存设置：校验盘符、写盘、应用主题与关闭行为；盘符变更后询问重建
    fn save_settings_handler(&mut self) {
        let cfg = match self.settings_edit.as_ref() {
            Some(cfg) => cfg.clone(),
            None => return,
        };

        // 至少一个盘符
        if cfg.scan_drives.is_empty() {
            self.settings_error = Some("请至少勾选一个参与索引的盘符".to_string());
            return;
        }

        let new_cfg = cfg;
        let normalized: Vec<String> = {
            let mut v: Vec<String> = new_cfg
                .scan_drives
                .iter()
                .map(|d| normalize_drive(d))
                .collect();
            v.sort();
            v.dedup();
            v
        };

        let prev = service::get_config();
        let prev_drives: Vec<String> = {
            let mut v: Vec<String> = prev
                .scan_drives
                .iter()
                .map(|d| normalize_drive(d))
                .collect();
            v.sort();
            v.dedup();
            v
        };
        let drives_changed = prev_drives != normalized;

        if service::save_settings(&new_cfg) {
            self.close_action = new_cfg.close_action.clone();
            self.theme_before = self.theme_id;
            self.settings_edit = None;
            self.show_settings = false;
            self.settings_error = None;
            app_log("设置已保存");
            if drives_changed {
                self.drive_confirm = true;
            }
        } else {
            self.settings_error = Some("保存设置失败".to_string());
        }
    }

    /// 触发重建索引（后台线程执行）
    pub fn start_rescan(&mut self) {
        let state = self.state.clone();
        let config = service::get_config();
        std::thread::spawn(move || {
            let ok = state.rescan(&config);
            if !ok {
                app_log("重建索引中止：没有可用的盘符");
            }
        });
        app_log("用户触发重建索引");
    }
}
