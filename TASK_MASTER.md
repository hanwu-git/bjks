# 本机快搜 — 全量开发任务书

> 项目代号：`本机快搜`
> 技术栈：Tauri 2.x + Rust (后端) + HTML/CSS/JS (前端)
> 目标：开发一套类 Everything 的 Windows 本地文件快速搜索工具
> 核心差异点：多段关键词搜索、扩展名模糊搜索、屏蔽路径/文件设置

---

## 执行规则

1. **任务串行执行**：按编号顺序逐个执行，不可跳步
2. **自检机制**：每个任务完成后，执行「自检项」中列出的所有验证
3. **通过 → 下一步**：自检全部通过，标记 `[x]`，进入下一个任务
4. **不通过 → 重做**：自检未通过，分析原因，修复后重新自检，直到通过
5. **人工介入条件**：仅在标记 `⚠️ 需确认` 的决策点暂停等待用户确认
6. **日志记录**：每个任务完成后，在 `dev_log.md` 中追加简要记录（耗时、结果、遇到的问题）

---

## 产品需求总览

### 核心功能

| 编号 | 功能 | 描述 |
|------|------|------|
| F1 | 快速文件索引 | 读取 NTFS MFT 枚举全盘文件，100万文件 < 10秒 |
| F2 | 多段关键词搜索 | 输入多个关键词（空格分隔），结果必须同时包含所有关键词 |
| F3 | 扩展名模糊搜索 | 输入 `.jp` 可匹配 `.jpg`、`.jpeg`；输入 `.doc` 可匹配 `.doc`、`.docx` |
| F4 | 屏蔽路径设置 | 用户可配置排除特定目录（如 `C:\Windows`）和文件模式（如 `*.tmp`） |
| F5 | 实时更新 | 文件增删改后索引自动更新，无需手动刷新 |
| F6 | 索引缓存 | 索引持久化到磁盘，重启后秒级加载 |
| F7 | 搜索结果排序 | 支持按名称、大小、修改时间排序 |

### 非功能需求

| 编号 | 需求 | 指标 |
|------|------|------|
| NF1 | 搜索响应时间 | 100万条索引中搜索 < 50ms |
| NF2 | 内存占用 | 100万文件索引 < 500MB |
| NF3 | 安装包体积 | < 15MB |
| NF4 | 支持系统 | Windows 10/11 (NTFS) |

---

## 阶段一：项目脚手架

### 任务 1.1 — 初始化 Tauri 2.x 项目

**目标**：创建可编译运行的空白 Tauri 项目

**操作步骤**：
1. 执行 `cargo create-tauri-app` 或手动初始化
2. 项目结构：
   ```
   e:\本机快搜\
   ├── src-tauri/
   │   ├── Cargo.toml
   │   ├── tauri.conf.json
   │   ├── src/
   │   │   └── main.rs
   │   └── icons/
   ├── src/
   │   ├── index.html
   │   ├── main.js
   │   └── styles.css
   ├── package.json
   └── TASK_MASTER.md
   ```
3. 确保 `cargo build` 和 `npm run dev`（或等效命令）均无报错
4. 运行程序，能看到空白窗口

**自检项**：
- [ ] `cargo build` 编译成功，无 error
- [ ] 程序启动后能显示窗口
- [ ] 窗口中能看到 "Hello World" 或等效占位文本
- [ ] `src-tauri/tauri.conf.json` 中 `productName` 为 `本机快搜`

---

### 任务 1.2 — 配置 Cargo 依赖

**目标**：引入所有必需的 Rust crate

**操作步骤**：
在 `src-tauri/Cargo.toml` 中添加：
```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-build = { version = "2", features = [] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rusqlite = { version = "0.31", features = ["bundled"] }
notify = "6"
glob = "0.3"
rayon = "1"
chrono = { version = "0.4", features = ["serde"] }
log = "0.4"
env_logger = "0.11"

[target.'cfg(windows)'.dependencies]
windows = { version = "0.58", features = [
    "Win32_Storage_FileSystem",
    "Win32_Foundation",
    "Win32_System_IO",
    "Win32_System_Ioctl",
    "Win32_Security",
] }
```

**自检项**：
- [ ] `cargo build` 成功，所有依赖下载并编译无报错
- [ ] 无依赖版本冲突警告

---

## 阶段二：核心数据模型

### 任务 2.1 — 定义文件条目数据结构

**目标**：定义索引中每个文件的核心数据模型

**操作步骤**：
创建 `src-tauri/src/models.rs`：
```rust
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
```

**自检项**：
- [ ] `cargo build` 成功
- [ ] `FileEntry` 实现了 `Serialize` + `Deserialize`
- [ ] 包含 `name_lower` 预计算字段

---

### 任务 2.2 — 定义配置数据结构

**目标**：定义屏蔽规则和全局配置的数据模型

**操作步骤**：
在 `src-tauri/src/models.rs` 中追加：
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub excluded_paths: Vec<String>,       // 排除的目录路径
    pub excluded_file_patterns: Vec<String>, // 排除的文件名模式（glob）
    pub excluded_extensions: Vec<String>,    // 排除的扩展名
    pub scan_drives: Vec<String>,           // 要扫描的盘符
    pub max_results: usize,                 // 最大返回结果数
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            excluded_paths: vec![
                "C:\\Windows".into(),
                "C:\\$Recycle.Bin".into(),
                "C:\\System Volume Information".into(),
            ],
            excluded_file_patterns: vec![
                "Thumbs.db".into(),
                "desktop.ini".into(),
            ],
            excluded_extensions: vec![],
            scan_drives: vec![],  // 空表示扫描所有 NTFS 盘
            max_results: 1000,
        }
    }
}
```

**自检项**：
- [ ] `cargo build` 成功
- [ ] `AppConfig::default()` 返回合理的默认值
- [ ] 默认排除 `C:\Windows` 等系统目录

---

### 任务 2.3 — 定义搜索请求/响应结构

**目标**：定义前后端 IPC 通信的数据结构

**操作步骤**：
在 `src-tauri/src/models.rs` 中追加：
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,           // 原始搜索输入
    pub sort_by: SortField,      // 排序字段
    pub sort_asc: bool,          // 是否升序
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
}
```

**自检项**：
- [ ] `cargo build` 成功
- [ ] 所有结构体都实现了 `Serialize` + `Deserialize`
- [ ] `SearchRequest` 包含分页参数

---

## 阶段三：MFT 扫描模块

### 任务 3.1 — 枚举 NTFS 磁盘卷

**目标**：列出系统中所有 NTFS 卷

**操作步骤**：
创建 `src-tauri/src/indexer/mod.rs` 和 `src-tauri/src/indexer/mft_scanner.rs`。

使用 Windows API `GetLogicalDrives` + `GetVolumeInformationW` 枚举 NTFS 卷：
```rust
// 伪代码结构
pub fn enumerate_ntfs_volumes() -> Vec<VolumeInfo> {
    // 1. 遍历 A-Z 盘符
    // 2. 检查驱动器是否就绪
    // 3. 获取文件系统类型，过滤出 NTFS
    // 4. 返回 VolumeInfo { drive_letter, volume_name, total_size, is_ntfs }
}
```

通过 `windows` crate 调用 Win32 API。

**自检项**：
- [ ] `cargo build` 成功
- [ ] 编写单元测试 `test_enumerate_volumes`，在当前系统上至少检测到一个 NTFS 卷
- [ ] 返回结果包含盘符和卷标

---

### 任务 3.2 — MFT 快速枚举文件

**目标**：通过 NTFS MFT 快速枚举指定卷上的所有文件

**操作步骤**：

方案选择（⚠️ 需确认）：
- **方案 A**：使用 `usn-journal-rs` crate（封装好但可能不够灵活）
- **方案 B**：直接调用 `FSCTL_ENUM_USN_DATA` / `FSCTL_QUERY_USN_JOURNAL` Win32 API（更底层但完全可控）
- **方案 C（推荐 MVP）**：先用 `walkdir` 做快速遍历实现功能，后续再替换为 MFT 读取

MVP 阶段建议先用方案 C，因为 `walkdir` 无需管理员权限，开发调试更方便。后续任务 3.4 再切换到 MFT。

```rust
// 方案 C 实现（MVP）
use walkdir::WalkDir;

pub fn scan_volume_fast(root: &str, excludes: &[String]) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut id_counter: u64 = 0;
    
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| !is_excluded(e.path(), excludes))
        .filter_map(|e| e.ok())
    {
        let metadata = entry.metadata().ok();
        let file_entry = FileEntry {
            id: id_counter,
            name: entry.file_name().to_string_lossy().to_string(),
            name_lower: entry.file_name().to_string_lossy().to_lowercase(),
            path: entry.path().to_string_lossy().to_string(),
            parent_path: entry.path().parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            extension: entry.path().extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
            size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
            modified: metadata.as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            is_dir: entry.file_type().is_dir(),
        };
        entries.push(file_entry);
        id_counter += 1;
    }
    entries
}
```

需要在 Cargo.toml 中添加 `walkdir = "2"` 依赖。

**自检项**：
- [ ] `cargo build` 成功
- [ ] 对 `C:\` 或某个子目录执行扫描，能返回文件列表
- [ ] 排除路径生效（如排除 `C:\Windows` 后结果中不包含该目录下的文件）
- [ ] 编写单元测试 `test_scan_subdirectory`，扫描一个已知目录并验证结果

---

### 任务 3.3 — 并行扫描多卷

**目标**：使用 rayon 并行扫描多个磁盘卷

**操作步骤**：
```rust
use rayon::prelude::*;

pub fn scan_all_volumes(volumes: &[VolumeInfo], excludes: &[String]) -> Vec<FileEntry> {
    volumes.par_iter()
        .flat_map(|vol| scan_volume_fast(&vol.drive_letter, excludes))
        .collect()
}
```

**自检项**：
- [ ] `cargo build` 成功
- [ ] 多盘并行扫描比单盘顺序扫描快（对比耗时）
- [ ] 结果中文件 ID 全局唯一（不冲突）

---

### 任务 3.4 — 切换到 MFT 直接读取（增强任务）

**目标**：将 walkdir 替换为 MFT 直接读取，大幅提升扫描速度

**操作步骤**：
1. 使用 `FSCTL_ENUM_USN_DATA` API 枚举 MFT 中的所有文件记录
2. 需要管理员权限运行
3. 如果权限不足，回退到 walkdir 方案
4. 保持 `scan_volume_fast` 函数签名不变，内部实现替换

```rust
// 权限检测
pub fn has_admin_privilege() -> bool {
    // 检查当前进程是否以管理员身份运行
}

// MFT 枚举（需要管理员权限）
pub fn scan_volume_mft(root: &str, excludes: &[String]) -> Result<Vec<FileEntry>, String> {
    // 1. 打开卷设备句柄 \\.\C:
    // 2. 调用 FSCTL_ENUM_USN_DATA 枚举所有 USN 记录
    // 3. 解析为 FileEntry
}

// 自动选择策略
pub fn scan_volume_smart(root: &str, excludes: &[String]) -> Vec<FileEntry> {
    if has_admin_privilege() {
        scan_volume_mft(root, excludes)
            .unwrap_or_else(|_| scan_volume_fast(root, excludes))
    } else {
        scan_volume_fast(root, excludes)
    }
}
```

**自检项**：
- [ ] `cargo build` 成功
- [ ] 管理员模式下 MFT 扫描速度明显快于 walkdir（对比耗时）
- [ ] 非管理员模式自动回退到 walkdir，不崩溃
- [ ] 两种模式返回的结果基本一致（允许少量差异如系统文件）

---

## 阶段四：索引缓存

### 任务 4.1 — SQLite 索引持久化

**目标**：将扫描结果存入 SQLite，重启后可快速加载

**操作步骤**：
创建 `src-tauri/src/indexer/cache.rs`：
```rust
use rusqlite::Connection;

pub struct IndexCache {
    conn: Connection,
}

impl IndexCache {
    pub fn open(path: &str) -> Result<Self, String> { ... }
    pub fn create_table(&self) -> Result<(), String> { ... }
    pub fn save_entries(&self, entries: &[FileEntry]) -> Result<(), String> { ... }
    pub fn load_all(&self) -> Result<Vec<FileEntry>, String> { ... }
    pub fn clear(&self) -> Result<(), String> { ... }
}
```

SQL 表结构：
```sql
CREATE TABLE IF NOT EXISTS file_index (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    name_lower TEXT NOT NULL,
    path TEXT NOT NULL,
    parent_path TEXT NOT NULL,
    extension TEXT NOT NULL,
    size INTEGER NOT NULL,
    modified INTEGER NOT NULL,
    is_dir INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_name_lower ON file_index(name_lower);
CREATE INDEX IF NOT EXISTS idx_extension ON file_index(extension);
```

**自检项**：
- [ ] `cargo build` 成功
- [ ] 扫描后将数据写入 SQLite 文件
- [ ] 从 SQLite 加载数据后，结果与原始扫描一致
- [ ] 加载速度 < 3秒（100万条）

---

### 任务 4.2 — 启动流程：缓存优先

**目标**：程序启动时优先从缓存加载，后台异步刷新

**操作步骤**：
```
启动 → 检查缓存文件是否存在且 < 24小时
  ├── 是 → 加载缓存 → 显示结果 → 后台异步重新扫描
  └── 否 → 全量扫描 → 写入缓存 → 显示结果
```

**自检项**：
- [ ] 首次启动执行全量扫描并生成缓存
- [ ] 第二次启动从缓存加载，速度明显更快
- [ ] 缓存过期（>24h）后自动触发重新扫描

---

## 阶段五：搜索引擎

### 任务 5.1 — 多段关键词搜索核心

**目标**：实现空格分隔的多关键词 AND 搜索

**操作步骤**：
创建 `src-tauri/src/search/mod.rs` 和 `src-tauri/src/search/engine.rs`：

```rust
pub struct SearchEngine {
    index: Vec<FileEntry>,
}

impl SearchEngine {
    pub fn search(&self, request: &SearchRequest) -> SearchResponse {
        let keywords: Vec<&str> = request.query.split_whitespace().collect();
        
        // 分离扩展名条件（以 . 开头）和文件名条件
        let (ext_keywords, name_keywords): (Vec<_>, Vec<_>) = keywords.iter()
            .partition(|k| k.starts_with('.') && k.len() > 1);
        
        let start = std::time::Instant::now();
        
        let mut results: Vec<&FileEntry> = self.index.iter()
            .filter(|entry| {
                // 屏蔽规则过滤（任务 6.1 实现）
                !self.is_excluded(entry)
            })
            .filter(|entry| {
                // 文件名必须包含所有关键词（AND 逻辑）
                name_keywords.iter().all(|kw| {
                    entry.name_lower.contains(&kw.to_lowercase())
                })
            })
            .filter(|entry| {
                // 扩展名模糊匹配
                if ext_keywords.is_empty() { return true; }
                ext_keywords.iter().any(|ext| {
                    let ext_clean = &ext[1..]; // 去掉 .
                    entry.extension.starts_with(&ext_clean.to_lowercase())
                })
            })
            .collect();
        
        // 排序
        self.sort_results(&mut results, &request.sort_by, request.sort_asc);
        
        let total = results.len();
        let paged = results.into_iter()
            .skip(request.offset)
            .take(request.limit)
            .cloned()
            .collect();
        
        SearchResponse {
            results: paged,
            total,
            query_time_ms: start.elapsed().as_millis() as u64,
            index_count: self.index.len(),
        }
    }
}
```

**自检项**：
- [ ] `cargo build` 成功
- [ ] 测试用例：搜索 `福建 交流` 返回的文件名同时包含两个词
- [ ] 测试用例：搜索 `report .pdf` 返回文件名含 report 且扩展名为 pdf 的文件
- [ ] 测试用例：搜索 `.jp` 匹配到 `.jpg` 和 `.jpeg` 文件
- [ ] 搜索空字符串返回所有文件
- [ ] 100万条索引搜索耗时 < 50ms

---

### 任务 5.2 — 搜索结果排序

**目标**：支持按名称、大小、修改时间排序

**操作步骤**：
在 `SearchEngine` 中实现 `sort_results` 方法：
```rust
fn sort_results(&self, results: &mut Vec<&FileEntry>, sort_by: &SortField, asc: bool) {
    results.sort_by(|a, b| {
        let cmp = match sort_by {
            SortField::Name => a.name_lower.cmp(&b.name_lower),
            SortField::Size => a.size.cmp(&b.size),
            SortField::Modified => a.modified.cmp(&b.modified),
            SortField::Path => a.path.cmp(&b.path),
        };
        if asc { cmp } else { cmp.reverse() }
    });
}
```

**自检项**：
- [ ] 按名称升序/降序排列正确
- [ ] 按大小排序正确（大文件在前/后）
- [ ] 按修改时间排序正确
- [ ] 默认排序为名称升序

---

### 任务 5.3 — 搜索性能优化

**目标**：优化搜索性能，确保大规模索引下 < 50ms

**操作步骤**：
1. 使用预计算的 `name_lower` 避免搜索时转小写
2. 使用 `rayon` 并行过滤（索引量 > 50 万时启用）
3. 提前终止：收集到 `max_results` 条后停止

```rust
// 并行过滤（大规模索引时）
if self.index.len() > 500_000 {
    use rayon::prelude::*;
    results = self.index.par_iter()
        .filter(|entry| { /* 过滤逻辑 */ })
        .collect();
} else {
    results = self.index.iter()
        .filter(|entry| { /* 过滤逻辑 */ })
        .collect();
}
```

**自检项**：
- [ ] 100万条索引搜索 < 50ms
- [ ] 并行/串行结果一致
- [ ] 内存占用无明显增长

---

## 阶段六：屏蔽规则

### 任务 6.1 — 屏蔽规则引擎

**目标**：实现路径排除和文件模式排除

**操作步骤**：
创建 `src-tauri/src/search/exclude.rs`：
```rust
use glob::Pattern;

pub struct ExcludeFilter {
    path_prefixes: Vec<String>,
    file_patterns: Vec<Pattern>,
    extensions: Vec<String>,
}

impl ExcludeFilter {
    pub fn from_config(config: &AppConfig) -> Self { ... }
    
    pub fn is_excluded(&self, entry: &FileEntry) -> bool {
        // 1. 路径前缀匹配
        if self.path_prefixes.iter().any(|p| {
            entry.parent_path.starts_with(p)
        }) { return true; }
        
        // 2. 文件名 glob 匹配
        if self.file_patterns.iter().any(|p| p.matches(&entry.name)) {
            return true;
        }
        
        // 3. 扩展名匹配
        if self.extensions.iter().any(|e| entry.extension == *e) {
            return true;
        }
        
        false
    }
}
```

**自检项**：
- [ ] `cargo build` 成功
- [ ] 排除 `C:\Windows` 后，该目录下所有文件不出现在搜索结果中
- [ ] 排除 `*.tmp` 后，所有 .tmp 文件被过滤
- [ ] 排除扩展名 `log` 后，所有 .log 文件被过滤
- [ ] 多条规则同时生效（AND 组合）

---

### 任务 6.2 — 屏蔽规则配置持久化

**目标**：屏蔽规则保存到配置文件，重启后保留

**操作步骤**：
创建 `src-tauri/src/config/settings.rs`：
```rust
const CONFIG_FILE: &str = "config.json";

pub fn load_config() -> AppConfig {
    let path = get_config_path();
    if path.exists() {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        AppConfig::default()
    }
}

pub fn save_config(config: &AppConfig) -> Result<(), String> {
    let path = get_config_path();
    let content = serde_json::to_string_pretty(config)
        .map_err(|e| e.to_string())?;
    std::fs::write(&path, content)
        .map_err(|e| e.to_string())
}

fn get_config_path() -> std::path::PathBuf {
    let app_dir = dirs::data_local_dir()
        .unwrap_or_default()
        .join("本机快搜");
    std::fs::create_dir_all(&app_dir).ok();
    app_dir.join(CONFIG_FILE)
}
```

需要在 Cargo.toml 添加 `dirs = "5"` 依赖。

**自检项**：
- [ ] 保存配置后，配置文件存在于磁盘
- [ ] 重启程序后加载的配置与保存的一致
- [ ] 配置文件格式为可读的 JSON

---

## 阶段七：实时更新

### 任务 7.1 — 文件系统监控

**目标**：监控文件系统变更，实时更新索引

**操作步骤**：
创建 `src-tauri/src/indexer/watcher.rs`：
```rust
use notify::{Watcher, RecursiveMode, Event, EventKind};
use std::sync::{Arc, Mutex};

pub struct FileWatcher {
    _watcher: notify::RecommendedWatcher,
}

impl FileWatcher {
    pub fn start(
        paths: &[String],
        index: Arc<Mutex<Vec<FileEntry>>>,
    ) -> Result<Self, String> {
        let watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
            if let Ok(event) = res {
                match event.kind {
                    EventKind::Create(_) => { /* 添加新文件到索引 */ }
                    EventKind::Remove(_) => { /* 从索引中移除 */ }
                    EventKind::Modify(_) => { /* 更新索引中的条目 */ }
                    _ => {}
                }
            }
        }).map_err(|e| e.to_string())?;
        
        // 监控各卷根目录
        for path in paths {
            watcher.watch(path.as_ref(), RecursiveMode::Recursive)
                .map_err(|e| e.to_string())?;
        }
        
        Ok(Self { _watcher: watcher })
    }
}
```

**自检项**：
- [ ] 启动监控后，新建文件能在 2 秒内出现在搜索结果中
- [ ] 删除文件后能从搜索结果中消失
- [ ] 重命名文件后搜索结果更新
- [ ] 监控不会导致 CPU 持续高占用

---

## 阶段八：IPC 命令层

### 任务 8.1 — 实现 Tauri 命令

**目标**：暴露 Rust 后端功能给前端调用

**操作步骤**：
创建 `src-tauri/src/commands/mod.rs`、`search.rs`、`settings.rs`：

```rust
// commands/search.rs
#[tauri::command]
pub fn search(request: SearchRequest) -> SearchResponse { ... }

#[tauri::command]
pub fn get_index_status() -> IndexStatus { ... }

// commands/settings.rs
#[tauri::command]
pub fn get_config() -> AppConfig { ... }

#[tauri::command]
pub fn save_settings(config: AppConfig) -> bool { ... }

#[tauri::command]
pub fn add_exclude_path(path: String) -> bool { ... }

#[tauri::command]
pub fn remove_exclude_path(path: String) -> bool { ... }

#[tauri::command]
pub fn rescan() -> bool { ... }
```

在 `main.rs` 中注册命令：
```rust
tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
        search,
        get_index_status,
        get_config,
        save_settings,
        add_exclude_path,
        remove_exclude_path,
        rescan,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
```

**自检项**：
- [ ] `cargo build` 成功
- [ ] 前端能通过 `invoke('search', {...})` 调用搜索并返回结果
- [ ] 前端能读写配置
- [ ] 所有命令都有正确的返回值

---

## 阶段九：前端 UI

### 任务 9.1 — 搜索界面

**目标**：实现主搜索界面

**UI 设计稿**：
```
┌──────────────────────────────────────────────────────┐
│  🔍 [______________搜索框________________] [⚙设置]  │
├──────────────────────────────────────────────────────┤
│  状态栏: 已索引 1,234,567 个文件 | 搜索耗时 12ms     │
├──────────────────────────────────────────────────────┤
│  排序: [名称 ▼] [大小] [修改时间]                    │
├──────────────────────────────────────────────────────┤
│  📁 文件名              │ 路径          │ 大小      │
│  ───────────────────────┼───────────────┼────────── │
│  📄 福建交流报告.docx   │ D:\Documents  │ 2.3 MB   │
│  📄 福建交流纪要.pdf    │ E:\Work       │ 856 KB   │
│  📁 福建交流资料        │ F:\Archive    │ <目录>   │
│  ...                                             │
├──────────────────────────────────────────────────────┤
│  共 42 条结果 | 第 1/1 页                            │
└──────────────────────────────────────────────────────┘
```

**实现要求**：
1. 搜索框：输入后 300ms 防抖自动搜索
2. 结果列表：虚拟滚动（文件数可能很多）
3. 支持键盘操作：↑↓ 选择，Enter 打开文件所在目录
4. 右键菜单：打开文件、打开所在目录、复制路径

**自检项**：
- [ ] 搜索框输入后自动触发搜索（防抖 300ms）
- [ ] 结果列表正确显示文件名、路径、大小
- [ ] 排序按钮可切换排序方式
- [ ] 状态栏显示索引数量和搜索耗时
- [ ] 键盘 ↑↓ 可选择条目
- [ ] Enter 键打开文件所在目录
- [ ] 右键菜单功能正常

---

### 任务 9.2 — 设置面板

**目标**：实现屏蔽规则管理界面

**UI 设计稿**：
```
┌──────────────────────────────────────────┐
│  ⚙ 设置                            [×]  │
├──────────────────────────────────────────┤
│                                          │
│  📂 屏蔽目录                             │
│  ┌────────────────────────────────────┐  │
│  │ C:\Windows                    [删除]│  │
│  │ C:\$Recycle.Bin               [删除]│  │
│  └────────────────────────────────────┘  │
│  [+ 添加目录]                            │
│                                          │
│  📄 屏蔽文件模式                         │
│  ┌────────────────────────────────────┐  │
│  │ *.tmp                         [删除]│  │
│  │ Thumbs.db                     [删除]│  │
│  └────────────────────────────────────┘  │
│  [+ 添加模式]                            │
│                                          │
│  🏷 屏蔽扩展名                           │
│  ┌────────────────────────────────────┐  │
│  │ log                           [删除]│  │
│  └────────────────────────────────────┘  │
│  [+ 添加扩展名]                          │
│                                          │
│  💾 磁盘扫描                             │
│  ☑ C:  ☑ D:  ☑ E:  ☐ F:               │
│                                          │
│        [保存]  [重新扫描]                │
└──────────────────────────────────────────┘
```

**自检项**：
- [ ] 能添加/删除屏蔽目录
- [ ] 能添加/删除屏蔽文件模式
- [ ] 能添加/删除屏蔽扩展名
- [ ] 能选择/取消选择扫描磁盘
- [ ] 保存后配置持久化
- [ ] 重新扫描按钮触发后台扫描

---

### 任务 9.3 — 索引进度显示

**目标**：扫描过程中显示进度

**UI 设计稿**：
```
┌──────────────────────────────────────────┐
│  📡 正在索引文件...                       │
│  ████████████████░░░░░░░░  67%           │
│  已扫描: 834,521 / 1,245,678 个文件      │
│  当前: D:\Documents\项目资料              │
└──────────────────────────────────────────┘
```

**自检项**：
- [ ] 扫描时显示进度条和百分比
- [ ] 数字实时更新
- [ ] 扫描完成后自动切换到搜索界面

---

## 阶段十：集成与测试

### 任务 10.1 — 端到端集成

**目标**：将所有模块串联，完整流程可运行

**操作步骤**：
1. 启动流程：
   - 加载配置 → 加载缓存（或全量扫描）→ 启动文件监控 → 显示 UI
2. 搜索流程：
   - 输入关键词 → IPC 调用搜索 → 返回结果 → 渲染列表
3. 设置流程：
   - 打开设置 → 修改规则 → 保存 → 重新过滤索引

**自检项**：
- [ ] 完整启动流程无报错
- [ ] 搜索 → 返回结果 → 点击打开文件，全链路通
- [ ] 修改屏蔽规则后搜索结果立即更新
- [ ] 关闭程序后重启，缓存加载正常

---

### 任务 10.2 — 性能基准测试

**目标**：验证性能指标达标

**测试用例**：
| 测试项 | 指标 | 通过条件 |
|--------|------|----------|
| 全量扫描 100万文件 | < 10秒（MFT）/ < 30秒（walkdir） | 耗时达标 |
| 缓存加载 100万条 | < 3秒 | 耗时达标 |
| 搜索响应（100万条） | < 50ms | 耗时达标 |
| 内存占用（100万条） | < 500MB | 任务管理器查看 |
| 安装包体积 | < 15MB | 查看安装包大小 |

**自检项**：
- [ ] 所有指标达标
- [ ] 如有未达标项，记录并优化

---

### 任务 10.3 — 边界情况测试

**目标**：验证各种边界情况不崩溃

**测试用例**：
| 场景 | 预期行为 |
|------|----------|
| 搜索框输入特殊字符（`< > " / \`） | 不崩溃，返回空或正常匹配 |
| 搜索框输入超长字符串（1000字符） | 不崩溃，正常处理 |
| 磁盘未就绪（如光驱无盘） | 跳过该盘，不报错 |
| 路径含中文/emoji | 正确显示 |
| 屏蔽所有目录后搜索 | 返回空结果，不崩溃 |
| 搜索过程中磁盘被拔出 | 不崩溃，优雅处理 |

**自检项**：
- [ ] 所有场景测试通过，无崩溃

---

## 阶段十一：打包发布

### 任务 11.1 — NSIS 安装包

**目标**：生成 Windows 安装包

**操作步骤**：
1. 配置 `tauri.conf.json` 中的 bundle 设置
2. 使用 Tauri 内置的 NSIS 打包
3. 配置安装包图标、名称、版本

**自检项**：
- [ ] `cargo tauri build` 成功生成安装包
- [ ] 安装包可正常安装
- [ ] 安装后程序可正常启动运行
- [ ] 安装包体积 < 15MB

---

### 任务 11.2 — 最终验收

**目标**：完整验收所有功能

**验收清单**：
- [ ] F1: 快速文件索引正常
- [ ] F2: 多段关键词搜索正常（核心需求）
- [ ] F3: 扩展名模糊搜索正常（核心需求）
- [ ] F4: 屏蔽路径设置正常（核心需求）
- [ ] F5: 实时更新正常
- [ ] F6: 索引缓存正常
- [ ] F7: 搜索结果排序正常
- [ ] NF1-NF4: 性能指标全部达标

---

## 任务依赖关系

```
1.1 → 1.2 → 2.1 → 2.2 → 2.3
                        ↓
                   3.1 → 3.2 → 3.3 → 3.4
                    ↓
                   4.1 → 4.2
                    ↓
              5.1 → 5.2 → 5.3
               ↓
              6.1 → 6.2
               ↓
              7.1
               ↓
              8.1
               ↓
         9.1 → 9.2 → 9.3
               ↓
         10.1 → 10.2 → 10.3
               ↓
         11.1 → 11.2
```

---

## 预估工时

| 阶段 | 预估时间 |
|------|----------|
| 阶段一：脚手架 | 30 分钟 |
| 阶段二：数据模型 | 20 分钟 |
| 阶段三：MFT 扫描 | 1-2 小时 |
| 阶段四：索引缓存 | 30 分钟 |
| 阶段五：搜索引擎 | 1-2 小时 |
| 阶段六：屏蔽规则 | 30 分钟 |
| 阶段七：实时更新 | 1 小时 |
| 阶段八：IPC 命令 | 30 分钟 |
| 阶段九：前端 UI | 2-3 小时 |
| 阶段十：集成测试 | 1-2 小时 |
| 阶段十一：打包发布 | 30 分钟 |
| **合计** | **约 8-13 小时** |

---

## 最终交付物

1. 可安装的 Windows 应用程序（NSIS 安装包）
2. 完整源代码（Tauri 2.x + Rust + HTML/CSS/JS）
3. 开发日志 `dev_log.md`
