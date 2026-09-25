use rusqlite::{Connection, params};
use crate::models::FileEntry;

pub struct IndexCache {
    conn: Connection,
}

impl IndexCache {
    pub fn open(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path)
            .map_err(|e| e.to_string())?;
        Ok(Self { conn })
    }

    pub fn create_table(&self) -> Result<(), String> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS file_index (
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
            CREATE INDEX IF NOT EXISTS idx_extension ON file_index(extension);"
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn save_entries(&self, entries: &[FileEntry]) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction()
            .map_err(|e| e.to_string())?;
        
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO file_index 
                 (id, name, name_lower, path, parent_path, extension, size, modified, is_dir)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            ).map_err(|e| e.to_string())?;

            for entry in entries {
                stmt.execute(params![
                    entry.id,
                    entry.name,
                    entry.name_lower,
                    entry.path,
                    entry.parent_path,
                    entry.extension,
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
            "SELECT id, name, name_lower, path, parent_path, extension, size, modified, is_dir
             FROM file_index"
        ).map_err(|e| e.to_string())?;

        let entries = stmt.query_map([], |row| {
            let is_dir_int: i32 = row.get(8)?;
            Ok(FileEntry {
                id: row.get(0)?,
                name: row.get(1)?,
                name_lower: row.get(2)?,
                path: row.get(3)?,
                parent_path: row.get(4)?,
                extension: row.get(5)?,
                size: row.get(6)?,
                modified: row.get(7)?,
                is_dir: is_dir_int != 0,
            })
        }).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

        Ok(entries)
    }

    pub fn clear(&self) -> Result<(), String> {
        self.conn.execute("DELETE FROM file_index", [])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_test_entry(id: u64, name: &str) -> FileEntry {
        FileEntry {
            id,
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            path: format!("C:\\test\\{}", name),
            parent_path: "C:\\test".to_string(),
            extension: "txt".to_string(),
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
        assert_eq!(loaded[0].name, "file1.txt");
        assert_eq!(loaded[1].name, "file2.txt");
        assert_eq!(loaded[2].name, "file3.txt");

        // 清空数据
        cache.clear().expect("清空数据失败");
        let loaded_after_clear = cache.load_all().expect("加载数据失败");
        assert_eq!(loaded_after_clear.len(), 0, "清空后应该没有记录");

        // 清理测试文件
        drop(cache);
        let _ = fs::remove_file(db_path);
    }
}
