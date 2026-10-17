//! QYZS CLI 可执行文件主入口

use std::path::Path;

#[tokio::main]
async fn main() {
    let mut args: Vec<String> = std::env::args().collect();

    // 别名形式兼容路由：若程序以 qyzs-pdf2md 形式被调用，内部自动重写为 qyzs pdf2md
    let sub_cmd = args.first().and_then(|exe_arg| {
        let file_stem = Path::new(exe_arg)
            .file_stem()
            .and_then(|s| s.to_str())?;
        file_stem.strip_prefix("qyzs-").map(|s| s.to_string())
    });

    if let Some(sub) = sub_cmd {
        args[0] = "qyzs".to_string();
        args.insert(1, sub);
    }

    if let Err(err) = qyzs::cli::run(args).await {
        eprintln!("\x1b[31m[ERR]\x1b[0m {}", err);
        std::process::exit(err.exit_code());
    }
}
