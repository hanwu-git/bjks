use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: u64,              // 唯一标识（MFT Record Number 或自增）
    pub name: String,         // 文件名（含扩展名）
    pub name_lower: String,   // 小写文件名（预计算，加速搜索）
    pub path: String,         // 完整路径
    pub parent_path: String,  // 父目录路径
    pub extension: String,    // 扩展名（不含点号，小写）
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
    pub offset: usize,           // 分页偏移
    pub limit: usize,            // 每页数量
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SortField {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppVersionInfo {
    pub version: String,
    pub name: String,
}
