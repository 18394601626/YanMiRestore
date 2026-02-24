//! 报告输出模块：负责计划、扫描与恢复结果的终端展示。
use std::path::{Path, PathBuf};

use crate::error::RecoveryResult;
use crate::model::{FsHint, FsMetrics, RecoverySession, ScanDepth, ScanPlan, ScanReport};
use crate::utils::label::target_kind_label;

pub fn write_scan_report(report: &ScanReport, output_dir: &Path) -> RecoveryResult<PathBuf> {
    std::fs::create_dir_all(output_dir)?;

    let file_name = format!(
        "{}-scan-report.json",
        sanitize_case_id(&report.plan.case_id)
    );
    let output_path = output_dir.join(file_name);
    let body = serde_json::to_string_pretty(report)?;

    std::fs::write(&output_path, body)?;
    Ok(output_path)
}

pub fn print_plan(plan: &ScanPlan) {
    println!("=== 恢复方案 ===");
    println!("案件编号：{}", plan.case_id);
    println!("目标类型：{}", target_kind_label(plan.target_kind));
    println!("扫描深度：{}", scan_depth_label(plan.depth));
    println!("文件系统提示：{}", fs_hint_label(plan.fs_hint));
    println!();
    println!("执行阶段：");
    for (index, item) in plan.stages.iter().enumerate() {
        println!("{}. {}: {}", index + 1, item.title, item.detail);
    }
    println!();
    println!("安全规则：");
    for item in &plan.safety_rules {
        println!("- {item}");
    }
    println!();
    println!("前置假设：");
    for item in &plan.assumptions {
        println!("- {item}");
    }
}

pub fn print_recovery_session(session: &RecoverySession) {
    let recovered = session
        .actions
        .iter()
        .filter(|item| item.status == "成功")
        .count();
    let failed = session
        .actions
        .iter()
        .filter(|item| item.status == "失败")
        .count();

    println!("=== 恢复结果 ===");
    println!("案件编号：{}", session.case_id);
    println!(
        "运行模式：{}",
        if session.dry_run { "预演" } else { "执行" }
    );
    println!("输出目录：{}", session.destination);
    println!("候选总数：{}", session.action_count);
    println!("成功数量：{}", recovered);
    println!("失败数量：{}", failed);
    println!("清单路径：{}", session.manifest_path);
    if !session.notes.is_empty() {
        println!("补充说明：");
        for note in &session.notes {
            println!("- {note}");
        }
    }
}

pub fn print_scan_summary(report: &ScanReport) {
    println!("=== 扫描结果 ===");
    println!("案件编号：{}", report.plan.case_id);
    println!("数据来源：{}", report.source);
    println!(
        "候选项总数：{}（文件系统：{}，签名雕刻：{}）",
        report.findings.len(),
        report.fs_result.items.len(),
        report.carve_result.items.len()
    );
    println!(
        "识别到的文件系统：{}",
        report
            .fs_result
            .detected_fs
            .as_deref()
            .map(detected_fs_label)
            .unwrap_or_else(|| "未知".to_string())
    );
    if let Some(kind) = report.device_snapshot.detected_target_kind {
        println!("识别到的设备类型：{}", target_kind_label(kind));
    }
    if let Some(hint) = &report.device_snapshot.device_hint {
        println!("设备识别依据：{hint}");
    }
    if let Some(raw) = &report.device_snapshot.low_level_source_path {
        println!("底层扫描路径：{raw}");
    }
    if !report.device_snapshot.notes.is_empty() {
        println!("设备说明：");
        for note in &report.device_snapshot.notes {
            println!("- {note}");
        }
    }

    if let Some(metrics) = &report.fs_result.metrics {
        print_fs_metrics(metrics);
    }

    if !report.warnings.is_empty() {
        println!("风险提示：");
        for warning in &report.warnings {
            println!("- {warning}");
        }
    }
}

fn print_fs_metrics(metrics: &FsMetrics) {
    if let Some(ntfs) = &metrics.ntfs {
        println!(
            "NTFS 统计：可恢复={} | 仅元数据={} | 压缩={} | 加密={} | 压缩且加密={} | 运行列表失败={} | 含稀疏段可恢复={}",
            ntfs.recoverable,
            ntfs.metadata_only,
            ntfs.unsupported_compressed,
            ntfs.unsupported_encrypted,
            ntfs.unsupported_compressed_encrypted,
            ntfs.runlist_failed,
            ntfs.recoverable_with_sparse
        );
    }

    if let Some(fat) = &metrics.fat {
        println!(
            "FAT 统计：卷数={} | FAT12={} | FAT16={} | FAT32={} | exFAT={} | 删除文件={} | 删除目录={} | 可分段恢复={} | 仅元数据={}",
            fat.volumes_scanned,
            fat.fat12_volumes,
            fat.fat16_volumes,
            fat.fat32_volumes,
            fat.exfat_volumes,
            fat.deleted_files,
            fat.deleted_directories,
            fat.with_recovery_segments,
            fat.metadata_only
        );
    }

    if let Some(ext4) = &metrics.ext4 {
        println!(
            "ext4 统计：卷数={} | 删除文件={} | 删除目录={} | 可分段恢复={} | 稀疏段={} | 仅元数据={} | 深度不支持={} | 旧指针文件={}",
            ext4.volumes_scanned,
            ext4.deleted_files,
            ext4.deleted_directories,
            ext4.with_recovery_segments,
            ext4.with_sparse_segments,
            ext4.metadata_only,
            ext4.extents_depth_unsupported,
            ext4.legacy_pointer_files
        );
    }
}

fn scan_depth_label(depth: ScanDepth) -> &'static str {
    match depth {
        ScanDepth::Metadata => "元数据扫描",
        ScanDepth::Deep => "深度扫描",
    }
}

fn fs_hint_label(hint: FsHint) -> &'static str {
    match hint {
        FsHint::Auto => "自动识别",
        FsHint::Ntfs => "NTFS",
        FsHint::Fat32 => "FAT32",
        FsHint::Exfat => "exFAT",
        FsHint::Ext4 => "ext4",
        FsHint::Apfs => "APFS",
        FsHint::F2fs => "F2FS",
    }
}

fn detected_fs_label(value: &str) -> String {
    if let Some(rest) = value.strip_prefix("mixed-") {
        let names = rest
            .split('-')
            .filter_map(|part| match part {
                "ntfs" => Some("NTFS"),
                "fat" => Some("FAT"),
                "family" => Some("exFAT"),
                "ext4" => Some("ext4"),
                "apfs" => Some("APFS"),
                "f2fs" => Some("F2FS"),
                _ => None,
            })
            .collect::<Vec<_>>();

        if names.is_empty() {
            return "混合（未知）".to_string();
        }
        return format!("混合（{}）", names.join(" + "));
    }

    match value {
        "ntfs" => "NTFS".to_string(),
        "fat32" => "FAT32".to_string(),
        "exfat" => "exFAT".to_string(),
        "fat-family" => "FAT/exFAT".to_string(),
        "ext4" => "ext4".to_string(),
        "apfs" => "APFS".to_string(),
        "f2fs" => "F2FS".to_string(),
        other => format!("未知（{other}）"),
    }
}

fn sanitize_case_id(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    if sanitized.is_empty() {
        "case".to_string()
    } else {
        sanitized
    }
}
