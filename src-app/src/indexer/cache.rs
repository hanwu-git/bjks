use rusqlite::{Connection, params};
use crate::models::FileEntry;

/// 缓存表结构。
/// path 索引是增量更新的性能关键：删除/改名都按完整路径定位，
/// 缺少 idx_path 时每次操作都会退化为 27 万行全表扫描，批量变更会把进程拖死。
const SCHEMA_SQL: &str = "
    CREATE TABLE IF NOT EXISTS file_index (
        name TEXT NOT NULL,
        path TEXT NOT NULL,
        extension TEXT NOT NULL,
        size INTEGER NOT NULL,
        modified INTEGER NOT NULL,
        is_dir INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_extension ON file_index(extension);
    CREATE INDEX IF NOT EXISTS idx_path ON file_index(path);
    CREATE TABLE IF NOT EXISTS cache_meta (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );";

pub struct IndexCache {
    conn: Connection,
}

impl IndexCache {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path)
            .map_err(|e| e.to_string())?;
        // 打开即确保表结构与索引存在（旧缓存库会自动补建 idx_path）
        conn.execute_batch(SCHEMA_SQL).map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    pub fn create_table(&self) -> Result<(), String> {
        self.conn.execute_batch(SCHEMA_SQL).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 写入缓存元数据（key 存在则覆盖），如记录本次索引使用的盘符
    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn.execute(
            "INSERT INTO cache_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 读取缓存元数据
    pub fn get_meta(&self, key: &str) -> Option<String> {
        self.conn
            .query_row("SELECT value FROM cache_meta WHERE key = ?1", params![key], |row| row.get(0))
            .ok()
    }

    pub fn save_entries(&self, entries: &[FileEntry]) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction()
            .map_err(|e| e.to_string())?;

        // 先清空旧记录再写入，保证保存操作幂等：
        // 否则缓存过期后台刷新时会向旧表追加，索引整体翻倍，搜索结果出现成对重复
        tx.execute("DELETE FROM file_index", params![])
            .map_err(|e| e.to_string())?;

        {
            let mut stmt = tx.prepare(
                "INSERT INTO file_index 
                 (name, path, extension, size, modified, is_dir)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)"
            ).map_err(|e| e.to_string())?;

            for entry in entries {
                stmt.execute(params![
                    entry.name.as_ref(),
                    entry.path.as_ref(),
                    entry.extension.as_ref(),
                    entry.size,
                    entry.modified,
                    entry.is_dir as i32,
                ]).map_err(|e| e.to_string())?;
            }
        }
        
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn load_all(&self) -> Result<Vec<FileEntry>, String> {
        let mut stmt = self.conn.prepare(
            "SELECT name, path, extension, size, modified, is_dir
             FROM file_index"
        ).map_err(|e| e.to_string())?;

        let entries = stmt.query_map([], |row| {
            let is_dir_int: i32 = row.get(5)?;
            Ok(FileEntry {
                name: row.get::<_, String>(0)?.into_boxed_str(),
                path: row.get::<_, String>(1)?.into_boxed_str(),
                extension: row.get::<_, String>(2)?.into_boxed_str(),
                size: row.get(3)?,
                modified: row.get(4)?,
                is_dir: is_dir_int != 0,
            })
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        Ok(entries)
    }

    /// 把 USN 增量变更写回缓存（单事务：删除 / 新增 / 改名，目录改名按前缀重写子树）
    /// renames: (旧路径, 新路径, 是否目录, 新文件名, 新扩展名)
    pub fn apply_batch(
        &self,
        deleted: &[String],
        created: &[FileEntry],
        renames: &[(String, String, bool, String, Box<str>)],
    ) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(|e| e.to_string())?;
        {
            let mut del = tx
                .prepare("DELETE FROM file_index WHERE path = ?1")
                .map_err(|e| e.to_string())?;
            for path in deleted {
                del.execute(params![path]).map_err(|e| e.to_string())?;
            }

            let mut ins = tx.prepare(
                "INSERT INTO file_index (name, path, extension, size, modified, is_dir)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|e| e.to_string())?;
            for e in created {
                ins.execute(params![
                    e.name.as_ref(),
                    e.path.as_ref(),
                    e.extension.as_ref(),
                    e.size,
                    e.modified,
                    e.is_dir as i32,
                ])
                .map_err(|e| e.to_string())?;
            }

            for (old, new, is_dir, new_name, new_ext) in renames {
                if *is_dir {
                    // 目录改名：子树前缀重写——`old\X` → `new\X`
                    // 关键点：
                    // 1) 用范围查询（`old\` ~ `old\` + 最大码点）而不是 substr() 前缀比较，
                    //    这样能命中 idx_path 索引，避免每次全表扫描 27 万行；
                    // 2) 上界必须用 U+10FFFF 而不是 `]`：SQLite 文本默认按 UTF-8 字节比较，
                    //    中文首字节 ≥ 0xE4 远大于 ']'(0x5D)，用 `]` 会把中文子目录整批漏掉；
                    // 3) 分隔符必须用参数 ?3 传单个反斜杠：SQLite 的字符串字面量不识别反斜杠转义，
                    //    写成 '\\' 实际是两个反斜杠，会把路径改成 `C:\new\\child`；
                    // 4) 截断位置用 SQL 的 length(?2)（按字符计），不能用 Rust 的 old.len()（按字节），
                    //    否则中文目录名会被多截掉几个字符。
                    tx.execute(
                        "UPDATE file_index
                         SET path = ?1 || ?3 || substr(path, length(?2) + 2)
                         WHERE path >= ?2 || ?3 AND path < ?2 || char(0x10FFFF)",
                        params![new, old, "\\"],
                    )
                    .map_err(|e| e.to_string())?;
                    // 目录本身
                    tx.execute(
                        "UPDATE file_index SET path = ?1, name = ?2, extension = '' WHERE path = ?3",
                        params![new, new_name, old],
                    )
                    .map_err(|e| e.to_string())?;
                } else {
                    tx.execute(
                        "UPDATE file_index SET path = ?1, name = ?2, extension = ?3 WHERE path = ?4",
                        params![new, new_name, new_ext.as_ref(), old],
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_test_entry(id: u64, name: &str) -> FileEntry {
        let _ = id;
        FileEntry {
            name: name.into(),
            path: format!("C:\\test\\{}", name).into(),
            extension: "txt".into(),
            size: 1024,
            modified: 1700000000,
            is_dir: false,
        }
    }

    #[test]
    fn test_cache_operations() {
        let db_path = "test_cache.db";
        
        // 清理可能存在的旧文件
        let _ = fs::remove_file(db_path);

        // 打开数据库
        let cache = IndexCache::open(db_path).expect("打开数据库失败");
        
        // 创建表
        cache.create_table().expect("创建表失败");

        // 保存数据
        let entries = vec![
            create_test_entry(1, "file1.txt"),
            create_test_entry(2, "file2.txt"),
            create_test_entry(3, "file3.txt"),
        ];
        cache.save_entries(&entries).expect("保存数据失败");

        // 加载数据
        let loaded = cache.load_all().expect("加载数据失败");
        assert_eq!(loaded.len(), 3, "应该加载 3 条记录");
        assert_eq!(loaded[0].name.as_ref(), "file1.txt");
        assert_eq!(loaded[1].name.as_ref(), "file2.txt");
        assert_eq!(loaded[2].name.as_ref(), "file3.txt");

        // 清理测试文件
        drop(cache);
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn test_save_twice_no_duplicates() {
        let db_path = "test_cache_twice.db";
        let _ = fs::remove_file(db_path);

        let cache = IndexCache::open(db_path).expect("打开数据库失败");
        cache.create_table().expect("创建表失败");

        let entries = vec![
            create_test_entry(1, "file1.txt"),
            create_test_entry(2, "file2.txt"),
        ];

        // 模拟缓存过期后台刷新：同一张表连续保存两次，第二次应整体替换而非追加
        cache.save_entries(&entries).expect("第一次保存失败");
        cache.save_entries(&entries).expect("第二次保存失败");

        let loaded = cache.load_all().expect("加载数据失败");
        assert_eq!(loaded.len(), 2, "重复保存后不应出现翻倍的记录");

        drop(cache);
        let _ = fs::remove_file(db_path);
    }
}
