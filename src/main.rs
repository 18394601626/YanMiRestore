//! 程序入口：初始化日志并启动命令执行。
mod app;
mod carver;
mod cli;
mod config;
mod device;
mod error;
mod fs;
mod model;
mod recovery;
mod report;
mod scanner;
mod utils;

use std::io::Write;

use clap::{CommandFactory, Parser};
use cli::Cli;
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    init_tracing();

    if std::env::args_os().len() <= 1 {
        let _ = config::init(None);
        print_startup_help()?;
        wait_for_enter();
        return Ok(());
    }

    let cli = Cli::parse();
    config::init(cli.config.as_deref())?;
    app::run(cli)
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .compact()
        .init();
}

fn print_startup_help() -> anyhow::Result<()> {
    println!("检测到未传入命令参数。");
    println!("这是命令行程序，请在终端中按子命令方式运行。");
    println!();

    Cli::command().print_help()?;
    println!();
    println!();
    println!("示例：");
    println!("  YanMiRestore.exe devices");
    println!("  YanMiRestore.exe design --case-id CASE-1001 --target-kind pc-disk --depth deep");
    println!(
        "  YanMiRestore.exe scan --source E:\\evidence\\disk.img --output .\\output --case-id CASE-1001 --depth deep --include-carving"
    );
    println!(
        "  YanMiRestore.exe recover --report .\\output\\CASE-1001-scan-report.json --destination .\\restore --execute"
    );
    Ok(())
}

fn wait_for_enter() {
    print!("按回车键退出...");
    let _ = std::io::stdout().flush();
    let mut input = String::new();
    match std::io::stdin().read_line(&mut input) {
        Ok(0) | Err(_) => {
            // 双击启动时标准输入可能不可用，短暂停留后退出，避免窗口一闪而过。
            let seconds = config::settings().ui.startup_hold_seconds.max(1);
            std::thread::sleep(std::time::Duration::from_secs(seconds));
        }
        Ok(_) => {}
    }
}
