//! 恢复执行模块：按扫描报告导出目标文件并写入清单。

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use filetime::{set_file_times, FileTime};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::config;
use crate::error::RecoveryError;
use crate::error::RecoveryResult;
use crate::model::{
    RecoverableItem, RecoveryAction, RecoveryRequest, RecoverySession, ScanReport, SourceSegment,
};

const DEFAULT_RECOVER_PROGRESS_TEMPLATE: &str =
    "{spinner:.green} {prefix:.bold} [{elapsed_precise}] [{bar:32.yellow/blue}] {pos}/{len} {percent:>3}% {msg}";
const DEFAULT_PROGRESS_CHARS: &str = "=>-";
const DEFAULT_RECOVER_PROGRESS_PREFIX: &str = "恢复";
const DEFAULT_SPINNER_FRAMES: [&str; 4] = ["|", "/", "-", "\\"];

fn copy_buffer_size() -> usize {
    config::settings().recovery.copy_buffer_size.max(1024)
}

fn raw_io_alignment_bytes() -> u64 {
    config::settings().recovery.raw_io_alignment_bytes.max(1)
}

/// 读取扫描报告并执行恢复流程，最终返回恢复会话。
pub fn execute_recovery(request: &RecoveryRequest) -> RecoveryResult<RecoverySession> {
    let report_raw = std::fs::read_to_string(&request.report_path)?;
    let report: ScanReport = serde_json::from_str(&report_raw)?;
    let source_path = resolve_source_path(&report.source, &request.report_path);
    validate_destination_path(&source_path, &request.destination)?;

    std::fs::create_dir_all(&request.destination)?;

    let total = report.findings.len() as u64;
    let progress = build_recovery_progress_bar(total);

    let mut actions = Vec::new();
    for (index, item) in report.findings.iter().enumerate() {
        progress.set_message(format!(
            "正在处理 {}/{}：{}",
            index + 1,
            report.findings.len(),
            item.suggested_name
        ));
        let action = if request.dry_run {
            RecoveryAction {
                item_id: item.id.clone(),
                status: "计划".to_string(),
                note: "预演模式：不执行实际提取。".to_string(),
                output_path: None,
                bytes_written: item.size_bytes,
            }
        } else {
            recover_item(&source_path, &request.destination, item, request)
        };
        actions.push(action);
        progress.inc(1);
    }
    progress.finish_with_message(if request.dry_run {
        "恢复预演完成，未写入目标文件"
    } else {
        "恢复执行完成，文件已导出"
    });

    let recovered_count = actions
        .iter()
        .filter(|action| action.status == "成功")
        .count();
    let failed_count = actions
        .iter()
        .filter(|action| action.status == "失败")
        .count();

    let mut notes = vec![
        "本命令不会修改源介质。".to_string(),
        format!("成功恢复数量：{recovered_count}"),
    ];
    if failed_count > 0 {
        notes.push(format!("失败数量：{failed_count}"));
    }
    if request.dry_run {
        notes.push("当前为预演模式。".to_string());
    }
    notes.push(if request.keep_original_name {
        "输出文件名策略：优先使用原文件名，冲突时自动追加序号。".to_string()
    } else {
        "输出文件名策略：使用案件条目前缀命名。".to_string()
    });
    notes.push(if request.preserve_timestamps {
        "时间戳策略：可获取时同步原文件访问/修改时间。".to_string()
    } else {
        "时间戳策略：不额外同步，使用恢复写入时间。".to_string()
    });
    notes.push(if request.skip_carved {
        "候选过滤策略：已跳过签名雕刻候选项，仅恢复文件系统候选项。".to_string()
    } else {
        "候选过滤策略：恢复全部候选项（含签名雕刻）。".to_string()
    });
    if report.findings.is_empty() {
        notes.push("扫描报告中没有可恢复候选项。".to_string());
    }

    let manifest_path = request.destination.join("恢复清单.json");

    let session = RecoverySession {
        generated_at: Utc::now().to_rfc3339(),
        case_id: report.plan.case_id,
        destination: request.destination.display().to_string(),
        dry_run: request.dry_run,
        action_count: actions.len() as u64,
        actions,
        notes,
        manifest_path: manifest_path.display().to_string(),
    };

    std::fs::write(&manifest_path, serde_json::to_string_pretty(&session)?)?;
    Ok(session)
}

/// 创建恢复阶段使用的动态进度条。
fn build_recovery_progress_bar(total: u64) -> ProgressBar {
    let bar = ProgressBar::new(total);
    let refresh_hz = config::settings().ui.progress_refresh_hz.max(1);
    bar.set_draw_target(ProgressDrawTarget::stdout_with_hz(refresh_hz));
    let spinner_frames = recover_spinner_frames();
    let style = ProgressStyle::with_template(recover_progress_template())
        .or_else(|_| ProgressStyle::with_template(DEFAULT_RECOVER_PROGRESS_TEMPLATE))
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars(recover_progress_chars())
        .tick_strings(&spinner_frames);
    bar.set_style(style);
    bar.set_prefix(recover_progress_prefix());
    let tick_ms = config::settings().ui.progress_tick_ms.max(10);
    bar.enable_steady_tick(Duration::from_millis(tick_ms));
    if total == 0 {
        bar.set_message("当前无可恢复条目");
    } else {
        bar.set_message("准备恢复任务");
    }
    bar
}

fn recover_progress_template() -> &'static str {
    let value = config::settings().ui.recover_progress_template.trim();
    if value.is_empty() {
        DEFAULT_RECOVER_PROGRESS_TEMPLATE
    } else {
        value
    }
}

fn recover_progress_chars() -> &'static str {
    let value = config::settings().ui.progress_chars.trim();
    if value.chars().count() < 2 {
        DEFAULT_PROGRESS_CHARS
    } else {
        value
    }
}

fn recover_progress_prefix() -> &'static str {
    let value = config::settings().ui.recover_progress_prefix.trim();
    if value.is_empty() {
        DEFAULT_RECOVER_PROGRESS_PREFIX
    } else {
        value
    }
}

fn recover_spinner_frames() -> Vec<&'static str> {
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

/// 根据候选项类型分派具体恢复策略。
fn recover_item(
    source_path: &Path,
    destination: &Path,
    item: &RecoverableItem,
    request: &RecoveryRequest,
) -> RecoveryAction {
    if request.skip_carved && item.category == "signature-carved" {
        return RecoveryAction {
            item_id: item.id.clone(),
            status: "跳过".to_string(),
            note: "按参数要求跳过签名雕刻候选项。".to_string(),
            output_path: None,
            bytes_written: None,
        };
    }

    if !item.source_segments.is_empty() {
        return recover_segment_list(source_path, destination, item, request);
    }

    if item.category.starts_with("ntfs-mft-deleted-") {
        return RecoveryAction {
            item_id: item.id.clone(),
            status: "跳过".to_string(),
            note: format!(
                "该 NTFS 条目仅有元数据，当前版本无法恢复数据段。扫描详情：{}",
                item.note
            ),
            output_path: None,
            bytes_written: None,
        };
    }

    if item.category.starts_with("ext4-deleted-") {
        return RecoveryAction {
            item_id: item.id.clone(),
            status: "跳过".to_string(),
            note: format!("该 ext4 条目缺少可恢复数据段。扫描详情：{}", item.note),
            output_path: None,
            bytes_written: None,
        };
    }

    if let Some(logical_path) = &item.source_path {
        return recover_logical_file(logical_path, destination, item, request);
    }

    if let (Some(offset), Some(size)) = (item.source_offset, item.size_bytes) {
        return recover_carved_segment(source_path, destination, item, offset, size, request);
    }

    RecoveryAction {
        item_id: item.id.clone(),
        status: "跳过".to_string(),
        note: "缺少恢复坐标（path/offset/size）。".to_string(),
        output_path: None,
        bytes_written: None,
    }
}

/// 按分段坐标拼接恢复文件。
fn recover_segment_list(
    source_path: &Path,
    destination: &Path,
    item: &RecoverableItem,
    request: &RecoveryRequest,
) -> RecoveryAction {
    let target_path = build_output_path(destination, item, request.keep_original_name);
    match extract_segments(
        source_path,
        &target_path,
        &item.source_segments,
        item.size_bytes,
    ) {
        Ok(bytes_written) if request.preserve_timestamps => RecoveryAction {
            item_id: item.id.clone(),
            status: "成功".to_string(),
            note: "已按分段源区间完成恢复。该条目来自底层数据段，缺少可用原始时间戳。".to_string(),
            output_path: Some(target_path.display().to_string()),
            bytes_written: Some(bytes_written),
        },
        Ok(bytes_written) => RecoveryAction {
            item_id: item.id.clone(),
            status: "成功".to_string(),
            note: "已按分段源区间完成恢复。".to_string(),
            output_path: Some(target_path.display().to_string()),
            bytes_written: Some(bytes_written),
        },
        Err(error) => RecoveryAction {
            item_id: item.id.clone(),
            status: "失败".to_string(),
            note: format!("分段提取失败：{error}"),
            output_path: Some(target_path.display().to_string()),
            bytes_written: None,
        },
    }
}

/// 从逻辑路径复制文件到恢复目录。
fn recover_logical_file(
    source_file: &str,
    destination: &Path,
    item: &RecoverableItem,
    request: &RecoveryRequest,
) -> RecoveryAction {
    let source = PathBuf::from(source_file);
    if !source.exists() {
        return RecoveryAction {
            item_id: item.id.clone(),
            status: "失败".to_string(),
            note: format!("未找到源逻辑文件：{}", source.display()),
            output_path: None,
            bytes_written: None,
        };
    }

    let target_path = build_output_path(destination, item, request.keep_original_name);
    match std::fs::copy(&source, &target_path) {
        Ok(bytes_written) if request.preserve_timestamps => {
            let note = match sync_file_timestamps(&source, &target_path) {
                Ok(()) => "已从回收站/垃圾桶路径复制逻辑删除文件，并同步原始时间戳。".to_string(),
                Err(error) => format!(
                    "已从回收站/垃圾桶路径复制逻辑删除文件，但同步时间戳失败：{}",
                    error
                ),
            };
            RecoveryAction {
                item_id: item.id.clone(),
                status: "成功".to_string(),
                note,
                output_path: Some(target_path.display().to_string()),
                bytes_written: Some(bytes_written),
            }
        }
        Ok(bytes_written) => RecoveryAction {
            item_id: item.id.clone(),
            status: "成功".to_string(),
            note: "已从回收站/垃圾桶路径复制逻辑删除文件。".to_string(),
            output_path: Some(target_path.display().to_string()),
            bytes_written: Some(bytes_written),
        },
        Err(error) => RecoveryAction {
            item_id: item.id.clone(),
            status: "失败".to_string(),
            note: format!("复制失败：{error}"),
            output_path: Some(target_path.display().to_string()),
            bytes_written: None,
        },
    }
}

/// 按偏移与长度从源介质提取连续数据。
fn recover_carved_segment(
    image_path: &Path,
    destination: &Path,
    item: &RecoverableItem,
    offset: u64,
    size: u64,
    request: &RecoveryRequest,
) -> RecoveryAction {
    let target_path = build_output_path(destination, item, request.keep_original_name);
    match extract_range(image_path, &target_path, offset, size) {
        Ok(bytes_written) if request.preserve_timestamps => RecoveryAction {
            item_id: item.id.clone(),
            status: "成功".to_string(),
            note: format!(
                "已从偏移 {} 提取雕刻数据。该条目来自底层数据段，缺少可用原始时间戳。",
                offset
            ),
            output_path: Some(target_path.display().to_string()),
            bytes_written: Some(bytes_written),
        },
        Ok(bytes_written) => RecoveryAction {
            item_id: item.id.clone(),
            status: "成功".to_string(),
            note: format!("已从偏移 {} 提取雕刻数据。", offset),
            output_path: Some(target_path.display().to_string()),
            bytes_written: Some(bytes_written),
        },
        Err(error) => RecoveryAction {
            item_id: item.id.clone(),
            status: "失败".to_string(),
            note: format!("雕刻提取失败：{error}"),
            output_path: Some(target_path.display().to_string()),
            bytes_written: None,
        },
    }
}

/// 从源数据中按偏移提取连续字节区间。
fn extract_range(
    source_path: &Path,
    target_path: &Path,
    offset: u64,
    size: u64,
) -> std::io::Result<u64> {
    let mut input = std::fs::File::open(source_path)?;
    let mut output = std::fs::File::create(target_path)?;
    copy_range_from_input(&mut input, source_path, &mut output, offset, size)
}

/// 按多个分段顺序提取并拼接恢复文件。
fn extract_segments(
    source_path: &Path,
    target_path: &Path,
    segments: &[SourceSegment],
    requested_size: Option<u64>,
) -> std::io::Result<u64> {
    let total_capacity = segments.iter().try_fold(0_u64, |acc, segment| {
        acc.checked_add(segment.length)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "分段长度发生溢出"))
    })?;
    let mut remaining = requested_size.unwrap_or(total_capacity);

    let mut output = std::fs::File::create(target_path)?;
    let mut input = if segments.iter().any(|segment| !segment.sparse) {
        Some(std::fs::File::open(source_path)?)
    } else {
        None
    };

    let mut written = 0_u64;
    let zero_buffer = vec![0_u8; copy_buffer_size()];

    for segment in segments {
        if remaining == 0 {
            break;
        }

        let segment_size = std::cmp::min(segment.length, remaining);

        if segment.sparse {
            let mut segment_remaining = segment_size;
            while segment_remaining > 0 {
                let chunk = std::cmp::min(segment_remaining as usize, zero_buffer.len());
                output.write_all(&zero_buffer[..chunk])?;
                segment_remaining -= chunk as u64;
                remaining -= chunk as u64;
                written += chunk as u64;
            }
            continue;
        }

        let input_file = input.as_mut().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "非稀疏段恢复需要提供源镜像",
            )
        })?;

        let copied = copy_range_from_input(
            input_file,
            source_path,
            &mut output,
            segment.offset,
            segment_size,
        )?;
        remaining -= copied;
        written += copied;
    }

    if let Some(expected) = requested_size {
        if written < expected {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!(
                    "请求从分段恢复 {} 字节，但仅写入 {} 字节",
                    expected, written
                ),
            ));
        }
    }

    Ok(written)
}

/// 根据源路径类型选择合适的读取方式。
fn copy_range_from_input(
    input: &mut std::fs::File,
    source_path: &Path,
    output: &mut std::fs::File,
    offset: u64,
    size: u64,
) -> std::io::Result<u64> {
    if size == 0 {
        return Ok(0);
    }

    if requires_windows_raw_alignment(source_path) {
        copy_range_aligned(input, output, offset, size, raw_io_alignment_bytes())
    } else {
        copy_range_direct(input, output, offset, size)
    }
}

/// 普通文件模式下的顺序读取。
fn copy_range_direct(
    input: &mut std::fs::File,
    output: &mut std::fs::File,
    offset: u64,
    size: u64,
) -> std::io::Result<u64> {
    input.seek(SeekFrom::Start(offset))?;

    let mut remaining = size;
    let mut written = 0_u64;
    let mut buffer = vec![0_u8; copy_buffer_size()];

    while remaining > 0 {
        let read_size = std::cmp::min(remaining as usize, buffer.len());
        let read_count = input.read(&mut buffer[..read_size])?;
        if read_count == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!(
                    "请求从偏移 {} 读取 {} 字节，但实际仅可读取 {} 字节",
                    offset, size, written
                ),
            ));
        }
        output.write_all(&buffer[..read_count])?;
        remaining -= read_count as u64;
        written += read_count as u64;
    }

    Ok(written)
}

/// 原始卷模式下按扇区对齐读取并回写目标文件。
fn copy_range_aligned(
    input: &mut std::fs::File,
    output: &mut std::fs::File,
    offset: u64,
    size: u64,
    alignment: u64,
) -> std::io::Result<u64> {
    if alignment == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "原始卷对齐读取的对齐值不能为 0",
        ));
    }
    if !alignment.is_power_of_two() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "原始卷对齐值必须是 2 的幂",
        ));
    }

    let copy_buffer_size = copy_buffer_size();
    let alignment_usize = usize::try_from(alignment)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "原始卷对齐值过大"))?;
    let max_required = alignment_usize
        .checked_add(copy_buffer_size)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "对齐缓存长度溢出"))?;
    let mut aligned_buffer = vec![0_u8; max_required];

    let mut current_offset = offset;
    let mut remaining = size;
    let mut written = 0_u64;

    while remaining > 0 {
        let chunk_u64 = std::cmp::min(remaining, copy_buffer_size as u64);
        let chunk = usize::try_from(chunk_u64).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "读取块长度转换失败")
        })?;

        let aligned_start = (current_offset / alignment) * alignment;
        let skip = usize::try_from(current_offset - aligned_start).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "读取偏移转换失败")
        })?;
        let required = skip
            .checked_add(chunk)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "读取长度溢出"))?;
        let aligned_len = align_up_usize(required, alignment_usize)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "对齐长度溢出"))?;

        input.seek(SeekFrom::Start(aligned_start))?;
        input
            .read_exact(&mut aligned_buffer[..aligned_len])
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::UnexpectedEof {
                    std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        format!(
                            "原始卷对齐读取在偏移 {} 提前结束（请求读取 {} 字节）",
                            aligned_start, aligned_len
                        ),
                    )
                } else {
                    error
                }
            })?;

        let end = skip
            .checked_add(chunk)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "写入范围溢出"))?;
        output.write_all(&aligned_buffer[skip..end])?;

        current_offset = current_offset.saturating_add(chunk as u64);
        remaining -= chunk as u64;
        written += chunk as u64;
    }

    Ok(written)
}

/// 向上对齐到指定边界。
fn align_up_usize(value: usize, alignment: usize) -> Option<usize> {
    if alignment == 0 {
        return None;
    }
    let remainder = value % alignment;
    if remainder == 0 {
        Some(value)
    } else {
        value.checked_add(alignment - remainder)
    }
}

/// 判断路径是否需要原始卷对齐读取。
fn requires_windows_raw_alignment(path: &Path) -> bool {
    #[cfg(windows)]
    {
        let raw = path.as_os_str().to_string_lossy();
        drive_letter_from_device_path(&raw).is_some()
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// 解析报告中的源路径，支持相对路径回退。
fn resolve_source_path(source: &str, report_path: &Path) -> PathBuf {
    let source_path = PathBuf::from(source);
    if source_path.is_absolute() {
        return source_path;
    }

    let base_dir = report_path.parent().unwrap_or_else(|| Path::new("."));
    let from_report_dir = base_dir.join(&source_path);
    if from_report_dir.exists() {
        return from_report_dir;
    }

    let from_cwd = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(&source_path);
    if from_cwd.exists() {
        return from_cwd;
    }

    from_report_dir
}

/// 根据命名策略生成输出路径，并自动处理重名。
fn build_output_path(
    destination: &Path,
    item: &RecoverableItem,
    keep_original_name: bool,
) -> PathBuf {
    let clean_name = sanitize_name(&item.suggested_name);
    let prefixed_name = format!("{}_{}", sanitize_name(&item.id), clean_name);
    let preferred = if keep_original_name {
        clean_name
    } else {
        prefixed_name
    };
    build_unique_path(destination, &preferred)
}

/// 校验目标目录安全性，避免写回源盘。
fn validate_destination_path(source_path: &Path, destination: &Path) -> RecoveryResult<()> {
    let source_abs = source_path
        .canonicalize()
        .unwrap_or_else(|_| source_path.to_path_buf());
    let destination_abs = absolute_path(destination)?;

    if destination_abs.starts_with(&source_abs) {
        return Err(RecoveryError::UnsafeDestination(format!(
            "目标路径 {} 位于源路径 {} 内部",
            destination_abs.display(),
            source_abs.display()
        )));
    }

    if source_abs.is_dir() && source_abs.starts_with(&destination_abs) {
        return Err(RecoveryError::UnsafeDestination(format!(
            "目标路径 {} 是源目录 {} 的上级目录",
            destination_abs.display(),
            source_abs.display()
        )));
    }

    if source_abs == destination_abs {
        return Err(RecoveryError::UnsafeDestination(format!(
            "目标路径 {} 与源路径相同",
            destination_abs.display()
        )));
    }

    if let (Some(source_volume), Some(destination_volume)) = (
        windows_volume_key(&source_abs),
        windows_volume_key(&destination_abs),
    ) {
        if source_volume == destination_volume {
            return Err(RecoveryError::UnsafeDestination(format!(
                "源路径（{}）与目标路径（{}）位于同一 Windows 卷",
                source_abs.display(),
                destination_abs.display()
            )));
        }
    }

    Ok(())
}

/// 获取路径的绝对形式（不存在时保留拼接结果）。
fn absolute_path(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };

    if absolute.exists() {
        absolute.canonicalize()
    } else {
        Ok(absolute)
    }
}

/// 提取 Windows 卷标识用于同卷判定。
fn windows_volume_key(path: &Path) -> Option<String> {
    let raw_text = path.as_os_str().to_string_lossy();
    if let Some(letter) = drive_letter_from_device_path(&raw_text) {
        return Some(format!("{letter}:"));
    }

    path.components().find_map(|component| {
        if let Component::Prefix(prefix) = component {
            Some(prefix.as_os_str().to_string_lossy().to_ascii_lowercase())
        } else {
            None
        }
    })
}

/// 从 Windows 设备路径中提取盘符前缀。
fn drive_letter_from_device_path(value: &str) -> Option<char> {
    let text = value.to_ascii_lowercase();
    let chars: Vec<char> = text.chars().collect();
    if chars.len() >= 6
        && chars[0] == '\\'
        && chars[1] == '\\'
        && (chars[2] == '.' || chars[2] == '?')
        && chars[3] == '\\'
        && chars[4].is_ascii_alphabetic()
        && chars[5] == ':'
    {
        return Some(chars[4].to_ascii_lowercase());
    }

    None
}

/// 清洗文件名，移除 Windows 非法字符并规避保留设备名。
fn sanitize_name(name: &str) -> String {
    let normalized = name
        .chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            {
                '_'
            } else {
                ch
            }
        })
        .collect::<String>();
    let normalized = normalized.trim().trim_end_matches([' ', '.']).to_string();

    if normalized.is_empty() {
        return "item.bin".to_string();
    }

    if is_windows_reserved_name(&normalized) {
        format!("_{}", normalized)
    } else {
        normalized
    }
}

/// 按名称冲突追加编号，尽量保持扩展名不变。
fn build_unique_path(destination: &Path, preferred_name: &str) -> PathBuf {
    let mut path = destination.join(preferred_name);
    if !path.exists() {
        return path;
    }

    let (stem, ext) = split_file_name(preferred_name);
    let mut counter = 1_u32;
    loop {
        let name = if let Some(ext) = &ext {
            format!("{stem}_{counter}.{ext}")
        } else {
            format!("{stem}_{counter}")
        };
        path = destination.join(name);
        if !path.exists() {
            return path;
        }
        counter = counter.saturating_add(1);
    }
}

/// 拆分文件名主干与扩展名。
fn split_file_name(name: &str) -> (String, Option<String>) {
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(|value| {
            if value.is_empty() {
                "item".to_string()
            } else {
                value.to_string()
            }
        })
        .unwrap_or_else(|| "item".to_string());
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string());
    (stem, ext)
}

/// 检测 Windows 保留设备名。
fn is_windows_reserved_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// 将源文件访问/修改时间同步到目标文件。
fn sync_file_timestamps(source: &Path, target: &Path) -> std::io::Result<()> {
    let metadata = source.metadata()?;
    let accessed = FileTime::from_last_access_time(&metadata);
    let modified = FileTime::from_last_modification_time(&metadata);
    set_file_times(target, accessed, modified)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Seek, SeekFrom, Write};

    use super::{
        align_up_usize, build_unique_path, copy_range_aligned, drive_letter_from_device_path,
        sanitize_name, validate_destination_path,
    };

    #[test]
    /// 目标目录位于源目录内部时应被拒绝。
    fn reject_destination_inside_source() {
        let base = std::env::temp_dir().join(format!("recover-path-test-{}", std::process::id()));
        let source = base.join("source");
        let destination = source.join("restore");
        std::fs::create_dir_all(&destination).expect("create dirs");

        let result = validate_destination_path(&source, &destination);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(windows)]
    #[test]
    /// Windows 下源与目标同卷时应被拒绝。
    fn reject_destination_on_same_windows_volume() {
        let base = std::env::temp_dir().join(format!("recover-vol-test-{}", std::process::id()));
        let source = base.join("source");
        let destination = base.join("restore");
        std::fs::create_dir_all(&source).expect("create source");
        std::fs::create_dir_all(&destination).expect("create destination");

        let result = validate_destination_path(&source, &destination);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    /// 验证对齐计算逻辑。
    fn align_up_works() {
        assert_eq!(align_up_usize(1024, 512), Some(1024));
        assert_eq!(align_up_usize(1025, 512), Some(1536));
        assert_eq!(align_up_usize(0, 512), Some(0));
    }

    #[test]
    /// 验证未对齐范围读取可正确提取数据。
    fn aligned_copy_supports_unaligned_window() {
        let base = std::env::temp_dir().join(format!("recover-align-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("create base dir");

        let source = base.join("source.bin");
        let target = base.join("target.bin");

        let payload: Vec<u8> = (0..20000).map(|v| (v % 251) as u8).collect();
        let mut source_file = std::fs::File::create(&source).expect("create source");
        source_file.write_all(&payload).expect("write source");
        drop(source_file);

        let mut input = std::fs::File::open(&source).expect("open source");
        let mut output = std::fs::File::create(&target).expect("create target");
        let written = copy_range_aligned(&mut input, &mut output, 333, 4097, 512).expect("copy");
        assert_eq!(written, 4097);
        drop(output);

        let mut restored = vec![0_u8; 4097];
        let mut target_file = std::fs::File::open(&target).expect("open target");
        target_file.seek(SeekFrom::Start(0)).expect("seek target");
        target_file.read_exact(&mut restored).expect("read target");
        assert_eq!(&restored[..], &payload[333..(333 + 4097)]);

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    /// 验证设备路径盘符解析。
    fn parse_drive_letter_from_device_path() {
        assert_eq!(drive_letter_from_device_path(r"\\.\F:"), Some('f'));
        assert_eq!(drive_letter_from_device_path(r"\\?\g:"), Some('g'));
        assert_eq!(drive_letter_from_device_path(r"F:\"), None);
        assert_eq!(drive_letter_from_device_path(r"C:\data"), None);
    }

    #[test]
    /// 验证文件名清洗策略。
    fn sanitize_name_keeps_chinese_and_filters_invalid_chars() {
        assert_eq!(
            sanitize_name("微信图片_2026:02:21?.jpg"),
            "微信图片_2026_02_21_.jpg"
        );
        assert_eq!(sanitize_name("  报表 .xlsx "), "报表 .xlsx");
        assert_eq!(sanitize_name("CON"), "_CON");
    }

    #[test]
    /// 验证冲突文件名生成逻辑。
    fn build_unique_path_preserves_extension() {
        let base = std::env::temp_dir().join(format!("recover-name-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("create base dir");

        let first = base.join("照片.jpg");
        std::fs::write(&first, b"a").expect("write first");
        let second = build_unique_path(&base, "照片.jpg");
        assert!(second.ends_with("照片_1.jpg"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
