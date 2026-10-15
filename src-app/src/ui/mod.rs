//! 主界面：搜索栏、排序栏、虚拟化结果表格、底部状态栏

mod about_window;
mod settings_window;
pub mod theme;

use crate::logger::app_log;
use crate::models::{FileEntry, SearchRequest, SortField, SortOrder};
use crate::service::{self, AppState};
use crate::egui;
use egui_extras::{Column, TableBuilder};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 托盘事件类型
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrayMsg {
    Show,
    Quit,
}

/// 文件类型下拉项（与旧版一致，选择仅填充扩展名过滤框）
const EXT_TYPES: [&str; 8] = [
    "所有", "音频", "压缩文件", "文档", "可执行文件", "文件夹", "图片", "视频",
];
const TYPE_ALL: usize = 0;
const TYPE_FOLDER: usize = 5;
const SORT_FIELD_LABELS: [(SortField, &str); 4] = [
    (SortField::Name, "名称"),
    (SortField::Path, "路径"),
    (SortField::Size, "大小"),
    (SortField::Modified, "修改时间"),
];
const DEFAULT_COL_WIDTHS: [f32; 6] = [50.0, 40.0, 260.0, 360.0, 150.0, 110.0];

/// 每页结果条数（与旧版 v1 一致）
const PAGE_SIZE: usize = 1000;

fn type_ext_query(i: usize) -> &'static str {
    match i {
        1 => "mp3 wav flac aac ogg wma m4a",
        2 => "zip rar 7z tar gz bz2 xz",
        3 => "doc docx pdf txt xls xlsx ppt pptx md csv",
        4 => "exe msi bat cmd ps1",
        6 => "jpg jpeg png gif bmp webp svg ico",
        7 => "mp4 avi mkv mov wmv flv webm",
        _ => "",
    }
}

/// 行操作动作（帧内收集，帧末执行，避免借用冲突）
enum RowAction {
    OpenFile(String),
    OpenFolder(String),
}

pub struct BjksApp {
    pub state: AppState,
    pub ctx: egui::Context,

    // 配置缓存
    pub close_action: String,
    pub theme_id: char,

    // 搜索状态
    query: String,
    ext_input: String,
    ext_type: usize,
    prev_ext_type: usize,
    sort_orders: Vec<SortOrder>,
    results: Vec<FileEntry>,
    total: usize,
    query_time_ms: u64,

    // 分页状态
    current_page: usize,  // 当前页码（从 1 开始）
    page_jump: String,    // 页码跳转输入
    jump_has_focus: bool, // 跳转输入框聚焦时屏蔽全局快捷键

    // 表格
    col_widths: [f32; 6],
    col_dirty: bool,
    col_saved_at: Instant,
    table_gen: u64,
    selected: Option<usize>,
    search_has_focus: bool,
    first_frame: bool,

    // 系统彩色图标纹理缓存（key：扩展名小写带点 / "__dir__"）
    icon_cache: std::collections::HashMap<String, egui::TextureHandle>,

    // 窗口开关与设置面板
    pub show_settings: bool,
    pub show_about: bool,
    pub show_readme: bool,
    pub settings_edit: Option<crate::models::AppConfig>,
    pub settings_error: Option<String>,
    pub drives_cache: Vec<String>,
    pub theme_before: char,
    pub rescan_confirm: bool,
    pub drive_confirm: bool,
    pub exclude_input_path: String,
    pub exclude_input_pattern: String,
    pub exclude_input_extension: String,

    // 托盘
    pub _tray: Option<tray_icon::TrayIcon>,
    pub tray_msgs: Arc<Mutex<Vec<TrayMsg>>>,

    // 窗口尺寸节流写回（points）
    pending_size: Option<(f32, f32, Instant)>,
}

impl BjksApp {
    pub fn new(ctx: egui::Context, state: AppState) -> Self {
        let config = service::get_config();
        let theme_id = match config.theme.as_str() {
            "a" | "b" | "c" => config.theme.chars().next().unwrap_or('a'),
            _ => 'a',
        };

        // 托盘初始化
        let (tray, tray_msgs) = match crate::load_app_icon() {
            Some(icon) => {
                let (tray, msgs) = setup_tray(&icon, ctx.clone());
                (tray, msgs)
            }
            None => (None, Arc::new(Mutex::new(Vec::new()))),
        };

        theme::apply(&ctx, theme_id);

        // 恢复列宽（兼容旧版 6 项配置；不足或类型不符时用默认值）
        let mut col_widths = DEFAULT_COL_WIDTHS;
        if config.column_widths.len() == 6 {
            for (i, w) in config.column_widths.iter().enumerate() {
                col_widths[i] = (*w as f32).max(40.0);
            }
        }

        Self {
            state,
            ctx: ctx.clone(),
            close_action: config.close_action.clone(),
            theme_id,
            query: String::new(),
            ext_input: String::new(),
            ext_type: TYPE_ALL,
            prev_ext_type: TYPE_ALL,
            sort_orders: vec![SortOrder {
                field: SortField::Name,
                asc: true,
            }],
            results: Vec::new(),
            total: 0,
            query_time_ms: 0,
            current_page: 1,
            page_jump: String::new(),
            jump_has_focus: false,
            col_widths,
            col_dirty: false,
            col_saved_at: Instant::now(),
            table_gen: 0,
            selected: None,
            search_has_focus: false,
            first_frame: true,
            icon_cache: std::collections::HashMap::new(),
            show_settings: false,
            show_about: false,
            show_readme: false,
            settings_edit: None,
            settings_error: None,
            drives_cache: Vec::new(),
            theme_before: theme_id,
            rescan_confirm: false,
            drive_confirm: false,
            exclude_input_path: String::new(),
            exclude_input_pattern: String::new(),
            exclude_input_extension: String::new(),
            _tray: tray,
            tray_msgs,
            pending_size: None,
        }
    }

    // ---- 搜索与排序 ----

    pub fn do_search(&mut self) {
        self.current_page = 1;
        self.run_search();
    }

    /// 按当前页码执行分页搜索（每页 PAGE_SIZE 条）
    fn run_search(&mut self) {
        let request = SearchRequest {
            query: self.query.trim().to_string(),
            ext_query: self.ext_input.trim().to_string(),
            only_folders: self.ext_type == TYPE_FOLDER,
            sort_by: self
                .sort_orders
                .first()
                .map(|o| o.field)
                .unwrap_or(SortField::None),
            sort_asc: self.sort_orders.first().map(|o| o.asc).unwrap_or(true),
            sort_orders: self.sort_orders.clone(),
            offset: (self.current_page.saturating_sub(1)) * PAGE_SIZE,
            limit: PAGE_SIZE,
        };

        let response = {
            let engine = self.state.engine.lock().unwrap();
            engine.search_with_limit(&request, PAGE_SIZE)
        };
        app_log(&format!(
            "搜索完成: query={}, total={}, page={}, query_time_ms={}",
            request.query, response.total, self.current_page, response.query_time_ms
        ));
        self.results = response.results;
        self.total = response.total;
        self.query_time_ms = response.query_time_ms;
        self.selected = None;
        self.page_jump.clear();
        self.table_gen += 1; // 重置表格滚动位置
    }

    fn total_pages(&self) -> usize {
        if self.total == 0 {
            1
        } else {
            self.total.div_ceil(PAGE_SIZE)
        }
    }

    fn go_to_page(&mut self, page: usize) {
        let p = page.clamp(1, self.total_pages());
        if p != self.current_page {
            self.current_page = p;
            self.run_search();
        } else {
            self.page_jump.clear();
        }
    }

    fn handle_sort_click(&mut self, field: SortField) {
        if let Some(i) = self
            .sort_orders
            .iter()
            .position(|o| o.field == field)
        {
            self.sort_orders[i].asc = !self.sort_orders[i].asc;
        } else {
            self.sort_orders.push(SortOrder { field, asc: true });
        }
        self.do_search();
    }

    fn reset_sort(&mut self) {
        self.sort_orders.clear();
        self.do_search();
    }

    fn sort_button_label(&self, field: SortField) -> String {
        let label = SORT_FIELD_LABELS
            .iter()
            .find(|(f, _)| *f == field)
            .map(|(_, l)| *l)
            .unwrap_or("");
        if let Some(i) = self.sort_orders.iter().position(|o| o.field == field) {
            let arrow = if self.sort_orders[i].asc { "▼" } else { "▲" };
            let num = if self.sort_orders.len() > 1 {
                format!("{}", i + 1)
            } else {
                String::new()
            };
            format!("{}{} {}", label, num, arrow)
        } else {
            label.to_string()
        }
    }

    fn is_sort_active(&self, field: SortField) -> bool {
        self.sort_orders.iter().any(|o| o.field == field)
    }

    // ---- 托盘 ----

    fn handle_tray_msgs(&mut self, ctx: &egui::Context) {
        let msgs: Vec<TrayMsg> = {
            let mut guard = self.tray_msgs.lock().unwrap();
            std::mem::take(&mut *guard)
        };
        for msg in msgs {
            match msg {
                TrayMsg::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    app_log("托盘：恢复主窗口");
                }
                TrayMsg::Quit => {
                    app_log("托盘：退出程序");
                    self.save_window_size_now();
                    std::process::exit(0);
                }
            }
        }
    }

    // ---- 键盘导航 ----

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.show_settings || self.show_about || self.show_readme {
            return;
        }
        let (up, down, enter) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Enter),
            )
        });
        let n = self.results.len();

        // 搜索框/页码跳转框聚焦时：箭头/回车交给输入框逻辑处理
        if self.search_has_focus || self.jump_has_focus {
            return;
        }
        if down && n > 0 {
            let next = self.selected.map(|s| s + 1).unwrap_or(0).min(n - 1);
            self.selected = Some(next);
        }
        if up && n > 0 {
            let prev = self.selected.map(|s| s.saturating_sub(1)).unwrap_or(0);
            self.selected = Some(prev);
        }
        if enter {
            match self.selected {
                Some(idx) if idx < n => {
                    let entry = self.results[idx].clone();
                    if entry.is_dir {
                        service::open_folder(&entry.path);
                    } else {
                        service::open_file(&entry.path);
                    }
                }
                _ => self.do_search(),
            }
        }
    }

    // ---- 系统图标 ----

    /// 计算条目的图标缓存键与是否目录
    fn icon_key_of(entry: &FileEntry) -> (String, bool) {
        if entry.is_dir {
            ("__dir__".to_string(), true)
        } else {
            let key = std::path::Path::new(entry.path.as_ref())
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| format!(".{}", s.to_lowercase()))
                .unwrap_or_default();
            (key, false)
        }
    }

    /// 按键取系统彩色图标纹理（懒加载 + 缓存）；系统 API 失败时返回 None，调用方回退 emoji
    fn icon_texture_by_key(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        is_dir: bool,
    ) -> Option<egui::TextureHandle> {
        if let Some(tex) = self.icon_cache.get(key) {
            return Some(tex.clone());
        }
        let (rgba, w, h) = crate::sysicon::load_system_icon(key, is_dir)?;
        let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
        let tex = ctx.load_texture(
            format!("sysicon_{}", key),
            image,
            egui::TextureOptions::LINEAR,
        );
        self.icon_cache.insert(key.to_string(), tex.clone());
        Some(tex)
    }

    // ---- 面板 ----

    fn top_bar(&mut self, ui_root: &mut egui::Ui) {
        let c = theme::theme_colors(self.theme_id);
        egui::Panel::top("search_bar")
            .frame(
                egui::Frame::default()
                    .fill(c.panel)
                    .inner_margin(egui::Margin::symmetric(8, 6)),
            )
            .show_inside(ui_root, |ui| {
                // 右到左布局：固定宽度控件从右缘依次排布，搜索框最后吃掉剩余宽度。
                // 全部控件固定高 28 + 垂直居中，保证按钮/输入框/下拉完全对齐
                const BAR_H: f32 = 28.0;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // ComboBox 收起态按钮高度取自 interact_size.y（其 .height() 是弹窗限高）
                    ui.spacing_mut().interact_size.y = BAR_H;
                    if ui
                        .add(
                            egui::Button::new("关于").min_size(egui::vec2(0.0, BAR_H)),
                        )
                        .clicked()
                    {
                        self.show_about = true;
                    }
                    if ui
                        .add(
                            egui::Button::new("⚙ 设置").min_size(egui::vec2(0.0, BAR_H)),
                        )
                        .clicked()
                    {
                        self.open_settings();
                    }
                    let ext_edit = ui.add_sized(
                        [170.0, BAR_H],
                        egui::TextEdit::singleline(&mut self.ext_input)
                            .min_size(egui::vec2(0.0, BAR_H))
                            .hint_text("扩展名过滤，如 jpg png pdf"),
                    );
                    if ext_edit.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        self.do_search();
                    }

                    egui::ComboBox::from_id_salt("ext_type_select")
                        .selected_text(EXT_TYPES[self.ext_type])
                        .width(95.0)
                        .show_ui(ui, |ui| {
                            for (i, label) in EXT_TYPES.iter().enumerate() {
                                ui.selectable_value(&mut self.ext_type, i, *label);
                            }
                        });

                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("搜索").color(c.primary_text),
                            )
                            .fill(c.primary)
                            .min_size(egui::vec2(0.0, BAR_H)),
                        )
                        .clicked()
                    {
                        self.do_search();
                    }

                    // 搜索框：占据最左侧剩余的全部宽度
                    let resp = ui.add_sized(
                        [ui.available_width().max(120.0), BAR_H],
                        egui::TextEdit::singleline(&mut self.query)
                            .min_size(egui::vec2(0.0, BAR_H))
                            .hint_text(
                                "输入关键词搜索文件... (支持多关键词，如: 福建 交流)",
                            ),
                    );
                    self.search_has_focus = resp.has_focus();
                    if self.first_frame {
                        resp.request_focus();
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.do_search();
                    }
                });
            });
        self.first_frame = false;

        // 类型下拉变更：仅填充扩展名输入框（与旧版行为一致）
        if self.ext_type != self.prev_ext_type {
            if self.ext_type == TYPE_ALL || self.ext_type == TYPE_FOLDER {
                self.ext_input.clear();
            } else {
                self.ext_input = type_ext_query(self.ext_type).to_string();
            }
            self.prev_ext_type = self.ext_type;
        }
    }

    fn sort_bar(&mut self, ui_root: &mut egui::Ui) {
        let c = theme::theme_colors(self.theme_id);
        egui::Panel::top("sort_bar")
            .frame(
                egui::Frame::default()
                    .fill(c.panel)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show_inside(ui_root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("排序:").color(c.text_sec),
                    );
                    for (field, _) in SORT_FIELD_LABELS {
                        let label = self.sort_button_label(field);
                        let btn = if self.is_sort_active(field) {
                            egui::Button::new(
                                egui::RichText::new(label).color(c.primary_text),
                            )
                            .fill(c.primary)
                        } else {
                            egui::Button::new(egui::RichText::new(label).color(c.text))
                        };
                        if ui.add(btn).clicked() {
                            self.handle_sort_click(field);
                        }
                    }
                    if ui
                        .add(egui::Button::new(
                            egui::RichText::new("重置排序").color(c.text_sec),
                        ))
                        .clicked()
                    {
                        self.reset_sort();
                    }
                    ui.label(
                        egui::RichText::new("依次点击多列可组合排序")
                            .color(c.text_faint),
                    );

                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            // right_to_left：先放的在最右，视觉顺序（左→右）为 已索引/进度/耗时
                            ui.label(
                                egui::RichText::new(format!(
                                    "搜索耗时 {}ms",
                                    self.query_time_ms
                                ))
                                .color(c.text_sec),
                            );
                            let status = self.state.index_status();
                            if status.is_scanning {
                                let total = status.total_volumes;
                                let current =
                                    (status.completed_volumes + 1).min(total.max(1));
                                if total > 0 {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "正在建立索引… 第 {}/{} 个盘",
                                            current, total
                                        ))
                                        .color(c.primary),
                                    );
                                } else {
                                    ui.label(
                                        egui::RichText::new("正在建立索引…")
                                            .color(c.primary),
                                    );
                                }
                                let pct = (status.progress_percent / 100.0).clamp(0.0, 1.0);
                                ui.add(
                                    egui::ProgressBar::new(pct)
                                        .desired_width(120.0)
                                        .desired_height(14.0)
                                        .show_percentage(),
                                );
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{}% · 已扫描 {} 个文件",
                                        status.progress_percent as u64,
                                        status.scanned_files
                                    ))
                                    .color(c.text_sec),
                                );
                            } else {
                                ui.label(format!(
                                    "已索引 {} 个文件",
                                    status.scanned_files
                                ));
                            }
                        },
                    );
                });
            });
    }

    fn results_table(&mut self, ui_root: &mut egui::Ui) {
        let c = theme::theme_colors(self.theme_id);

        // 预取本页系统彩色图标（在 panel 闭包外完成：去重后仅未缓存的扩展名调一次 Shell API）
        let icon_keys: Vec<(String, bool)> =
            self.results.iter().map(Self::icon_key_of).collect();
        {
            let ctx = self.ctx.clone();
            let mut fetched: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            for (k, d) in &icon_keys {
                if fetched.insert(k.clone()) {
                    self.icon_texture_by_key(&ctx, k, *d);
                }
            }
        }
        let icon_textures: Vec<Option<egui::TextureHandle>> = icon_keys
            .iter()
            .map(|(k, _)| self.icon_cache.get(k).cloned())
            .collect();

        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(c.bg)
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show_inside(ui_root, |ui| {
                let id_salt = format!("results_table_{}", self.table_gen);

                // 借用拆分：表格渲染期间只读 results，行状态用局部变量收集
                let page_offset = (self.current_page.saturating_sub(1)) * PAGE_SIZE;
                let results = &self.results;
                let mut selected = self.selected;
                let mut actions: Vec<RowAction> = Vec::new();
                let mut new_widths: Option<[f32; 6]> = None;

                let table = TableBuilder::new(ui)
                    .id_salt(id_salt)
                    .striped(true)
                    .resizable(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::exact(self.col_widths[0]).resizable(false))
                    .column(
                        Column::exact(self.col_widths[1])
                            .resizable(false)
                            .clip(true),
                    )
                    .column(
                        Column::initial(self.col_widths[2])
                            .resizable(true)
                            .clip(true)
                            .range(40.0..=3000.0),
                    )
                    .column(
                        Column::initial(self.col_widths[3])
                            .resizable(true)
                            .clip(true)
                            .range(40.0..=3000.0),
                    )
                    .column(
                        Column::initial(self.col_widths[4])
                            .resizable(true)
                            .range(60.0..=500.0),
                    )
                    .column(
                        Column::initial(self.col_widths[5])
                            .resizable(true)
                            .range(40.0..=500.0),
                    )
                    .column(Column::remainder());

                table
                    .header(24.0, |mut header| {
                        for title in ["序号", "", "文件名", "路径", "修改时间", "大小", ""] {
                            header.col(|ui| {
                                ui.strong(title);
                            });
                        }
                    })
                    .body(|body| {
                        // 读取实际列宽用于持久化
                        let widths = body.widths();
                        if widths.len() >= 6 {
                            let w: [f32; 6] = [
                                widths[0], widths[1], widths[2], widths[3], widths[4],
                                widths[5],
                            ];
                            new_widths = Some(w);
                        }

                        body.rows(26.0, results.len(), |mut row| {
                            let idx = row.index();
                            let entry = &results[idx];
                            let is_selected = selected == Some(idx);

                            // 序号（跨页连续编号）
                            row.col(|ui| {
                                let num = page_offset + idx + 1;
                                let t = if is_selected {
                                    egui::RichText::new(format!("{}", num)).strong()
                                } else {
                                    egui::RichText::new(format!("{}", num)).color(c.text_sec)
                                };
                                ui.label(t);
                            });
                            // 图标：系统关联彩色图标纹理（单元格内居中，尺寸不超过单元格）；
                            // 取不到时回退 emoji
                            row.col(|ui| {
                                let s = 16.0_f32
                                    .min(ui.available_width())
                                    .min(ui.available_height());
                                ui.with_layout(
                                    egui::Layout::centered_and_justified(
                                        egui::Direction::LeftToRight,
                                    ),
                                    |ui| match icon_textures.get(idx).and_then(|t| t.as_ref()) {
                                        Some(tex) => {
                                            ui.add(egui::Image::from_texture(
                                                egui::load::SizedTexture::new(
                                                    tex,
                                                    egui::vec2(s, s),
                                                ),
                                            ));
                                        }
                                        None => {
                                            ui.label(service::file_icon(entry));
                                        }
                                    },
                                );
                            });
                            // 文件名（截断 + 悬停完整名）
                            row.col(|ui| {
                                let t = egui::RichText::new(entry.name.as_ref());
                                let t = if is_selected { t.strong() } else { t };
                                let mut resp = ui.add(
                                    egui::Label::new(t)
                                        .wrap_mode(egui::TextWrapMode::Truncate),
                                );
                                if resp.hovered() {
                                    resp = resp.on_hover_text(entry.name.as_ref());
                                }
                                row_interaction(
                                    &resp,
                                    entry,
                                    &mut selected,
                                    idx,
                                    &mut actions,
                                );
                            });
                            // 路径（父目录，截断 + 悬停完整路径）
                            row.col(|ui| {
                                let parent = entry
                                    .path
                                    .rsplit_once('\\')
                                    .map(|(p, _)| p)
                                    .unwrap_or(entry.path.as_ref());
                                let t = egui::RichText::new(parent).color(c.text_sec);
                                let mut resp = ui.add(
                                    egui::Label::new(t)
                                        .wrap_mode(egui::TextWrapMode::Truncate),
                                );
                                if resp.hovered() {
                                    resp = resp.on_hover_text(entry.path.as_ref());
                                }
                                row_interaction(
                                    &resp,
                                    entry,
                                    &mut selected,
                                    idx,
                                    &mut actions,
                                );
                            });
                            // 修改时间
                            row.col(|ui| {
                                let t = egui::RichText::new(service::format_datetime(
                                    entry.modified,
                                ))
                                .color(c.text_sec);
                                ui.label(t);
                            });
                            // 大小
                            row.col(|ui| {
                                let size = if entry.is_dir {
                                    String::new()
                                } else {
                                    service::format_size(entry.size)
                                };
                                let t = egui::RichText::new(size).color(c.text_sec);
                                ui.label(t);
                            });
                            // filler
                            row.col(|_ui| {});
                        });
                    });

                // 写回行状态
                self.selected = selected;
                for action in actions {
                    match action {
                        RowAction::OpenFile(p) => {
                            service::open_file(&p);
                        }
                        RowAction::OpenFolder(p) => {
                            service::open_folder(&p);
                        }
                    }
                }
                // 列宽变化：更新并标记待保存
                if let Some(w) = new_widths {
                    if (w[2] - self.col_widths[2]).abs() > 0.5
                        || (w[3] - self.col_widths[3]).abs() > 0.5
                        || (w[4] - self.col_widths[4]).abs() > 0.5
                        || (w[5] - self.col_widths[5]).abs() > 0.5
                    {
                        self.col_widths = w;
                        self.col_dirty = true;
                        self.col_saved_at = Instant::now();
                    }
                }
            });
    }

    fn bottom_bar(&mut self, ui_root: &mut egui::Ui) {
        let c = theme::theme_colors(self.theme_id);
        let pages = self.total_pages();
        egui::Panel::bottom("status_bar")
            .frame(
                egui::Frame::default()
                    .fill(c.panel)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show_inside(ui_root, |ui| {
                ui.horizontal(|ui| {
                    // 左侧：总数与当前显示范围
                    let text = if self.total == 0 {
                        "无匹配结果".to_string()
                    } else {
                        let from = (self.current_page - 1) * PAGE_SIZE + 1;
                        let to = (self.current_page * PAGE_SIZE).min(self.total);
                        format!("共 {} 条结果（当前显示 {}–{} 条）", self.total, from, to)
                    };
                    ui.label(egui::RichText::new(text).color(c.text_sec));

                    // 右侧：分页控件（right_to_left：先加的在最右）
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui
                                .add_enabled(
                                    self.current_page < pages,
                                    egui::Button::new("下一页"),
                                )
                                .clicked()
                            {
                                self.go_to_page(self.current_page + 1);
                            }
                            // 页码跳转：输入页码后回车
                            let jump = ui.add_sized(
                                [56.0, 20.0],
                                egui::TextEdit::singleline(&mut self.page_jump)
                                    .hint_text("页码"),
                            );
                            self.jump_has_focus = jump.has_focus();
                            if jump.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            {
                                if let Ok(p) = self.page_jump.trim().parse::<usize>() {
                                    self.go_to_page(p);
                                }
                            }
                            ui.label(
                                egui::RichText::new(format!(
                                    "第 {} / {} 页",
                                    self.current_page, pages
                                ))
                                .color(c.text_sec),
                            );
                            if ui
                                .add_enabled(
                                    self.current_page > 1,
                                    egui::Button::new("上一页"),
                                )
                                .clicked()
                            {
                                self.go_to_page(self.current_page.saturating_sub(1));
                            }
                        },
                    );
                });
            });
    }

    // ---- 配置持久化 ----

    /// 记录窗口尺寸（节流 800ms）
    fn save_window_size(&mut self, w: u32, h: u32) {
        let mut config = service::get_config();
        if config.window_width != w || config.window_height != h {
            config.window_width = w;
            config.window_height = h;
            if service::save_settings(&config) {
                app_log(&format!("已记录窗口尺寸: {}x{}", w, h));
            }
        }
    }

    /// 立即保存当前窗口尺寸（退出前调用）
    fn save_window_size_now(&mut self) {
        let (w, h) = self.ctx.input(|i| {
            let r = i.viewport_rect();
            (r.width() as u32, r.height() as u32)
        });
        if w > 0 && h > 0 {
            self.save_window_size(w, h);
        }
    }

    /// 保存列宽（节流 400ms）
    fn save_column_widths(&mut self) {
        self.col_dirty = false;
        self.col_saved_at = Instant::now();
        let mut config = service::get_config();
        config.column_widths = self.col_widths.iter().map(|w| *w as u32).collect();
        service::save_settings(&config);
    }
}

/// 单元格交互：单击选中、双击打开、右键菜单
fn row_interaction(
    resp: &egui::Response,
    entry: &FileEntry,
    selected: &mut Option<usize>,
    idx: usize,
    actions: &mut Vec<RowAction>,
) {
    if resp.clicked() {
        *selected = Some(idx);
    }
    if resp.double_clicked() {
        if entry.is_dir {
            actions.push(RowAction::OpenFolder(entry.path.to_string()));
        } else {
            actions.push(RowAction::OpenFile(entry.path.to_string()));
        }
    }
    resp.context_menu(|ui| {
        if ui.button("打开文件").clicked() {
            if entry.is_dir {
                actions.push(RowAction::OpenFolder(entry.path.to_string()));
            } else {
                actions.push(RowAction::OpenFile(entry.path.to_string()));
            }
            ui.close();
        }
        if ui.button("打开所在目录").clicked() {
            actions.push(RowAction::OpenFolder(entry.path.to_string()));
            ui.close();
        }
        if ui.button("复制路径").clicked() {
            ui.ctx().copy_text(entry.path.to_string());
            ui.close();
        }
    });
}

/// 创建托盘图标与事件轮询线程
fn setup_tray(
    icon: &egui::IconData,
    ui_ctx: egui::Context,
) -> (Option<tray_icon::TrayIcon>, Arc<Mutex<Vec<TrayMsg>>>) {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let msgs = Arc::new(Mutex::new(Vec::new()));

    let menu = Menu::new();
    let show_item = MenuItem::with_id("show", "显示主窗口", true, None);
    let quit_item = MenuItem::with_id("quit", "退出程序", true, None);
    if let Err(e) = menu.append(&show_item) {
        app_log(&format!("托盘菜单构建失败: {}", e));
        return (None, msgs);
    }
    if let Err(e) = menu.append(&quit_item) {
        app_log(&format!("托盘菜单构建失败: {}", e));
        return (None, msgs);
    }

    let tray_icon = match tray_icon::Icon::from_rgba(
        icon.rgba.clone(),
        icon.width as u32,
        icon.height as u32,
    ) {
        Ok(ico) => {
            let builder = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .with_tooltip("本机快搜")
                .with_icon(ico);
            builder.build().ok()
        }
        Err(e) => {
            app_log(&format!("托盘图标转换失败: {}", e));
            None
        }
    };

    // 事件轮询线程：收集托盘/菜单事件并唤醒 UI
    let msgs_for_thread = Arc::clone(&msgs);
    let ctx_for_poll = ui_ctx.clone();
    std::thread::spawn(move || loop {
        let mut got = false;
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                msgs_for_thread.lock().unwrap().push(TrayMsg::Show);
                got = true;
            }
        }
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            match ev.id.0.as_str() {
                "show" => {
                    msgs_for_thread.lock().unwrap().push(TrayMsg::Show);
                    got = true;
                }
                "quit" => {
                    msgs_for_thread.lock().unwrap().push(TrayMsg::Quit);
                    got = true;
                }
                _ => {}
            }
        }
        if got {
            ctx_for_poll.request_repaint();
        }
        std::thread::sleep(Duration::from_millis(150));
    });

    (tray_icon, msgs)
}

impl egui_software_backend::App for BjksApp {
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        _backend: &mut egui_software_backend::SoftwareBackend,
    ) {
        let ctx = ui.ctx().clone();

        // 0. 托盘消息
        self.handle_tray_msgs(&ctx);

        // 1. 窗口尺寸变化记录（800ms 节流写回）
        let size = ctx.input(|i| {
            let r = i.viewport_rect();
            (r.width(), r.height())
        });
        let stable = match self.pending_size {
            Some((w, h, _)) => (w - size.0).abs() < 0.5 && (h - size.1).abs() < 0.5,
            None => false,
        };
        if !stable {
            self.pending_size = Some((size.0, size.1, Instant::now()));
        }
        if let Some((w, h, t)) = self.pending_size {
            if t.elapsed() >= Duration::from_millis(800) {
                self.save_window_size(w as u32, h as u32);
                self.pending_size = None;
            }
        }

        // 2. 列宽节流写盘（400ms）
        if self.col_dirty && self.col_saved_at.elapsed() >= Duration::from_millis(400) {
            self.save_column_widths();
        }

        // 3. 全局键盘导航
        self.handle_shortcuts(&ctx);

        // 4. 主界面面板
        // 注意：show_inside 模式靠 cursor 流分配空间，所有 top/bottom 面板必须先添加，
        // CentralPanel 最后添加占据中间剩余区域，否则后添加的 bottom 面板会被挤成 0 高
        self.top_bar(ui);
        self.sort_bar(ui);
        self.bottom_bar(ui);

        // 5. 盘符变更后的重建索引询问（横幅，同为 bottom 面板，须在 CentralPanel 之前）
        if self.drive_confirm {
            egui::Panel::bottom("drive_confirm_bar")
                .frame(
                    egui::Frame::default()
                        .fill(theme::theme_colors(self.theme_id).panel_alt)
                        .inner_margin(egui::Margin::symmetric(8, 6)),
                )
                .show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("索引盘符已变更，是否立即重建索引使新盘符生效？");
                        if ui.button("立即重建").clicked() {
                            self.drive_confirm = false;
                            self.start_rescan();
                        }
                        if ui.button("跳过").clicked() {
                            self.drive_confirm = false;
                        }
                    });
                });
        }

        // 中央结果表格（必须最后添加）
        self.results_table(ui);

        // 6. 弹窗（独立 Window 层，不受面板流影响）
        self.settings_window(&ctx);
        self.about_window(&ctx);
        self.readme_window(&ctx);
    }

    fn on_close_requested(&mut self, ctx: &egui::Context) -> bool {
        // 返回 false 拦截关闭并隐藏窗口（最小化到托盘）；true 放行退出
        if self.close_action != "exit" {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            app_log("窗口关闭请求：已最小化到托盘");
            false
        } else {
            self.save_window_size_now();
            app_log("窗口关闭请求：直接退出程序");
            true
        }
    }

    fn on_exit(&mut self, _ctx: &egui::Context) {
        self.save_window_size_now();
    }
}
