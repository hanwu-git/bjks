//! 构建脚本：为 Windows exe 嵌入程序图标（资源管理器 / 任务栏 / 标题栏显示）
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        res.compile().expect("嵌入 exe 图标失败");
    }
}
