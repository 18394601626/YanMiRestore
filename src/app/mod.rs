//! 应用入口编排：命令分发与流程控制。
use std::io::IsTerminal;
use std::io::{self, Write};

use anyhow::Result;
use tracing::info;

use crate::cli::{Cli, Commands, DesignArgs, RecoverArgs, ScanArgs};
use crate::device;
use crate::device::DeviceInspector;
use crate::device::LocalDeviceInspector;
use crate::model::{PlanInput, RecoveryRequest, ScanRequest, TargetKind};
use crate::utils::label::target_kind_label;
use crate::{recovery, report, scanner};

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Design(args) => run_design(args),
        Commands::Devices => run_devices(),
        Commands::Scan(args) => run_scan(args)?,
        Commands::Recover(args) => run_recover(args)?,
    }

    Ok(())
}

fn run_design(args: DesignArgs) {
    println!("正在生成恢复方案，请稍候...");
    let plan_input = PlanInput {
        case_id: args.case_id,
        target_kind: args.target_kind,
        depth: args.depth,
        fs_hint: args.fs_hint,
        include_carving: args.include_carving,
    };

    let plan = scanner::build_plan(&plan_input);
    report::print_plan(&plan);
    println!("恢复方案生成完成。");
}

fn run_devices() {
    println!("正在检测可用设备...");
    let devices = device::list_available_devices();
    if devices.is_empty() {
        println!("未检测到可用设备。可手动使用 --source 指定路径。");
        return;
    }

    println!("共检测到 {} 个设备：", devices.len());
    for (index, item) in devices.iter().enumerate() {
        println!(
            "{}. {} | 类型：{} | 说明：{}",
            index + 1,
            item.path.display(),
            target_kind_label(item.target_kind),
            item.note
        );
    }
    println!("可直接复制以上路径用于 scan --source <路径>。");
}

fn run_scan(args: ScanArgs) -> Result<()> {
    println!("正在执行只读扫描，请耐心等待...");
    let skip_confirm = args.yes;
    let request = ScanRequest {
        plan: PlanInput {
            case_id: args.case_id,
            target_kind: args.target_kind,
            depth: args.depth,
            fs_hint: args.fs_hint,
            include_carving: args.include_carving,
        },
        source: args.source,
        output_dir: args.output,
        target_kind: args.target_kind,
        depth: args.depth,
        fs_hint: args.fs_hint,
        include_carving: args.include_carving,
    };

    if !maybe_confirm_auto_detect(&request, skip_confirm)? {
        println!("已取消扫描。");
        return Ok(());
    }

    let scan_report = scanner::execute_scan(&request)?;
    let report_path = report::write_scan_report(&scan_report, &request.output_dir)?;

    report::print_scan_summary(&scan_report);
    info!(path = ?report_path, "扫描报告已写入");
    println!("扫描完成，报告已保存：{}", report_path.display());
    Ok(())
}

fn run_recover(args: RecoverArgs) -> Result<()> {
    if args.execute {
        println!("正在执行恢复，请勿关闭窗口...");
    } else {
        println!("正在执行恢复预演（仅生成计划，不写入文件）...");
    }

    let request = RecoveryRequest {
        report_path: args.report,
        destination: args.destination,
        dry_run: !args.execute,
        keep_original_name: !args.no_keep_original_name,
        preserve_timestamps: !args.no_preserve_timestamps,
        skip_carved: args.skip_carved,
    };

    let session = recovery::execute_recovery(&request)?;
    report::print_recovery_session(&session);
    println!("恢复任务完成。");
    Ok(())
}

fn maybe_confirm_auto_detect(request: &ScanRequest, skip_confirm: bool) -> Result<bool> {
    if request.target_kind != TargetKind::Auto || skip_confirm {
        return Ok(true);
    }

    if !io::stdin().is_terminal() {
        println!("检测到非交互终端，已跳过自动识别键入确认。");
        return Ok(true);
    }

    let inspector = LocalDeviceInspector;
    let snapshot = inspector.inspect(request)?;
    let detected = snapshot.detected_target_kind.unwrap_or(TargetKind::Other);
    let hint = snapshot.device_hint.as_deref().unwrap_or("无可用识别依据");

    println!("自动识别结果：{}（{}）", target_kind_label(detected), hint);
    println!("请输入 y/yes 确认继续扫描，输入其他内容将取消：");
    print!("> ");
    io::stdout().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let answer = answer.trim().to_ascii_lowercase();
    if answer == "y" || answer == "yes" {
        println!("已确认，继续扫描。");
        Ok(true)
    } else {
        Ok(false)
    }
}
