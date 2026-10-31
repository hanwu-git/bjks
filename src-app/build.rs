//! 构建脚本：为 Windows exe 嵌入程序图标（资源管理器 / 任务栏 / 标题栏显示）
//! 并声明 UAC 管理员权限（USN 变更日志需要管理员权限打开卷句柄）
//!
//! 测试例外：设 BJKS_NO_MANIFEST=1 时不嵌入 UAC 清单（cargo test 的测试二进制
//! 无法在普通权限下启动带 requireAdministrator 的 exe，os error 740）
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        println!("cargo:rerun-if-env-changed=BJKS_NO_MANIFEST");
        if std::env::var("BJKS_NO_MANIFEST").is_err() {
            res.set_manifest(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>"#,
            );
        }
        res.compile().expect("嵌入 exe 资源失败");
    }
}
