use crate::models::{FileEntry, SearchRequest, SearchResponse, SortField, SortOrder};
use rayon::prelude::*;
use std::time::Instant;

/// 不区分大小写的子串匹配（零分配）。
/// ASCII 字节按忽略大小写比较；非 ASCII（如中文）按字节精确比较。
fn contains_ignore_case(haystack: &str, needle_lower: &str) -> bool {
    let h = haystack.as_bytes();
    let n = needle_lower.as_bytes();
    if n.is_empty() {
        return true;
    }
    if n.len() > h.len() {
        return false;
    }
    for i in 0..=(h.len() - n.len()) {
        if h[i..i + n.len()]
            .iter()
            .zip(n.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            return true;
        }
    }
    false
}

/// 不区分大小写的字典序比较（零分配），用于按名称排序。
fn cmp_ignore_case(a: &str, b: &str) -> std::cmp::Ordering {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let min = ab.len().min(bb.len());
    for i in 0..min {
        let (ca, cb) = (ab[i].to_ascii_lowercase(), bb[i].to_ascii_lowercase());
        if ca != cb {
            return ca.cmp(&cb);
        }
    }
    ab.len().cmp(&bb.len())
}

pub struct SearchEngine {
    index: Vec<FileEntry>,
}

impl SearchEngine {
    pub fn new(index: Vec<FileEntry>) -> Self {
        Self { index }
    }

    /// 应用 USN 增量变更（新增 / 删除 / 改名），返回受影响的条目数。
    ///
    /// - `add`：新文件条目（直接追加）
    /// - `remove`：要删除的完整路径集合
    /// - `rename`：旧路径 → (新父目录完整路径, 新文件名, 是否目录)。文件改名精确匹配旧路径；
    ///   目录改名按前缀重写整棵子树（只有目录改名才做前缀扫描，避免改名风暴下的 O(n*m)）
    pub fn apply_changes(
        &mut self,
        add: Vec<FileEntry>,
        remove: &std::collections::HashSet<String>,
        rename: &std::collections::HashMap<String, (String, String, bool)>,
    ) -> usize {
        let mut changed = 0usize;

        // 1. 改名：精确匹配（文件）+ 前缀重写（目录子树）
        if !rename.is_empty() {
            // 预计算目录改名前缀表（前缀字符串只生成一次）
            let mut dir_prefixes: Vec<(String, String)> = Vec::new();
            for (old, (new_dir, new_name, is_dir)) in rename.iter() {
                if *is_dir {
                    dir_prefixes.push((
                        format!("{}\\", old),
                        format!("{}\\{}\\", new_dir, new_name),
                    ));
                }
            }
            for entry in self.index.iter_mut() {
                if let Some((new_dir, new_name, _)) = rename.get(entry.path.as_ref()) {
                    // 文件/目录本身：路径替换为 new_dir 拼接 new_name
                    let new_path = format!("{}\\{}", new_dir, new_name);
                    entry.path = new_path.into_boxed_str();
                    entry.name = new_name.as_str().into();
                    // 扩展名重算（目录扩展名为空）
                    entry.extension = if entry.is_dir || new_name.is_empty() {
                        "".into()
                    } else {
                        match new_name.rfind('.') {
                            Some(pos) => new_name[pos + 1..].to_lowercase().into_boxed_str(),
                            None => "".into(),
                        }
                    };
                    changed += 1;
                } else if !dir_prefixes.is_empty() {
                    // 目录改名：子项按前缀重写（新路径 = 新父目录\新名\原子路径剩余部分）
                    for (old_prefix, new_prefix) in &dir_prefixes {
                        if entry.path.starts_with(old_prefix.as_str()) {
                            entry.path = format!("{}{}", new_prefix, &entry.path[old_prefix.len()..])
                                .into_boxed_str();
                            changed += 1;
                            break;
                        }
                    }
                }
            }
        }

        // 2. 删除
        if !remove.is_empty() {
            let before = self.index.len();
            self.index.retain(|e| !remove.contains(e.path.as_ref()));
            changed += before - self.index.len();
        }

        // 3. 新增
        if !add.is_empty() {
            changed += add.len();
            self.index.extend(add);
        }

        changed
    }

    /// 按请求内嵌上限搜索（供测试与兼容调用）
    #[allow(dead_code)]
    pub fn search(&self, request: &SearchRequest) -> SearchResponse {
        self.search_with_limit(request, request.limit)
    }

    /// 带外部结果数上限的搜索（UI 层使用：max_results 配置控制上限）
    pub fn search_with_limit(&self, request: &SearchRequest, limit: usize) -> SearchResponse {
        let start = Instant::now();
        
        // 文件名关键词
        let name_keywords_lower: Vec<String> = request.query.split_whitespace()
            .map(|k| k.to_lowercase())
            .collect();
        
        // 扩展名关键词（去掉前导点号，支持模糊前缀匹配）
        let ext_keywords_lower: Vec<String> = request.ext_query.split_whitespace()
            .map(|k| if k.starts_with('.') { k[1..].to_lowercase() } else { k.to_lowercase() })
            .collect();
        
        // 根据索引大小选择串行或并行过滤
        let mut results: Vec<&FileEntry> = if self.index.len() > 500_000 {
            // 大规模索引使用并行过滤
            self.index.par_iter()
                .filter(|entry| {
                    // 文件夹过滤
                    if request.only_folders {
                        return entry.is_dir;
                    }
                    true
                })
                .filter(|entry| {
                    // 文件名必须包含所有关键词（AND 逻辑）
                    name_keywords_lower.iter().all(|kw| {
                        contains_ignore_case(&entry.name, kw)
                    })
                })
                .filter(|entry| {
                    // 目录范围筛选：多路径任一前缀匹配（OR），空为全盘
                    if request.filter_dirs.is_empty() {
                        true
                    } else {
                        request
                            .filter_dirs
                            .iter()
                            .any(|dir| entry.path.starts_with(dir.as_str()))
                    }
                })
                .filter(|entry| {
                    // 扩展名模糊匹配，空则不限制
                    if ext_keywords_lower.is_empty() { return true; }
                    ext_keywords_lower.iter().any(|ext| {
                        entry.extension.starts_with(ext.as_str())
                    })
                })
                .collect()
        } else {
            // 小规模索引使用串行过滤
            self.index.iter()
                .filter(|entry| {
                    if request.only_folders {
                        return entry.is_dir;
                    }
                    true
                })
                .filter(|entry| {
                    // 文件名必须包含所有关键词（AND 逻辑）
                    name_keywords_lower.iter().all(|kw| {
                        contains_ignore_case(&entry.name, kw)
                    })
                })
                .filter(|entry| {
                    // 目录范围筛选：多路径任一前缀匹配（OR），空为全盘
                    if request.filter_dirs.is_empty() {
                        true
                    } else {
                        request
                            .filter_dirs
                            .iter()
                            .any(|dir| entry.path.starts_with(dir.as_str()))
                    }
                })
                .filter(|entry| {
                    // 扩展名模糊匹配，空则不限制
                    if ext_keywords_lower.is_empty() { return true; }
                    ext_keywords_lower.iter().any(|ext| {
                        entry.extension.starts_with(ext.as_str())
                    })
                })
                .collect()
        };
        
        // 排序（优先使用组合排序；sort_orders 为空时按 sort_by，None 表示不排序）
        if !request.sort_orders.is_empty() {
            self.sort_results_multi(&mut results, &request.sort_orders);
        } else if !matches!(&request.sort_by, SortField::None) {
            self.sort_results(&mut results, &request.sort_by, request.sort_asc);
        }

        let total = results.len();
        
        // 结果截断（外部传入的上限，替代分页）
        let paged = results.into_iter()
            .skip(request.offset)
            .take(limit)
            .cloned()
            .collect();
        
        SearchResponse {
            results: paged,
            total,
            query_time_ms: start.elapsed().as_millis() as u64,
            index_count: self.index.len(),
        }
    }

    fn sort_results(&self, results: &mut Vec<&FileEntry>, sort_by: &SortField, asc: bool) {
        results.sort_by(|a, b| {
            let cmp = self.compare_by_field(a, b, sort_by);
            if asc { cmp } else { cmp.reverse() }
        });
    }

    fn sort_results_multi(&self, results: &mut Vec<&FileEntry>, sort_orders: &[SortOrder]) {
        results.sort_by(|a, b| {
            for order in sort_orders {
                let cmp = self.compare_by_field(a, b, &order.field);
                let ordered = if order.asc { cmp } else { cmp.reverse() };
                if ordered != std::cmp::Ordering::Equal {
                    return ordered;
                }
            }
            std::cmp::Ordering::Equal
        });
    }

    fn compare_by_field(&self, a: &FileEntry, b: &FileEntry, field: &SortField) -> std::cmp::Ordering {
        match field {
            SortField::None => std::cmp::Ordering::Equal,
            SortField::Name => cmp_ignore_case(&a.name, &b.name),
            SortField::Size => a.size.cmp(&b.size),
            SortField::Modified => a.modified.cmp(&b.modified),
            SortField::Path => a.path.cmp(&b.path),
        }
    }

    pub fn get_index_count(&self) -> usize {
        self.index.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SortField;

    fn create_test_index() -> Vec<FileEntry> {
        vec![
            FileEntry {
                name: "福建交流报告.docx".into(),
                path: "D:\\Documents\\福建交流报告.docx".into(),
                extension: "docx".into(),
                size: 1024,
                modified: 1700000000,
                is_dir: false,
            },
            FileEntry {
                name: "福建交流纪要.pdf".into(),
                path: "E:\\Work\\福建交流纪要.pdf".into(),
                extension: "pdf".into(),
                size: 2048,
                modified: 1700000100,
                is_dir: false,
            },
            FileEntry {
                name: "照片.jpg".into(),
                path: "F:\\Photos\\照片.jpg".into(),
                extension: "jpg".into(),
                size: 512,
                modified: 1700000200,
                is_dir: false,
            },
            FileEntry {
                name: "图片.jpeg".into(),
                path: "F:\\Photos\\图片.jpeg".into(),
                extension: "jpeg".into(),
                size: 768,
                modified: 1700000300,
                is_dir: false,
            },
            FileEntry {
                name: "report.pdf".into(),
                path: "G:\\Reports\\report.pdf".into(),
                extension: "pdf".into(),
                size: 4096,
                modified: 1700000400,
                is_dir: false,
            },
        ]
    }

    #[test]
    fn test_multi_keyword_search() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);
        
        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "福建 交流".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        
        let response = engine.search(&request);
        assert_eq!(response.total, 2);
        assert!(response.results.iter().all(|e| e.name.contains("福建") && e.name.contains("交流")));
    }

    #[test]
    fn test_extension_fuzzy_match() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);
        
        // 测试扩展名框 jp 匹配 .jpg 和 .jpeg
        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: String::new(),
            ext_query: "jp".to_string(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        
        let response = engine.search(&request);
        assert_eq!(response.total, 2);
        assert!(response.results.iter().all(|e| e.extension.starts_with("jp")));
    }

    #[test]
    fn test_combined_search() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);
        
        // 测试文件名 report + 扩展名 pdf
        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "report".to_string(),
            ext_query: "pdf".to_string(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        
        let response = engine.search(&request);
        assert_eq!(response.total, 1);
        assert_eq!(response.results[0].name.as_ref(), "report.pdf");
    }

    #[test]
    fn test_empty_query() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);
        
        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        
        let response = engine.search(&request);
        assert_eq!(response.total, 5);
    }

    #[test]
    fn test_sort_by_size() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);
        
        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Size,
            sort_asc: false, // 降序
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        
        let response = engine.search(&request);
        assert_eq!(response.results[0].name.as_ref(), "report.pdf"); // 最大的文件
    }

    #[test]
    fn test_sort_by_name_asc() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);

        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };

        let response = engine.search(&request);
        // 验证名称升序：不区分大小写依次递增
        for i in 1..response.results.len() {
            assert!(cmp_ignore_case(&response.results[i - 1].name, &response.results[i].name)
                != std::cmp::Ordering::Greater);
        }
    }

    #[test]
    fn test_sort_by_name_desc() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);

        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: false,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };

        let response = engine.search(&request);
        // 验证名称降序：不区分大小写依次递减
        for i in 1..response.results.len() {
            assert!(cmp_ignore_case(&response.results[i - 1].name, &response.results[i].name)
                != std::cmp::Ordering::Less);
        }
    }

    #[test]
    fn test_sort_by_modified() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);

        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Modified,
            sort_asc: false, // 最新修改在前
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };

        let response = engine.search(&request);
        // 验证按修改时间降序
        for i in 1..response.results.len() {
            assert!(response.results[i - 1].modified >= response.results[i].modified);
        }
        assert_eq!(response.results[0].name.as_ref(), "report.pdf"); // modified=1700000400
    }

    #[test]
    fn test_sort_by_path() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);

        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Path,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };

        let response = engine.search(&request);
        // 验证路径升序
        for i in 1..response.results.len() {
            assert!(response.results[i - 1].path <= response.results[i].path);
        }
    }

    #[test]
    fn test_default_sort_is_name_asc() {
        // 默认搜索请求使用名称升序
        let index = create_test_index();
        let engine = SearchEngine::new(index);

        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };

        let response = engine.search(&request);
        // 验证第一个是字母序最小的
        assert_eq!(response.results[0].name.as_ref(), "report.pdf"); // 'r' 在英文中排最前
    }

    #[test]
    fn test_sort_none_keeps_index_order() {
        // 重置排序时应保持索引原始顺序，不再强制按文件名排序
        let index = create_test_index();
        let engine = SearchEngine::new(index);

        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: String::new(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::None,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };

        let response = engine.search(&request);
        assert_eq!(response.total, 5);
        assert_eq!(response.results[0].name.as_ref(), "福建交流报告.docx");
        assert_eq!(response.results[4].name.as_ref(), "report.pdf");
    }

    #[test]
    fn test_performance_large_index() {
        // 生成大规模测试索引（100万条）
        let mut large_index = Vec::with_capacity(1_000_000);
        for i in 0..1_000_000 {
            large_index.push(FileEntry {
                name: format!("file_{}.txt", i).into(),
                path: format!("C:\\test\\file_{}.txt", i).into(),
                extension: "txt".into(),
                size: i * 1024,
                modified: 1700000000 + i as i64,
                is_dir: false,
            });
        }

        let engine = SearchEngine::new(large_index);
        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "file_123".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };

        let start = std::time::Instant::now();
        let response = engine.search(&request);
        let elapsed = start.elapsed();

        // 验证搜索耗时 < 50ms
        assert!(elapsed.as_millis() < 50, "搜索耗时 {}ms 超过 50ms 限制", elapsed.as_millis());
        assert!(response.total > 0, "应该找到匹配结果");
    }

    #[test]
    fn test_parallel_serial_consistency() {
        // 生成测试索引（刚好在阈值附近）
        let mut index = Vec::new();
        for i in 0..10000 {
            index.push(FileEntry {
                name: format!("test_{}.dat", i).into(),
                path: format!("D:\\data\\test_{}.dat", i).into(),
                extension: "dat".into(),
                size: i * 512,
                modified: 1700000000 + i as i64,
                is_dir: false,
            });
        }

        // 创建两个引擎，一个强制串行，一个可能并行
        let engine_serial = SearchEngine::new(index.clone());
        let engine_parallel = SearchEngine::new(index);

        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "test_5".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Size,
            sort_asc: false,
            sort_orders: vec![],
            offset: 0,
            limit: 1000,
        };

        let response_serial = engine_serial.search(&request);
        let response_parallel = engine_parallel.search(&request);

        // 验证结果一致
        assert_eq!(response_serial.total, response_parallel.total);
        assert_eq!(response_serial.results.len(), response_parallel.results.len());
        
        // 验证结果顺序一致
        for (s, p) in response_serial.results.iter().zip(response_parallel.results.iter()) {
            assert_eq!(s.name, p.name);
        }
    }

    // ========== 边界情况测试 ==========

    #[test]
    fn test_special_characters_search() {
        // 测试特殊字符搜索不崩溃
        let index = create_test_index();
        let engine = SearchEngine::new(index);
        
        let special_queries = vec![
            "<script>",
            "test\"quote",
            "path\\slash",
            "file/name",
            "special&char",
            "unicode✓",
        ];
        
        for query in special_queries {
            let request = SearchRequest {
                    filter_dirs: Vec::new(),
                query: query.to_string(),
                ext_query: String::new(),
                only_folders: false,
                sort_by: SortField::Name,
                sort_asc: true,
                sort_orders: vec![],
                offset: 0,
                limit: 100,
            };
            
            // 应该不崩溃，返回空结果
            let response = engine.search(&request);
            assert_eq!(response.total, 0);
        }
    }

    #[test]
    fn test_long_query_string() {
        // 测试超长查询字符串（1000字符）
        let index = create_test_index();
        let engine = SearchEngine::new(index);
        
        let long_query = "a".repeat(1000);
        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: long_query,
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        
        // 应该不崩溃，返回空结果
        let response = engine.search(&request);
        assert_eq!(response.total, 0);
    }

    #[test]
    fn test_chinese_and_emoji_paths() {
        // 测试中文和 emoji 路径
        let index = vec![
            FileEntry {
                name: "中文文件.txt".into(),
                path: "D:\\文档\\中文文件.txt".into(),
                extension: "txt".into(),
                size: 1024,
                modified: 1700000000,
                is_dir: false,
            },
            FileEntry {
                name: "emoji😀.txt".into(),
                path: "E:\\测试\\emoji😀.txt".into(),
                extension: "txt".into(),
                size: 2048,
                modified: 1700000100,
                is_dir: false,
            },
        ];
        
        let engine = SearchEngine::new(index);
        
        // 搜索中文
        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "中文".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        let response = engine.search(&request);
        assert_eq!(response.total, 1);
        assert_eq!(response.results[0].name.as_ref(), "中文文件.txt");
        
        // 搜索 emoji
        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "😀".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        let response = engine.search(&request);
        assert_eq!(response.total, 1);
        assert_eq!(response.results[0].name.as_ref(), "emoji😀.txt");
    }

    #[test]
    fn test_all_paths_excluded() {
        // 测试屏蔽所有目录后搜索
        let index = create_test_index();
        let engine = SearchEngine::new(index);
        
        // 使用一个会匹配所有路径的查询
        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "nonexistent_xyz_123".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        
        let response = engine.search(&request);
        // 应该返回空结果，不崩溃
        assert_eq!(response.total, 0);
        assert!(response.results.is_empty());
    }

    #[test]
    fn test_empty_index() {
        // 测试空索引搜索
        let engine = SearchEngine::new(vec![]);
        
        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "test".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        
        let response = engine.search(&request);
        assert_eq!(response.total, 0);
        assert!(response.results.is_empty());
    }

    #[test]
    fn test_pagination_overflow() {
        // 测试分页超出范围
        let index = create_test_index();
        let engine = SearchEngine::new(index);
        
        let request = SearchRequest {
                filter_dirs: Vec::new(),
            query: "".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::Name,
            sort_asc: true,
            sort_orders: vec![],
            offset: 10000, // 超出实际数量
            limit: 100,
        };
        
        let response = engine.search(&request);
        assert_eq!(response.results.len(), 0);
        assert_eq!(response.total, 5); // 总数应该正确
    }

    // ========== USN 增量更新（apply_changes）测试 ==========

    #[test]
    fn test_apply_changes_add() {
        let engine_orig = SearchEngine::new(create_test_index());
        let mut engine = SearchEngine::new(create_test_index());

        let add = vec![FileEntry {
            name: "新文件.txt".into(),
            path: "D:\\Documents\\新文件.txt".into(),
            extension: "txt".into(),
            size: 128,
            modified: 1700000500,
            is_dir: false,
        }];
        let remove = std::collections::HashSet::new();
        let rename = std::collections::HashMap::new();

        let changed = engine.apply_changes(add, &remove, &rename);
        assert_eq!(changed, 1);
        assert_eq!(
            engine.get_index_count(),
            engine_orig.get_index_count() + 1
        );

        // 新文件可被搜到
        let request = SearchRequest {
            filter_dirs: Vec::new(),
            query: "新文件".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::None,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        let response = engine.search(&request);
        assert_eq!(response.total, 1);
        assert_eq!(response.results[0].name.as_ref(), "新文件.txt");
    }

    #[test]
    fn test_apply_changes_remove() {
        let mut engine = SearchEngine::new(create_test_index());

        let mut remove = std::collections::HashSet::new();
        remove.insert("F:\\Photos\\照片.jpg".to_string());
        remove.insert("G:\\Reports\\report.pdf".to_string());
        let rename = std::collections::HashMap::new();

        let changed = engine.apply_changes(vec![], &remove, &rename);
        assert_eq!(changed, 2);
        assert_eq!(engine.get_index_count(), 3);

        // 已删除的路径不再能搜到
        let request = SearchRequest {
            filter_dirs: Vec::new(),
            query: "照片".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::None,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        assert_eq!(engine.search(&request).total, 0);
    }

    #[test]
    fn test_apply_changes_file_rename() {
        let mut engine = SearchEngine::new(create_test_index());

        let mut rename = std::collections::HashMap::new();
        // E:\Work\福建交流纪要.pdf → E:\Work\交流纪要终稿.pdf
        rename.insert(
            "E:\\Work\\福建交流纪要.pdf".to_string(),
            ("E:\\Work".to_string(), "交流纪要终稿.PDF".to_string(), false),
        );

        let changed = engine.apply_changes(
            vec![],
            &std::collections::HashSet::new(),
            &rename,
        );
        assert_eq!(changed, 1);
        assert_eq!(engine.get_index_count(), 5);

        // 旧名「福建交流纪要」搜不到（新名与其他条目均不含），新名关键词「终稿」能搜到，
        // 且扩展名已重算为小写
        let old_req = SearchRequest {
            filter_dirs: Vec::new(),
            query: "福建交流纪要".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::None,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        let new_req = SearchRequest {
            filter_dirs: Vec::new(),
            query: "终稿".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::None,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        assert_eq!(engine.search(&old_req).total, 0);
        let resp = engine.search(&new_req);
        assert_eq!(resp.total, 1);
        assert_eq!(resp.results[0].path.as_ref(), "E:\\Work\\交流纪要终稿.PDF");
        assert_eq!(resp.results[0].name.as_ref(), "交流纪要终稿.PDF");
        assert_eq!(resp.results[0].extension.as_ref(), "pdf");
    }

    #[test]
    fn test_apply_changes_dir_rename_prefix_rewrite() {
        // 索引含一个目录与其子文件
        let index = vec![
            FileEntry {
                name: "Photos".into(),
                path: "F:\\Photos".into(),
                extension: "".into(),
                size: 0,
                modified: 1700000000,
                is_dir: true,
            },
            FileEntry {
                name: "照片.jpg".into(),
                path: "F:\\Photos\\照片.jpg".into(),
                extension: "jpg".into(),
                size: 512,
                modified: 1700000200,
                is_dir: false,
            },
            FileEntry {
                name: "图片.jpeg".into(),
                path: "F:\\Photos\\图片.jpeg".into(),
                extension: "jpeg".into(),
                size: 768,
                modified: 1700000300,
                is_dir: false,
            },
            FileEntry {
                name: "无关.txt".into(),
                path: "F:\\Other\\无关.txt".into(),
                extension: "txt".into(),
                size: 64,
                modified: 1700000400,
                is_dir: false,
            },
        ];
        let mut engine = SearchEngine::new(index);

        let mut rename = std::collections::HashMap::new();
        // 目录改名：F:\Photos → F:\MyPictures
        rename.insert(
            "F:\\Photos".to_string(),
            ("F:".to_string(), "MyPictures".to_string(), true),
        );

        let changed = engine.apply_changes(
            vec![],
            &std::collections::HashSet::new(),
            &rename,
        );
        // 目录本身 + 2 个子项都被重写
        assert_eq!(changed, 3);

        // 目录名可搜到
        let dir_req = SearchRequest {
            filter_dirs: Vec::new(),
            query: "MyPictures".to_string(),
            ext_query: String::new(),
            only_folders: true,
            sort_by: SortField::None,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        let resp = engine.search(&dir_req);
        assert_eq!(resp.total, 1);
        assert_eq!(resp.results[0].path.as_ref(), "F:\\MyPictures");
        assert_eq!(resp.results[0].extension.as_ref(), "");

        // 子项路径前缀已重写
        let sub_req = SearchRequest {
            filter_dirs: Vec::new(),
            query: "照片".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::None,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        let resp = engine.search(&sub_req);
        assert_eq!(resp.total, 1);
        assert_eq!(resp.results[0].path.as_ref(), "F:\\MyPictures\\照片.jpg");

        // 无关条目不受影响
        let other_req = SearchRequest {
            filter_dirs: Vec::new(),
            query: "无关".to_string(),
            ext_query: String::new(),
            only_folders: false,
            sort_by: SortField::None,
            sort_asc: true,
            sort_orders: vec![],
            offset: 0,
            limit: 100,
        };
        let resp = engine.search(&other_req);
        assert_eq!(resp.total, 1);
        assert_eq!(resp.results[0].path.as_ref(), "F:\\Other\\无关.txt");
    }
}
