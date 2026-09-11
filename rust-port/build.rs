fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-changed=assets/app.html");
    println!("cargo:rerun-if-changed=assets/appicon.png");

    // Windows 资源：程序图标 + DPI 感知清单 + 版本信息
    //
    // 注意：用 compile_for 只作用于主程序，不要用 compile()。
    // 否则资源（含清单）也会被链进 cargo test 生成的测试可执行文件，
    // 与 rustc 自身嵌入的清单冲突，启动时报「并行配置不正确」(14001)。
    #[cfg(windows)]
    {
        use embed_resource::CompilationResult;
        if let CompilationResult::Failed(err) =
            embed_resource::compile_for("assets/app.rc", &["PasswordManager"], embed_resource::NONE)
        {
            panic!("编译 Windows 资源失败: {err}");
        }
    }

    // 前端单文件资源必须已生成，否则编译期直接给出可读的错误
    if !std::path::Path::new("assets/app.html").exists() {
        panic!(
            "缺少 assets/app.html。请先在 rust-port/ 目录执行：\n    node scripts/build-frontend.mjs --build\n"
        );
    }
}
