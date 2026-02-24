//! 文件系统扫描总调度：协调 NTFS/FAT/ext4/APFS/F2FS 探测。

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::config;
use crate::model::{
    DeviceSnapshot, Ext4DataSummary, FatDataSummary, FsHint, FsMetrics, FsScanResult,
    NtfsDataSummary, RecoverableItem, ScanDepth, ScanRequest,
};

mod ext4;
mod fat;
mod ntfs;
mod partition;

fn max_logical_findings() -> usize {
    config::settings().scan.max_logical_findings.max(1)
}

/// 文件系统扫描接口。
pub trait FileSystemScanner {
    /// 对给定数据源执行文件系统扫描并返回结果。
    fn scan(&self, request: &ScanRequest, snapshot: &DeviceSnapshot) -> FsScanResult;
}

#[derive(Debug, Default)]
/// 默认扫描器实现：按启发式规则依次探测各文件系统。
pub struct HeuristicFsScanner;

impl FileSystemScanner for HeuristicFsScanner {
    /// 执行扫描主流程。
    fn scan(&self, request: &ScanRequest, _snapshot: &DeviceSnapshot) -> FsScanResult {
        let mut notes = vec![
            "APFS/F2FS 删除元数据解析尚未实现。".to_string(),
            "当前版本支持回收站逻辑枚举、NTFS/FAT/exFAT/ext4 删除扫描，以及 MBR/GPT（含扩展/逻辑分区链）偏移探测。".to_string(),
        ];
        let mut items = Vec::new();
        let mut ntfs_summary: Option<NtfsDataSummary> = None;
        let mut fat_summary: Option<FatDataSummary> = None;
        let mut ext4_summary: Option<Ext4DataSummary> = None;

        if request.depth == ScanDepth::Deep {
            notes.push("已启用深度模式：将扩大 NTFS/FAT/ext4 扫描范围。".to_string());
        }

        let mut ntfs_detected = false;
        let mut fat_detected = false;
        let mut ext4_detected = false;
        let mut apfs_detected = false;
        let mut f2fs_detected = false;
        let probe_ntfs = matches!(request.fs_hint, FsHint::Auto | FsHint::Ntfs);
        let probe_fat = matches!(
            request.fs_hint,
            FsHint::Auto | FsHint::Fat32 | FsHint::Exfat
        );
        let probe_ext4 = matches!(request.fs_hint, FsHint::Auto | FsHint::Ext4);
        let probe_apfs = matches!(request.fs_hint, FsHint::Auto | FsHint::Apfs);
        let probe_f2fs = matches!(request.fs_hint, FsHint::Auto | FsHint::F2fs);

        if request.source.is_dir() {
            items.extend(collect_logical_deleted_candidates(
                &request.source,
                &mut notes,
            ));
        } else {
            let ntfs_volumes = if probe_ntfs {
                discover_ntfs_volumes(request, &mut notes)
            } else {
                Vec::new()
            };
            ntfs_detected = !ntfs_volumes.is_empty();

            if ntfs_detected {
                for (volume_index, (offset, label)) in ntfs_volumes.iter().enumerate() {
                    let mut volume_output = ntfs::scan_deleted_mft_entries_at(
                        &request.source,
                        request.depth,
                        &mut notes,
                        *offset,
                        label,
                    );

                    if let Some(summary) = ntfs_summary.as_mut() {
                        summary.add_assign(&volume_output.summary);
                    } else {
                        ntfs_summary = Some(volume_output.summary.clone());
                    }

                    for item in &mut volume_output.items {
                        item.id = format!("ntfs-vol{volume_index:02}-{}", item.id);
                    }
                    items.extend(volume_output.items);
                }
            } else if probe_ntfs {
                notes.push("源文件未检测到 NTFS 卷，已跳过 MFT 扫描。".to_string());
            }

            let fat_volumes = if probe_fat {
                discover_fat_volumes(request, &mut notes)
            } else {
                Vec::new()
            };
            fat_detected = !fat_volumes.is_empty();

            if fat_detected {
                for (volume_index, (offset, label, kind)) in fat_volumes.iter().enumerate() {
                    let mut volume_items = fat::scan_deleted_entries_at(
                        &request.source,
                        request.depth,
                        &mut notes,
                        *offset,
                        label,
                    );

                    let volume_summary = summarize_fat_volume(&volume_items, *kind);
                    if let Some(summary) = fat_summary.as_mut() {
                        summary.add_assign(&volume_summary);
                    } else {
                        fat_summary = Some(volume_summary);
                    }

                    for item in &mut volume_items {
                        item.id = format!("fat-vol{volume_index:02}-{}", item.id);
                    }
                    items.extend(volume_items);
                }
            } else if probe_fat {
                notes.push("源文件未检测到 FAT/exFAT 卷，已跳过 FAT 扫描。".to_string());
            }

            let ext4_volumes = if probe_ext4 {
                discover_ext4_volumes(request, &mut notes)
            } else {
                Vec::new()
            };
            ext4_detected = !ext4_volumes.is_empty();

            if ext4_detected {
                for (volume_index, (offset, label)) in ext4_volumes.iter().enumerate() {
                    let mut volume_output = ext4::scan_deleted_inodes_at(
                        &request.source,
                        request.depth,
                        &mut notes,
                        *offset,
                        label,
                    );

                    if let Some(summary) = ext4_summary.as_mut() {
                        summary.add_assign(&volume_output.summary);
                    } else {
                        ext4_summary = Some(volume_output.summary.clone());
                    }

                    for item in &mut volume_output.items {
                        item.id = format!("ext4-vol{volume_index:02}-{}", item.id);
                    }
                    items.extend(volume_output.items);
                }
            } else if probe_ext4 {
                notes.push("源文件未检测到 ext4 卷，已跳过 ext4 inode 扫描。".to_string());
            }

            let apfs_volumes = if probe_apfs {
                discover_apfs_volumes(request, &mut notes)
            } else {
                Vec::new()
            };
            apfs_detected = !apfs_volumes.is_empty();
            if apfs_detected {
                notes.push(format!(
                    "检测到 {} 个 APFS 卷签名，但删除元数据解析尚未实现。",
                    apfs_volumes.len()
                ));
            } else if probe_apfs && request.fs_hint == FsHint::Apfs {
                notes.push("源文件未检测到 APFS 卷签名。".to_string());
            }

            let f2fs_volumes = if probe_f2fs {
                discover_f2fs_volumes(request, &mut notes)
            } else {
                Vec::new()
            };
            f2fs_detected = !f2fs_volumes.is_empty();
            if f2fs_detected {
                notes.push(format!(
                    "检测到 {} 个 F2FS 卷签名，但删除元数据解析尚未实现。",
                    f2fs_volumes.len()
                ));
            } else if probe_f2fs && request.fs_hint == FsHint::F2fs {
                notes.push("源文件未检测到 F2FS 卷签名。".to_string());
            }
        }

        let detected_fs = match request.fs_hint {
            FsHint::Auto => {
                let mut detected = Vec::new();
                if ntfs_detected {
                    detected.push("ntfs");
                }
                if fat_detected {
                    detected.push("fat-family");
                }
                if ext4_detected {
                    detected.push("ext4");
                }
                if apfs_detected {
                    detected.push("apfs");
                }
                if f2fs_detected {
                    detected.push("f2fs");
                }

                if detected.is_empty() {
                    detect_fs_from_path(&request.source)
                } else if detected.len() == 1 {
                    Some(detected[0].to_string())
                } else {
                    Some(format!("mixed-{}", detected.join("-")))
                }
            }
            hint => Some(format!("{hint:?}").to_ascii_lowercase()),
        };

        let ntfs_metrics = ntfs_summary.and_then(|summary| {
            if summary.is_zero() {
                None
            } else {
                Some(summary)
            }
        });
        let fat_metrics = fat_summary.and_then(|summary| {
            if summary.is_zero() {
                None
            } else {
                Some(summary)
            }
        });
        let ext4_metrics = ext4_summary.and_then(|summary| {
            if summary.is_zero() {
                None
            } else {
                Some(summary)
            }
        });
        let metrics = if ntfs_metrics.is_some() || fat_metrics.is_some() || ext4_metrics.is_some() {
            Some(FsMetrics {
                ntfs: ntfs_metrics,
                fat: fat_metrics,
                ext4: ext4_metrics,
            })
        } else {
            None
        };

        FsScanResult {
            detected_fs,
            deleted_entry_candidates: items.len() as u64,
            notes,
            items,
            metrics,
        }
    }
}

/// 发现候选对象并返回列表。
fn discover_ext4_volumes(request: &ScanRequest, notes: &mut Vec<String>) -> Vec<(u64, String)> {
    let mut volumes = Vec::new();
    let mut seen_offsets = HashSet::new();

    match ext4::has_ext4_signature_at(&request.source, 0) {
        Ok(true) => {
            seen_offsets.insert(0);
            volumes.push((0, "whole-image".to_string()));
        }
        Ok(false) => {}
        Err(error) => notes.push(format!("ext4 在偏移 0 的签名检测失败：{error}")),
    }

    let partitions = partition::discover_partitions(&request.source, notes);
    for candidate in partitions {
        if !seen_offsets.insert(candidate.offset) {
            continue;
        }

        match ext4::has_ext4_signature_at(&request.source, candidate.offset) {
            Ok(true) => {
                volumes.push((
                    candidate.offset,
                    format!(
                        "{}:{}@{}",
                        candidate.scheme, candidate.label, candidate.offset
                    ),
                ));
            }
            Ok(false) => {}
            Err(error) => notes.push(format!(
                "ext4 在分区 {}（偏移 {}）的签名检测失败：{}",
                candidate.label, candidate.offset, error
            )),
        }
    }

    volumes
}

/// 发现候选对象并返回列表。
fn discover_apfs_volumes(request: &ScanRequest, notes: &mut Vec<String>) -> Vec<(u64, String)> {
    let mut volumes = Vec::new();
    let mut seen_offsets = HashSet::new();

    match has_apfs_signature_at(&request.source, 0) {
        Ok(true) => {
            seen_offsets.insert(0);
            volumes.push((0, "whole-image".to_string()));
        }
        Ok(false) => {}
        Err(error) => notes.push(format!("APFS 在偏移 0 的签名检测失败：{error}")),
    }

    let partitions = partition::discover_partitions(&request.source, notes);
    for candidate in partitions {
        if !seen_offsets.insert(candidate.offset) {
            continue;
        }

        match has_apfs_signature_at(&request.source, candidate.offset) {
            Ok(true) => volumes.push((
                candidate.offset,
                format!(
                    "{}:{}@{}",
                    candidate.scheme, candidate.label, candidate.offset
                ),
            )),
            Ok(false) => {}
            Err(error) => notes.push(format!(
                "APFS 在分区 {}（偏移 {}）的签名检测失败：{}",
                candidate.label, candidate.offset, error
            )),
        }
    }

    volumes
}

/// 发现候选对象并返回列表。
fn discover_f2fs_volumes(request: &ScanRequest, notes: &mut Vec<String>) -> Vec<(u64, String)> {
    let mut volumes = Vec::new();
    let mut seen_offsets = HashSet::new();

    match has_f2fs_signature_at(&request.source, 0) {
        Ok(true) => {
            seen_offsets.insert(0);
            volumes.push((0, "whole-image".to_string()));
        }
        Ok(false) => {}
        Err(error) => notes.push(format!("F2FS 在偏移 0 的签名检测失败：{error}")),
    }

    let partitions = partition::discover_partitions(&request.source, notes);
    for candidate in partitions {
        if !seen_offsets.insert(candidate.offset) {
            continue;
        }

        match has_f2fs_signature_at(&request.source, candidate.offset) {
            Ok(true) => volumes.push((
                candidate.offset,
                format!(
                    "{}:{}@{}",
                    candidate.scheme, candidate.label, candidate.offset
                ),
            )),
            Ok(false) => {}
            Err(error) => notes.push(format!(
                "F2FS 在分区 {}（偏移 {}）的签名检测失败：{}",
                candidate.label, candidate.offset, error
            )),
        }
    }

    volumes
}

/// 判断目标特征是否存在。
fn has_apfs_signature_at(source: &Path, volume_offset: u64) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(source)?;
    let Some(offset) = volume_offset.checked_add(32) else {
        return Ok(false);
    };

    file.seek(SeekFrom::Start(offset))?;
    let mut signature = [0_u8; 4];
    if file.read_exact(&mut signature).is_err() {
        return Ok(false);
    }

    Ok(&signature == b"NXSB" || &signature == b"APSB")
}

/// 判断目标特征是否存在。
fn has_f2fs_signature_at(source: &Path, volume_offset: u64) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(source)?;
    let Some(offset) = volume_offset.checked_add(1024) else {
        return Ok(false);
    };

    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = [0_u8; 4];
    if file.read_exact(&mut bytes).is_err() {
        return Ok(false);
    }

    Ok(u32::from_le_bytes(bytes) == 0xF2F5_2010)
}

/// 内部辅助方法：summarize_fat_volume。
fn summarize_fat_volume(items: &[RecoverableItem], kind: fat::FatVolumeKind) -> FatDataSummary {
    let mut summary = FatDataSummary {
        volumes_scanned: 1,
        ..FatDataSummary::default()
    };

    match kind {
        fat::FatVolumeKind::Fat12 => summary.fat12_volumes = 1,
        fat::FatVolumeKind::Fat16 => summary.fat16_volumes = 1,
        fat::FatVolumeKind::Fat32 => summary.fat32_volumes = 1,
        fat::FatVolumeKind::ExFat => summary.exfat_volumes = 1,
    }

    for item in items {
        if item.category.contains("deleted-directory") {
            summary.deleted_directories += 1;
        } else {
            summary.deleted_files += 1;
        }

        if item.source_segments.is_empty() {
            summary.metadata_only += 1;
        } else {
            summary.with_recovery_segments += 1;
        }
    }

    summary
}

/// 发现候选对象并返回列表。
fn discover_ntfs_volumes(request: &ScanRequest, notes: &mut Vec<String>) -> Vec<(u64, String)> {
    let mut volumes = Vec::new();
    let mut seen_offsets = HashSet::new();

    // 说明：用于当前解析或测试步骤。
    match ntfs::has_ntfs_signature_at(&request.source, 0) {
        Ok(true) => {
            seen_offsets.insert(0);
            volumes.push((0, "whole-image".to_string()));
        }
        Ok(false) => {}
        Err(error) => notes.push(format!("NTFS 在偏移 0 的签名检测失败：{error}")),
    }

    let partitions = partition::discover_partitions(&request.source, notes);
    for candidate in partitions {
        if !seen_offsets.insert(candidate.offset) {
            continue;
        }

        let should_probe = match request.fs_hint {
            FsHint::Ntfs => true,
            FsHint::Auto => true,
            _ => false,
        };
        if !should_probe {
            continue;
        }

        match ntfs::has_ntfs_signature_at(&request.source, candidate.offset) {
            Ok(true) => {
                volumes.push((
                    candidate.offset,
                    format!(
                        "{}:{}@{}",
                        candidate.scheme, candidate.label, candidate.offset
                    ),
                ));
            }
            Ok(false) => {}
            Err(error) => notes.push(format!(
                "NTFS 在分区 {}（偏移 {}）的签名检测失败：{}",
                candidate.label, candidate.offset, error
            )),
        }
    }

    volumes
}

/// 发现候选对象并返回列表。
fn discover_fat_volumes(
    request: &ScanRequest,
    notes: &mut Vec<String>,
) -> Vec<(u64, String, fat::FatVolumeKind)> {
    let mut volumes = Vec::new();
    let mut seen_offsets = HashSet::new();

    // 说明：用于当前解析或测试步骤。
    match fat::detect_fat_signature_at(&request.source, 0) {
        Ok(Some(kind)) => {
            seen_offsets.insert(0);
            volumes.push((0, format!("whole-image:{kind:?}"), kind));
        }
        Ok(None) => {}
        Err(error) => notes.push(format!("FAT 在偏移 0 的签名检测失败：{error}")),
    }

    let partitions = partition::discover_partitions(&request.source, notes);
    for candidate in partitions {
        if !seen_offsets.insert(candidate.offset) {
            continue;
        }

        match fat::detect_fat_signature_at(&request.source, candidate.offset) {
            Ok(Some(kind)) => {
                volumes.push((
                    candidate.offset,
                    format!(
                        "{}:{}@{}:{kind:?}",
                        candidate.scheme, candidate.label, candidate.offset
                    ),
                    kind,
                ));
            }
            Ok(None) => {}
            Err(error) => notes.push(format!(
                "FAT 在分区 {}（偏移 {}）的签名检测失败：{}",
                candidate.label, candidate.offset, error
            )),
        }
    }

    volumes
}

/// 内部辅助方法：collect_logical_deleted_candidates。
fn collect_logical_deleted_candidates(
    source_root: &Path,
    notes: &mut Vec<String>,
) -> Vec<RecoverableItem> {
    let mut roots = Vec::new();
    for dir in &config::settings().scan.recycle_dirs {
        if dir.is_empty() {
            continue;
        }
        let candidate = source_root.join(dir);
        if candidate.exists() {
            roots.push(candidate);
        }
    }

    if roots.is_empty() {
        notes.push("在提供的源路径下未找到回收站/垃圾桶目录。".to_string());
        return Vec::new();
    }

    let mut stack: Vec<PathBuf> = roots;
    let mut findings = Vec::new();
    let mut visited_files: usize = 0;
    let max_findings = max_logical_findings();

    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) => {
                notes.push(format!("无法读取目录 {}：{error}", dir.display()));
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(value) => value,
                Err(error) => {
                    notes.push(format!("读取目录项失败：{error}"));
                    continue;
                }
            };

            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(value) => value,
                Err(error) => {
                    notes.push(format!("无法读取 {} 的文件类型：{error}", path.display()));
                    continue;
                }
            };

            if file_type.is_dir() {
                stack.push(path);
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            visited_files += 1;
            if findings.len() >= max_findings {
                continue;
            }

            let file_name = path
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown.bin".to_string());
            let resolved_source = path.canonicalize().unwrap_or_else(|_| path.clone());

            let size_bytes = std::fs::metadata(&path).ok().map(|metadata| metadata.len());

            findings.push(RecoverableItem {
                id: format!("logical-{visited_files:06}"),
                category: "logical-trash-file".to_string(),
                confidence: 0.72,
                note: "在回收站/垃圾桶目录中发现该文件。".to_string(),
                suggested_name: file_name,
                source_path: Some(resolved_source.display().to_string()),
                source_offset: None,
                size_bytes,
                source_segments: Vec::new(),
            });
        }
    }

    if visited_files > max_findings {
        notes.push(format!("逻辑扫描结果已截断为 {max_findings} 条。"));
    }

    findings
}

/// 判断目标特征是否存在。
fn detect_fs_from_path(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    let guess = match ext.as_str() {
        "img" | "raw" | "dd" => "unknown-image",
        "e01" => "ewf-image",
        "vhd" | "vhdx" => "virtual-disk",
        _ => return None,
    };

    Some(guess.to_string())
}

#[cfg(test)]
mod tests {
    use super::{has_apfs_signature_at, has_f2fs_signature_at, summarize_fat_volume};
    use crate::model::{RecoverableItem, SourceSegment};

    #[test]
    /// 汇总 FAT/exFAT 条目统计，验证卷类型与恢复模式计数。
    fn summarize_fat_volume_counts_types_and_recovery_modes() {
        let items = vec![
            RecoverableItem {
                id: "a".to_string(),
                category: "fat-deleted-file".to_string(),
                confidence: 0.8,
                note: String::new(),
                suggested_name: "a.bin".to_string(),
                source_path: None,
                source_offset: None,
                size_bytes: Some(10),
                source_segments: vec![SourceSegment {
                    offset: 100,
                    length: 10,
                    sparse: false,
                }],
            },
            RecoverableItem {
                id: "b".to_string(),
                category: "exfat-deleted-directory".to_string(),
                confidence: 0.6,
                note: String::new(),
                suggested_name: "dir".to_string(),
                source_path: None,
                source_offset: None,
                size_bytes: None,
                source_segments: Vec::new(),
            },
        ];

        let summary = summarize_fat_volume(&items, super::fat::FatVolumeKind::ExFat);
        assert_eq!(summary.volumes_scanned, 1);
        assert_eq!(summary.exfat_volumes, 1);
        assert_eq!(summary.deleted_files, 1);
        assert_eq!(summary.deleted_directories, 1);
        assert_eq!(summary.with_recovery_segments, 1);
        assert_eq!(summary.metadata_only, 1);
    }

    #[test]
    /// 判断目标特征是否存在。
    fn detects_apfs_signature() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!("apfs-detect-{}.img", std::process::id()));

        let mut image = vec![0_u8; 4 * 1024];
        image[32..36].copy_from_slice(b"NXSB");
        std::fs::write(&path, &image).expect("write image");

        assert!(has_apfs_signature_at(&path, 0).expect("check APFS"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    /// 判断目标特征是否存在。
    fn detects_f2fs_signature() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!("f2fs-detect-{}.img", std::process::id()));

        let mut image = vec![0_u8; 4 * 1024];
        image[1024..1028].copy_from_slice(&0xF2F5_2010_u32.to_le_bytes());
        std::fs::write(&path, &image).expect("write image");

        assert!(has_f2fs_signature_at(&path, 0).expect("check F2FS"));
        let _ = std::fs::remove_file(path);
    }
}
