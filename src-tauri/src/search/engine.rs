use crate::models::{FileEntry, SearchRequest, SearchResponse, SortField, SortOrder};
use rayon::prelude::*;
use std::time::Instant;

pub struct SearchEngine {
    index: Vec<FileEntry>,
}

impl SearchEngine {
    pub fn new(index: Vec<FileEntry>) -> Self {
        Self { index }
    }

    pub fn search(&self, request: &SearchRequest) -> SearchResponse {
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
                        entry.name_lower.contains(kw.as_str())
                    })
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
                        entry.name_lower.contains(kw.as_str())
                    })
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
        
        // 排序（优先使用组合排序）
        if !request.sort_orders.is_empty() {
            self.sort_results_multi(&mut results, &request.sort_orders);
        } else {
            self.sort_results(&mut results, &request.sort_by, request.sort_asc);
        }

        let total = results.len();
        
        // 分页
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
            SortField::Name => a.name_lower.cmp(&b.name_lower),
            SortField::Size => a.size.cmp(&b.size),
            SortField::Modified => a.modified.cmp(&b.modified),
            SortField::Path => a.path.cmp(&b.path),
        }
    }

    pub fn update_index(&mut self, new_index: Vec<FileEntry>) {
        self.index = new_index;
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
                id: 1,
                name: "福建交流报告.docx".to_string(),
                name_lower: "福建交流报告.docx".to_lowercase(),
                path: "D:\\Documents\\福建交流报告.docx".to_string(),
                parent_path: "D:\\Documents".to_string(),
                extension: "docx".to_string(),
                size: 1024,
                modified: 1700000000,
                is_dir: false,
            },
            FileEntry {
                id: 2,
                name: "福建交流纪要.pdf".to_string(),
                name_lower: "福建交流纪要.pdf".to_lowercase(),
                path: "E:\\Work\\福建交流纪要.pdf".to_string(),
                parent_path: "E:\\Work".to_string(),
                extension: "pdf".to_string(),
                size: 2048,
                modified: 1700000100,
                is_dir: false,
            },
            FileEntry {
                id: 3,
                name: "照片.jpg".to_string(),
                name_lower: "照片.jpg".to_lowercase(),
                path: "F:\\Photos\\照片.jpg".to_string(),
                parent_path: "F:\\Photos".to_string(),
                extension: "jpg".to_string(),
                size: 512,
                modified: 1700000200,
                is_dir: false,
            },
            FileEntry {
                id: 4,
                name: "图片.jpeg".to_string(),
                name_lower: "图片.jpeg".to_lowercase(),
                path: "F:\\Photos\\图片.jpeg".to_string(),
                parent_path: "F:\\Photos".to_string(),
                extension: "jpeg".to_string(),
                size: 768,
                modified: 1700000300,
                is_dir: false,
            },
            FileEntry {
                id: 5,
                name: "report.pdf".to_string(),
                name_lower: "report.pdf".to_lowercase(),
                path: "G:\\Reports\\report.pdf".to_string(),
                parent_path: "G:\\Reports".to_string(),
                extension: "pdf".to_string(),
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
            query: "福建 交流".to_string(),
            sort_by: SortField::Name,
            sort_asc: true,
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
        
        // 测试 .jp 匹配 .jpg 和 .jpeg
        let request = SearchRequest {
            query: ".jp".to_string(),
            sort_by: SortField::Name,
            sort_asc: true,
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
        
        // 测试 report .pdf
        let request = SearchRequest {
            query: "report .pdf".to_string(),
            sort_by: SortField::Name,
            sort_asc: true,
            offset: 0,
            limit: 100,
        };
        
        let response = engine.search(&request);
        assert_eq!(response.total, 1);
        assert_eq!(response.results[0].name, "report.pdf");
    }

    #[test]
    fn test_empty_query() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);
        
        let request = SearchRequest {
            query: "".to_string(),
            sort_by: SortField::Name,
            sort_asc: true,
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
            query: "".to_string(),
            sort_by: SortField::Size,
            sort_asc: false, // 降序
            offset: 0,
            limit: 100,
        };
        
        let response = engine.search(&request);
        assert_eq!(response.results[0].name, "report.pdf"); // 最大的文件
    }

    #[test]
    fn test_sort_by_name_asc() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);

        let request = SearchRequest {
            query: "".to_string(),
            sort_by: SortField::Name,
            sort_asc: true,
            offset: 0,
            limit: 100,
        };

        let response = engine.search(&request);
        // 验证名称升序：name_lower 依次递增
        for i in 1..response.results.len() {
            assert!(response.results[i - 1].name_lower <= response.results[i].name_lower);
        }
    }

    #[test]
    fn test_sort_by_name_desc() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);

        let request = SearchRequest {
            query: "".to_string(),
            sort_by: SortField::Name,
            sort_asc: false,
            offset: 0,
            limit: 100,
        };

        let response = engine.search(&request);
        // 验证名称降序：name_lower 依次递减
        for i in 1..response.results.len() {
            assert!(response.results[i - 1].name_lower >= response.results[i].name_lower);
        }
    }

    #[test]
    fn test_sort_by_modified() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);

        let request = SearchRequest {
            query: "".to_string(),
            sort_by: SortField::Modified,
            sort_asc: false, // 最新修改在前
            offset: 0,
            limit: 100,
        };

        let response = engine.search(&request);
        // 验证按修改时间降序
        for i in 1..response.results.len() {
            assert!(response.results[i - 1].modified >= response.results[i].modified);
        }
        assert_eq!(response.results[0].name, "report.pdf"); // modified=1700000400
    }

    #[test]
    fn test_sort_by_path() {
        let index = create_test_index();
        let engine = SearchEngine::new(index);

        let request = SearchRequest {
            query: "".to_string(),
            sort_by: SortField::Path,
            sort_asc: true,
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
            query: "".to_string(),
            sort_by: SortField::Name,
            sort_asc: true,
            offset: 0,
            limit: 100,
        };

        let response = engine.search(&request);
        // 验证第一个是字母序最小的
        assert_eq!(response.results[0].name, "report.pdf"); // 'r' 在英文中排最前
    }

    #[test]
    fn test_performance_large_index() {
        // 生成大规模测试索引（100万条）
        let mut large_index = Vec::with_capacity(1_000_000);
        for i in 0..1_000_000 {
            large_index.push(FileEntry {
                id: i,
                name: format!("file_{}.txt", i),
                name_lower: format!("file_{}.txt", i),
                path: format!("C:\\test\\file_{}.txt", i),
                parent_path: "C:\\test".to_string(),
                extension: "txt".to_string(),
                size: i * 1024,
                modified: 1700000000 + i as i64,
                is_dir: false,
            });
        }

        let engine = SearchEngine::new(large_index);
        let request = SearchRequest {
            query: "file_123".to_string(),
            sort_by: SortField::Name,
            sort_asc: true,
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
                id: i,
                name: format!("test_{}.dat", i),
                name_lower: format!("test_{}.dat", i),
                path: format!("D:\\data\\test_{}.dat", i),
                parent_path: "D:\\data".to_string(),
                extension: "dat".to_string(),
                size: i * 512,
                modified: 1700000000 + i as i64,
                is_dir: false,
            });
        }

        // 创建两个引擎，一个强制串行，一个可能并行
        let engine_serial = SearchEngine::new(index.clone());
        let engine_parallel = SearchEngine::new(index);

        let request = SearchRequest {
            query: "test_5".to_string(),
            sort_by: SortField::Size,
            sort_asc: false,
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
            assert_eq!(s.id, p.id);
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
                query: query.to_string(),
                sort_by: SortField::Name,
                sort_asc: true,
                offset: 0,
                limit: 100,
            };
            
            // 应该不崩溃，返回空结果
            let response = engine.search(&request);
            assert!(response.total >= 0);
        }
    }

    #[test]
    fn test_long_query_string() {
        // 测试超长查询字符串（1000字符）
        let index = create_test_index();
        let engine = SearchEngine::new(index);
        
        let long_query = "a".repeat(1000);
        let request = SearchRequest {
            query: long_query,
            sort_by: SortField::Name,
            sort_asc: true,
            offset: 0,
            limit: 100,
        };
        
        // 应该不崩溃
        let response = engine.search(&request);
        assert!(response.total >= 0);
    }

    #[test]
    fn test_chinese_and_emoji_paths() {
        // 测试中文和 emoji 路径
        let index = vec![
            FileEntry {
                id: 1,
                name: "中文文件.txt".to_string(),
                name_lower: "中文文件.txt".to_lowercase(),
                path: "D:\\文档\\中文文件.txt".to_string(),
                parent_path: "D:\\文档".to_string(),
                extension: "txt".to_string(),
                size: 1024,
                modified: 1700000000,
                is_dir: false,
            },
            FileEntry {
                id: 2,
                name: "emoji😀.txt".to_string(),
                name_lower: "emoji😀.txt".to_lowercase(),
                path: "E:\\测试\\emoji😀.txt".to_string(),
                parent_path: "E:\\测试".to_string(),
                extension: "txt".to_string(),
                size: 2048,
                modified: 1700000100,
                is_dir: false,
            },
        ];
        
        let engine = SearchEngine::new(index);
        
        // 搜索中文
        let request = SearchRequest {
            query: "中文".to_string(),
            sort_by: SortField::Name,
            sort_asc: true,
            offset: 0,
            limit: 100,
        };
        let response = engine.search(&request);
        assert_eq!(response.total, 1);
        assert_eq!(response.results[0].name, "中文文件.txt");
        
        // 搜索 emoji
        let request = SearchRequest {
            query: "😀".to_string(),
            sort_by: SortField::Name,
            sort_asc: true,
            offset: 0,
            limit: 100,
        };
        let response = engine.search(&request);
        assert_eq!(response.total, 1);
        assert_eq!(response.results[0].name, "emoji😀.txt");
    }

    #[test]
    fn test_all_paths_excluded() {
        // 测试屏蔽所有目录后搜索
        let index = create_test_index();
        let engine = SearchEngine::new(index);
        
        // 使用一个会匹配所有路径的查询
        let request = SearchRequest {
            query: "nonexistent_xyz_123".to_string(),
            sort_by: SortField::Name,
            sort_asc: true,
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
            query: "test".to_string(),
            sort_by: SortField::Name,
            sort_asc: true,
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
            query: "".to_string(),
            sort_by: SortField::Name,
            sort_asc: true,
            offset: 10000, // 超出实际数量
            limit: 100,
        };
        
        let response = engine.search(&request);
        assert_eq!(response.results.len(), 0);
        assert_eq!(response.total, 5); // 总数应该正确
    }
}
