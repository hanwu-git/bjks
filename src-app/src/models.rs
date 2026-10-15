use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: Box<str>,       // 文件名（含扩展名）
    pub path: Box<str>,       // 完整路径
    pub extension: Box<str>,  // 扩展名（不含点号，小写）
    pub size: u64,            // 文件大小（字节）
    pub modified: i64,        // 修改时间（Unix 时间戳）
    pub is_dir: bool,         // 是否为目录
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub excluded_paths: Vec<String>,       // 排除的目录路径
    pub excluded_file_patterns: Vec<String>, // 排除的文件名模式（glob）
    pub excluded_extensions: Vec<String>,    // 排除的扩展名
    pub scan_drives: Vec<String>,           // 要扫描的盘符
    pub max_results: usize,                 // 最大返回结果数
    #[serde(default)]                       // 上次关闭时的窗口宽度（0 表示未记录，使用默认尺寸）
    pub window_width: u32,
    #[serde(default)]                       // 上次关闭时的窗口高度（0 表示未记录，使用默认尺寸）
    pub window_height: u32,
    #[serde(default = "default_close_action")] // 关闭窗口行为：minimize（最小化到托盘）/ exit（直接退出）
    pub close_action: String,
    #[serde(default)]                       // 列表列宽（像素，顺序：图标/文件名/路径/修改时间/大小；空表示使用默认）
    pub column_widths: Vec<u32>,
    #[serde(default = "default_theme")]     // 界面配色：a 青岚（浅色）/ b 暗夜（深色）/ c 暖沙（暖色）
    pub theme: String,
}

/// 界面配色默认值：方案 A 青岚，兼容旧配置文件缺字段的情况
fn default_theme() -> String {
    "a".to_string()
}

/// 关闭窗口的默认行为：最小化到托盘，兼容旧配置文件缺字段的情况
fn default_close_action() -> String {
    "minimize".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            excluded_paths: vec![
                "C:\\Windows".into(),
                "C:\\Program Files".into(),
                "C:\\Program Files (x86)".into(),
                "C:\\ProgramData".into(),
                "C:\\$Recycle.Bin".into(),
                "C:\\System Volume Information".into(),
                "C:\\pagefile.sys".into(),
                "C:\\swapfile.sys".into(),
                "C:\\hiberfil.sys".into(),
            ],
            excluded_file_patterns: vec![
                "Thumbs.db".into(),
                "desktop.ini".into(),
                "*.tmp".into(),
            ],
            excluded_extensions: vec![],
            scan_drives: vec!["C:".into(), "D:".into()],  // 默认只索引 C 盘和 D 盘
            max_results: 1000,
            window_width: 1200,
            window_height: 800,
            close_action: default_close_action(),
            column_widths: vec![],
            theme: default_theme(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortOrder {
    pub field: SortField,
    pub asc: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,           // 文件名搜索输入
    pub ext_query: String,       // 扩展名搜索输入
    pub only_folders: bool,      // 是否只显示文件夹
    pub sort_by: SortField,      // 排序字段（兼容单字段排序）
    pub sort_asc: bool,          // 是否升序
    #[serde(default)]
    pub sort_orders: Vec<SortOrder>, // 组合排序（优先使用）
    #[serde(default)]
    pub filter_dirs: Vec<String>, // 目录范围筛选（完整路径，以 \ 结尾；空为全盘，多路径任一匹配）
    pub offset: usize,           // 分页偏移
    pub limit: usize,            // 每页数量
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortField {
    None,
    Name,
    Size,
    Modified,
    Path,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<FileEntry>,
    pub total: usize,            // 匹配的总数
    pub query_time_ms: u64,      // 搜索耗时（毫秒）
    pub index_count: usize,      // 索引中的文件总数
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStatus {
    pub is_scanning: bool,
    pub scanned_files: usize,
    pub total_estimated: usize,  // 0 表示未知
    pub progress_percent: f32,
    pub total_volumes: usize,     // 本次扫描的盘总数
    pub completed_volumes: usize, // 已扫描完成的盘数
    pub current_volume: String,   // 当前正在扫描的盘符（如 "D:\\"）
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScanProgress {
    pub total_volumes: usize,
    pub completed_volumes: usize,
    pub current_volume: String,
    pub scanned_files: usize,
}
