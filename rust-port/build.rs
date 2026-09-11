fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");

    // Windows 资源：程序图标 + DPI 感知清单 + 版本信息
    #[cfg(windows)]
    embed_resource::compile("assets/app.rc", embed_resource::NONE);

    // 前端单文件资源必须已生成，否则编译期直接给出可读的错误
    let html = std::path::Path::new("assets/app.html");
    if !html.exists() {
        panic!(
            "缺少 assets/app.html。请先在 rust-port/ 目录执行：\n    node scripts/build-frontend.mjs --build\n"
        );
    }
    println!("cargo:rerun-if-changed=assets/app.html");
}
