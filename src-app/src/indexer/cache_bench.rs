//! 缓存增量写回离线压测（无需管理员权限、无需 NTFS）
//!
//! 用法：`BJKS.exe --cache-bench [缓存库路径]`
//!
//! 做什么：把真实缓存库复制一份到临时文件，用与实测崩溃同规模的批处理
//! （约 7800 删除 / 8500 新增 / 7700 改名，含中文目录改名）跑一遍
//! `IndexCache::apply_batch`，输出各阶段耗时、查询计划与实际写入结果。
//!
//! 为什么需要它：运行中的索引更新崩溃点在「开始写缓存」之后，而该阶段依赖
//! 真实缓存库的数据分布（27 万行 + 索引状态），只有用真实库跑才能复现。
//! 本压测不触碰原库（只在副本上操作），可反复运行。

use crate::indexer::cache::IndexCache;
use crate::logger::app_log;
use crate::models::FileEntry;
use std::time::Instant;

/// 删除条数（对齐 2026-11-01 01:38 实测日志）
const BENCH_DELETE: usize = 7822;
/// 新增条数
const BENCH_CREATE: usize = 8496;
/// 改名条数
const BENCH_RENAME: usize = 7745;

fn extension_of(name: &str) -> Box<str> {
    match name.rfind('.') {
        Some(pos) if pos + 1 < name.len() => name[pos + 1..].to_lowercase().into_boxed_str(),
        _ => "".into(),
    }
}

fn file_entry(path: &str) -> FileEntry {
    let name = path.rsplit('\\').next().unwrap_or(path);
    FileEntry {
        name: name.into(),
        path: path.into(),
        extension: extension_of(name),
        size: 1024,
        modified: 1_700_000_000,
        is_dir: false,
    }
}

/// 打印某条 SQL 的查询计划（验证是否命中索引，这是「全表扫描把进程拖死」的关键判据）
fn explain(db: &rusqlite::Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) {
    match db.prepare(&format!("EXPLAIN QUERY PLAN {}", sql)) {
        Ok(mut stmt) => {
            let rows = stmt.query_map(params, |r| r.get::<_, String>(3));
            if let Ok(rows) = rows {
                for r in rows.flatten() {
                    app_log(&format!("    [查询计划] {}", r));
                }
            }
        }
        Err(e) => app_log(&format!("    [查询计划] 生成失败：{}", e)),
    }
}

pub fn run(db_path: &str) -> i32 {
    let src = std::path::Path::new(db_path);
    if !src.exists() {
        app_log(&format!("缓存压测：缓存库不存在，跳过（{}）", db_path));
        return 2;
    }

    // 副本放在同目录：跨盘复制 48MB 也要几秒，同目录更快且不影响原库
    let bench_path = src.with_extension("bench.db");
    let _ = std::fs::remove_file(&bench_path);

    let t0 = Instant::now();
    if let Err(e) = std::fs::copy(src, &bench_path) {
        app_log(&format!("缓存压测：复制缓存库失败（{}）", e));
        return 2;
    }
    let bench = bench_path.to_str().unwrap_or_default().to_string();
    app_log(&format!(
        "缓存压测：[1/6] 复制缓存库完成，耗时 {:?}，副本 {}",
        t0.elapsed(),
        bench
    ));

    // 2. 打开（旧库会在这里补建 idx_path 索引，本身是 27 万行的一次性成本）
    let t1 = Instant::now();
    let cache = match IndexCache::open(&bench) {
        Ok(c) => c,
        Err(e) => {
            app_log(&format!("缓存压测：打开失败（{}）", e));
            return 2;
        }
    };
    app_log(&format!(
        "缓存压测：[2/6] 打开并确保表结构/索引，耗时 {:?}",
        t1.elapsed()
    ));

    // 3. 读取全部条目作为样本（真实数据分布）
    let t2 = Instant::now();
    let entries = match cache.load_all() {
        Ok(v) => v,
        Err(e) => {
            app_log(&format!("缓存压测：读取条目失败（{}）", e));
            return 2;
        }
    };
    app_log(&format!(
        "缓存压测：[3/6] 读取 {} 条缓存条目，耗时 {:?}",
        entries.len(),
        t2.elapsed()
    ));
    if entries.len() < BENCH_DELETE + BENCH_RENAME {
        app_log("缓存压测：条目不足，按实际数量缩减规模");
    }
    let files: Vec<&FileEntry> = entries.iter().filter(|e| !e.is_dir).collect();
    let dirs: Vec<&FileEntry> = entries.iter().filter(|e| e.is_dir).collect();

    // 3.5 历史脏数据检查：旧版目录改名 SQL 用了 '\\' 字面量（SQLite 里等于两个反斜杠），
    // 可能把路径写成 `C:\new\\child`；顺带确认中文路径没有被 substr 截断成残片
    {
        let conn = cache_conn(&bench);
        let dirty: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_index WHERE instr(path, ?1) > 0",
                [r"\\"],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        let sample: Option<String> = conn
            .query_row(
                "SELECT path FROM file_index WHERE instr(path, ?1) > 0 LIMIT 1",
                [r"\\"],
                |r| r.get(0),
            )
            .ok();
        app_log(&format!(
            "缓存压测：[3.5/6] 历史脏路径（含双反斜杠）{} 条，样例 {:?}",
            dirty, sample
        ));
    }

    // 4. 组装与实测同规模的批处理
    let n_del = BENCH_DELETE.min(files.len() / 3);
    let deleted: Vec<String> = files.iter().take(n_del).map(|e| e.path.to_string()).collect();

    let created: Vec<FileEntry> = (0..BENCH_CREATE)
        .map(|i| file_entry(&format!("C:\\bench_new\\新建文件_{}.tmp", i)))
        .collect();

    let n_ren = BENCH_RENAME.min(files.len() / 2);
    let mut renames: Vec<(String, String, bool, String, Box<str>)> = files
        .iter()
        .skip(n_del)
        .take(n_ren)
        .enumerate()
        .map(|(i, e)| {
            let old = e.path.to_string();
            let new = format!("C:\\bench_new\\改名文件_{}.txt", i);
            (old, new.clone(), false, format!("改名文件_{}.txt", i), "txt".into())
        })
        .collect();
    // 额外的目录改名（含中文）：验证子树前缀重写与 UTF-8 截断
    // 跳过盘根（`C:\` 拼接分隔符后无意义）
    let mut dir_checks: Vec<(String, String)> = Vec::new();
    for (i, d) in dirs.iter().filter(|d| d.path.len() > 3).take(3).enumerate() {
        let old = d.path.to_string();
        let new = format!("C:\\压测改名目录_{}", i);
        dir_checks.push((old.clone(), new.clone()));
        renames.push((old, new.clone(), true, format!("压测改名目录_{}", i), "".into()));
    }

    app_log(&format!(
        "缓存压测：[4/6] 组装批处理：删除 {} / 新增 {} / 改名 {}（含 {} 个目录）",
        deleted.len(),
        created.len(),
        renames.len(),
        dir_checks.len()
    ));

    // 5. 查询计划：确认增量语句命中索引（不命中会退化为 27 万行全表扫描 × N 次）
    app_log("缓存压测：[5/6] 关键语句查询计划");
    explain(&cache_conn(&bench), "SELECT 1 WHERE 0", &[]);
    explain(
        &cache_conn(&bench),
        "UPDATE file_index
         SET path = ?1 || ?3 || substr(path, length(?2) + 2)
         WHERE path >= ?2 || ?3 AND path < ?2 || char(0x10FFFF)",
        &[&"C:\\new", &"C:\\old", &"\\"],
    );
    explain(
        &cache_conn(&bench),
        "DELETE FROM file_index WHERE path = ?1",
        &[&"C:\\x"],
    );

    // 6. 正式压测
    let t3 = Instant::now();
    let before = entries.len();
    match cache.apply_batch(&deleted, &created, &renames) {
        Ok(()) => {
            app_log(&format!(
                "缓存压测：[6/6] apply_batch 完成，耗时 {:?}（删除 {} + 新增 {} + 改名 {}）",
                t3.elapsed(),
                deleted.len(),
                created.len(),
                renames.len()
            ));
        }
        Err(e) => {
            app_log(&format!("缓存压测：apply_batch 失败（{}）", e));
            return 1;
        }
    }
    drop(cache);

    // 校验：条数 = 原 - 删除 + 新增（改名不改变数量）；目录改名子树已重写
    let conn = cache_conn(&bench);
    let after: i64 = conn
        .query_row("SELECT COUNT(*) FROM file_index", [], |r| r.get(0))
        .unwrap_or(-1);
    let expect = before as i64 - deleted.len() as i64 + created.len() as i64;
    app_log(&format!(
        "缓存压测：条数校验 期望 {} 实际 {} → {}",
        expect,
        after,
        if after == expect { "通过" } else { "失败" }
    ));

    let mut ok = after == expect;
    for (old, new) in &dir_checks {
        let leftover: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_index
                 WHERE path >= ?1 || char(92) AND path < ?1 || char(0x10FFFF)",
                [old],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        let rewritten: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_index
                 WHERE path >= ?1 || char(92) AND path < ?1 || char(0x10FFFF)",
                [new],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        let origin: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_index WHERE path = ?1",
                [old],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        // 通过判据：旧子树已无残留、旧目录本身已不在原路径；
        // 新子树条数允许为 0（原目录可能是空目录，其自身用另一条 UPDATE 处理）
        let pass = leftover == 0 && origin == 0;
        if !pass {
            ok = false;
        }
        app_log(&format!(
            "缓存压测：目录改名校验 [{}] → [{}] 旧子树残留 {} 新子树 {} 原目录 {} → {}",
            old,
            new,
            leftover,
            rewritten,
            origin,
            if pass { "通过" } else { "失败" }
        ));
    }
    drop(conn);
    let _ = std::fs::remove_file(&bench_path);

    app_log(&format!(
        "缓存压测结束：{}（总耗时 {:?}）",
        if ok { "全部通过" } else { "存在失败项" },
        t0.elapsed()
    ));
    if ok {
        0
    } else {
        1
    }
}

/// 单独开一个只读连接用于查询计划与校验（不与写入连接共享状态）
fn cache_conn(path: &str) -> rusqlite::Connection {
    rusqlite::Connection::open(path).unwrap_or_else(|_| {
        // 兜底：内存库，保证 explain 不会 panic
        rusqlite::Connection::open_in_memory().expect("内存库不可用")
    })
}
