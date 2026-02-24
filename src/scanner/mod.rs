//! 扫描编排模块：生成计划并组装最终扫描报告。

use std::time::Duration;

use chrono::Utc;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::carver::{FileCarver, SignatureCarver};
use crate::config;
use crate::device::{DeviceInspector, LocalDeviceInspector};
use crate::error::RecoveryResult;
use crate::fs::{FileSystemScanner, HeuristicFsScanner};
use crate::model::{
    DeviceSnapshot, PlanInput, PlanStage, ScanPlan, ScanReport, ScanRequest, TargetKind,
};

const DEFAULT_SCAN_PROGRESS_TEMPLATE: &str =
    "{spinner:.green} {prefix:.bold} [{elapsed_precise}] [{bar:32.cyan/blue}] {pos}/{len} {percent:>3}% {msg}";
const DEFAULT_PROGRESS_CHARS: &str = "=>-";
const DEFAULT_SCAN_PROGRESS_PREFIX: &str = "扫描";
const DEFAULT_SPINNER_FRAMES: [&str; 4] = ["|", "/", "-", "\\"];

pub fn build_plan(input: &PlanInput) -> ScanPlan {
    let mut stages = vec![
        stage(
            "stage-1",
            "采集数据源",
            "以只读方式打开数据源并采集介质基础信息。",
        ),
        stage(
            "stage-2",
            "元数据扫描",
            "解析文件系统元数据并定位已删除条目。",
        ),
    ];

    if input.include_carving {
        stages.push(stage(
            "stage-3",
            "签名雕刻",
            "在未分配区域中扫描已知文件签名。",
        ));
    }

    stages.push(stage(
        "stage-4",
        "导出恢复",
        "将恢复数据导出到与源盘不同的目标介质。",
    ));

    let mut assumptions = vec![
        "程序不会向源介质写入任何数据。".to_string(),
        "恢复文件会导出到其他磁盘。".to_string(),
    ];

    match input.target_kind {
        TargetKind::Phone => {
            assumptions.push("手机恢复通常需要逻辑备份导出或物理镜像。".to_string());
            assumptions.push("锁定或加密设备可能阻断删除数据恢复。".to_string());
        }
        TargetKind::Auto | TargetKind::PcDisk | TargetKind::UsbDisk | TargetKind::Other => {
            assumptions.push("SSD 设备上的 TRIM 可能永久擦除已删除数据块。".to_string());
        }
    }

    let safety_rules = vec![
        "始终以只读方式挂载源介质。".to_string(),
        "禁止将恢复数据写回源卷。".to_string(),
        "保留案件清单与操作日志以便追溯。".to_string(),
    ];

    ScanPlan {
        case_id: input.case_id.clone(),
        target_kind: input.target_kind,
        depth: input.depth,
        fs_hint: input.fs_hint,
        stages,
        safety_rules,
        assumptions,
    }
}

pub fn execute_scan(request: &ScanRequest) -> RecoveryResult<ScanReport> {
    let total_steps = if request.include_carving { 5 } else { 4 };
    let progress = build_scan_progress_bar(total_steps);
    progress.set_message("准备扫描任务");

    let plan = build_plan(&request.plan);
    progress.inc(1);
    progress.set_message("读取设备与卷信息");

    let inspector = LocalDeviceInspector;
    let device_snapshot = inspector.inspect(request)?;
    let effective_source = device_snapshot
        .low_level_source_path
        .as_ref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| request.source.clone());
    let report_source = effective_source
        .canonicalize()
        .unwrap_or_else(|_| effective_source.clone());

    let mut scan_request = request.clone();
    scan_request.source = effective_source;
    progress.inc(1);
    progress.set_message("扫描文件系统元数据");

    let fs_scanner = HeuristicFsScanner;
    let fs_result = fs_scanner.scan(&scan_request, &device_snapshot);
    progress.inc(1);
    progress.set_message(if request.include_carving {
        "执行签名特征扫描"
    } else {
        "汇总扫描结果"
    });

    let carver = SignatureCarver;
    let carve_result = carver.carve(&scan_request);
    progress.inc(1);
    progress.set_message("生成扫描报告");

    let mut findings = Vec::new();
    findings.extend(fs_result.items.clone());
    findings.extend(carve_result.items.clone());

    let warnings = build_warnings(request, &device_snapshot, findings.is_empty());
    progress.inc(1);
    progress.finish_with_message("扫描完成，正在输出摘要");

    Ok(ScanReport {
        generated_at: Utc::now().to_rfc3339(),
        plan,
        source: report_source.display().to_string(),
        device_snapshot,
        fs_result,
        carve_result,
        findings,
        warnings,
    })
}

fn build_warnings(
    request: &ScanRequest,
    snapshot: &DeviceSnapshot,
    no_findings: bool,
) -> Vec<String> {
    let mut warnings = Vec::new();
    let effective_kind = snapshot.detected_target_kind.unwrap_or(request.target_kind);
    let using_low_level_source = snapshot.low_level_source_path.is_some();

    if effective_kind == TargetKind::Phone
        && snapshot.source_type == "mounted-path"
        && !using_low_level_source
    {
        warnings
            .push("当前为手机目录模式，通常只能覆盖逻辑文件，难以命中底层删除数据块。".to_string());
    }

    if effective_kind != TargetKind::Phone
        && snapshot.source_type == "mounted-path"
        && !using_low_level_source
    {
        warnings.push(
            "当前为目录扫描模式，覆盖范围有限；建议使用原始磁盘镜像以提升恢复覆盖率。".to_string(),
        );
    }

    if no_findings {
        warnings.push("本次未发现可恢复候选项。建议使用原始镜像并启用签名雕刻后重试。".to_string());
    }

    warnings
}

fn build_scan_progress_bar(total_steps: u64) -> ProgressBar {
    let bar = ProgressBar::new(total_steps);
    let refresh_hz = config::settings().ui.progress_refresh_hz.max(1);
    bar.set_draw_target(ProgressDrawTarget::stdout_with_hz(refresh_hz));
    let spinner_frames = scan_spinner_frames();
    let style = ProgressStyle::with_template(scan_progress_template())
        .or_else(|_| ProgressStyle::with_template(DEFAULT_SCAN_PROGRESS_TEMPLATE))
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars(scan_progress_chars())
        .tick_strings(&spinner_frames);
    bar.set_style(style);
    bar.set_prefix(scan_progress_prefix());
    let tick_ms = config::settings().ui.progress_tick_ms.max(10);
    bar.enable_steady_tick(Duration::from_millis(tick_ms));
    bar
}

fn scan_progress_template() -> &'static str {
    let value = config::settings().ui.scan_progress_template.trim();
    if value.is_empty() {
        DEFAULT_SCAN_PROGRESS_TEMPLATE
    } else {
        value
    }
}

fn scan_progress_chars() -> &'static str {
    let value = config::settings().ui.progress_chars.trim();
    if value.chars().count() < 2 {
        DEFAULT_PROGRESS_CHARS
    } else {
        value
    }
}

fn scan_progress_prefix() -> &'static str {
    let value = config::settings().ui.scan_progress_prefix.trim();
    if value.is_empty() {
        DEFAULT_SCAN_PROGRESS_PREFIX
    } else {
        value
    }
}

fn scan_spinner_frames() -> Vec<&'static str> {
    let frames = config::settings()
        .ui
        .spinner_frames
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();

    if frames.len() < 2 {
        DEFAULT_SPINNER_FRAMES.to_vec()
    } else {
        frames
    }
}

fn stage(id: &str, title: &str, detail: &str) -> PlanStage {
    PlanStage {
        id: id.to_string(),
        title: title.to_string(),
        detail: detail.to_string(),
    }
}
