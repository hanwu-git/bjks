//! USN 增量更新端到端自检（`BJKS.exe --usn-selftest [盘符]`）
//!
//! 在真实卷上依次执行「创建 → 改名 → 删除」临时文件，用与生产代码完全相同的
//! `read_changes` + `coalesce` 链路断言变更是否被正确识别。
//! 结果写入应用日志（%LOCALAPPDATA%\本机快搜\app.log），退出码 0 表示全部通过。
//! 该模式需要管理员权限（打开卷句柄），由 exe 清单自动申请。

use super::usn::{self, VolUsnState};
use crate::logger::{app_data_dir, app_log};
use std::fs;
use std::thread::sleep;
use std::time::Duration;

/// USN 记录从写盘到可读有微小延迟，每步等待后读取
const SETTLE_MS: u64 = 800;

fn log(msg: &str) {
    app_log(msg);
}

/// 读一轮变更并归并，打印原始/归并统计
fn step(state: &mut VolUsnState, drive: &str, tag: &str) -> usn::UsnBatch {
    match usn::read_changes(drive, state) {
        Some(records) => {
            let raw = records.len();
            let batch = usn::coalesce(drive, &records, state);
            log(&format!(
                "[{}] 原始 {} 条 → 新增 {} 删除 {} 改名 {}",
                tag,
                raw,
                batch.created.len(),
                batch.deleted.len(),
                batch.renamed.len()
            ));
            for (_, p, _) in batch.created.iter().take(3) {
                log(&format!("  + {}", p));
            }
            for p in batch.deleted.iter().take(3) {
                log(&format!("  - {}", p));
            }
            for (a, b, _) in batch.renamed.iter().take(3) {
                log(&format!("  ~ {} => {}", a, b));
            }
            batch
        }
        None => {
            log(&format!("[{}] 读取 USN 变更失败（游标失效或权限不足）", tag));
            usn::UsnBatch::default()
        }
    }
}

/// 执行自检并返回退出码：0=全部通过，1=有断言失败，2=环境类失败
pub fn run(drive: &str) -> i32 {
    log(&format!("===== USN 自检开始（卷 {}）=====", drive));

    // 1. 查询日志
    let (journal_id, _) = match usn::query_journal(drive) {
        Some(v) => v,
        None => {
            log("✗ 查询 USN 日志失败（无管理员权限或该卷非 NTFS）");
            return 2;
        }
    };
    log(&format!("✓ USN 日志可查询，journal_id={}", journal_id));

    // 2. 枚举 MFT（同时把游标定位到当前末尾）
    let mut state = match usn::enum_mft(drive) {
        Some(s) => s,
        None => {
            log("✗ MFT 枚举失败");
            return 2;
        }
    };
    log(&format!("✓ MFT 枚举完成，目录树 {} 条", state.dirs.len()));
    for (frn, node) in state.dirs.iter().take(3) {
        log(&format!("  样例目录 FRN={} 父={} 名={}", frn, node.parent_frn, node.name));
    }

    let dir = app_data_dir();
    let f1 = dir.join("usn_selftest_a.tmp");
    let f2 = dir.join("usn_selftest_b.tmp");
    let name1 = "usn_selftest_a.tmp";
    let name2 = "usn_selftest_b.tmp";

    // 清理上次残留
    let _ = fs::remove_file(&f1);
    let _ = fs::remove_file(&f2);
    sleep(Duration::from_millis(SETTLE_MS));
    let _ = step(&mut state, drive, "清理"); // 吃掉清理产生的记录

    let mut fails = 0;

    // 3. 创建
    if let Err(e) = fs::write(&f1, b"bjks usn selftest") {
        log(&format!("✗ 创建测试文件失败：{}", e));
        return 2;
    }
    sleep(Duration::from_millis(SETTLE_MS));
    let b = step(&mut state, drive, "创建");
    if b.created.iter().any(|(_, p, _)| p.ends_with(name1)) {
        log("✓ 创建事件被正确识别");
    } else {
        log("✗ 创建事件未被识别");
        fails += 1;
    }

    // 4. 改名
    if let Err(e) = fs::rename(&f1, &f2) {
        log(&format!("✗ 改名失败：{}", e));
        return 2;
    }
    sleep(Duration::from_millis(SETTLE_MS));
    let b = step(&mut state, drive, "改名");
    if b.renamed.iter().any(|(_, new, _)| new.ends_with(name2)) {
        log("✓ 改名事件被正确识别");
    } else {
        log("✗ 改名事件未被识别");
        fails += 1;
    }

    // 5. 删除
    if let Err(e) = fs::remove_file(&f2) {
        log(&format!("✗ 删除失败：{}", e));
        return 2;
    }
    sleep(Duration::from_millis(SETTLE_MS));
    let b = step(&mut state, drive, "删除");
    if b.deleted.iter().any(|p| p.ends_with(name2)) {
        log("✓ 删除事件被正确识别");
    } else {
        log("✗ 删除事件未被识别");
        fails += 1;
    }

    if fails == 0 {
        log("===== USN 自检通过（创建/改名/删除 3 项全部正确）=====");
        0
    } else {
        log(&format!("===== USN 自检失败（{} 项未通过）=====", fails));
        1
    }
}
