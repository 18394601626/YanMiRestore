//! NTFS 扫描模块：解析 MFT 删除记录并提取可恢复数据段。

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::config;
use crate::model::{NtfsDataSummary, NtfsScanOutput, RecoverableItem, ScanDepth, SourceSegment};

const NTFS_OEM_ID: &[u8; 8] = b"NTFS    ";
const MFT_RECORD_SIGNATURE: &[u8; 4] = b"FILE";
const ATTR_TYPE_FILE_NAME: u32 = 0x30;
const ATTR_TYPE_DATA: u32 = 0x80;
const ATTR_TYPE_END: u32 = 0xFFFF_FFFF;
const ATTR_FLAG_COMPRESSED: u16 = 0x0001;
const ATTR_FLAG_ENCRYPTED: u16 = 0x4000;

fn max_ntfs_findings() -> usize {
    config::settings().fs.ntfs.max_findings.max(1)
}

fn max_mft_records(depth: ScanDepth) -> usize {
    match depth {
        ScanDepth::Metadata => config::settings().fs.ntfs.max_mft_records_metadata.max(1),
        ScanDepth::Deep => config::settings().fs.ntfs.max_mft_records_deep.max(1),
    }
}

#[derive(Debug, Clone, Copy)]
struct NtfsBootInfo {
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    cluster_size: u32,
    total_sectors: u64,
    mft_lcn: u64,
    record_size: u32,
}

#[derive(Debug, Clone)]
struct DataInfo {
    size_bytes: Option<u64>,
    segments: Vec<SourceSegment>,
    compressed: bool,
    encrypted: bool,
    runlist_failed: bool,
    sparse: bool,
}

pub fn has_ntfs_signature_at(source: &Path, volume_offset: u64) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(source)?;
    let mut boot = [0_u8; 512];
    read_exact_at(&mut file, volume_offset, &mut boot)?;
    Ok(boot.get(3..11) == Some(NTFS_OEM_ID))
}

pub fn scan_deleted_mft_entries_at(
    source: &Path,
    depth: ScanDepth,
    notes: &mut Vec<String>,
    volume_offset: u64,
    volume_label: &str,
) -> NtfsScanOutput {
    let mut file = match std::fs::File::open(source) {
        Ok(value) => value,
        Err(error) => {
            notes.push(format!(
                "NTFS 扫描 [{} @ {}]：无法打开源文件：{}",
                volume_label, volume_offset, error
            ));
            return NtfsScanOutput::default();
        }
    };

    let mut boot = [0_u8; 512];
    if let Err(error) = read_exact_at(&mut file, volume_offset, &mut boot) {
        notes.push(format!(
            "NTFS 扫描 [{} @ {}]：无法读取引导扇区：{}",
            volume_label, volume_offset, error
        ));
        return NtfsScanOutput::default();
    }

    if boot.get(3..11) != Some(NTFS_OEM_ID) {
        notes.push(format!(
            "NTFS 扫描 [{} @ {}]：未识别为 NTFS。",
            volume_label, volume_offset
        ));
        return NtfsScanOutput::default();
    }

    let Some(boot_info) = parse_boot_info(&boot) else {
        notes.push(format!(
            "NTFS 扫描 [{} @ {}]：引导扇区字段无效。",
            volume_label, volume_offset
        ));
        return NtfsScanOutput::default();
    };

    let file_size = file
        .metadata()
        .ok()
        .map(|value| value.len())
        .filter(|value| *value > 0)
        .or_else(|| {
            boot_info
                .total_sectors
                .checked_mul(u64::from(boot_info.bytes_per_sector))
        });
    let Some(file_size) = file_size else {
        notes.push(format!(
            "NTFS 扫描 [{} @ {}]：无法获取源容量。",
            volume_label, volume_offset
        ));
        return NtfsScanOutput::default();
    };

    let Some(mft_relative_offset) = boot_info
        .mft_lcn
        .checked_mul(u64::from(boot_info.cluster_size))
    else {
        notes.push(format!(
            "NTFS 扫描 [{} @ {}]：MFT 偏移溢出。",
            volume_label, volume_offset
        ));
        return NtfsScanOutput::default();
    };
    let Some(mft_offset) = volume_offset.checked_add(mft_relative_offset) else {
        notes.push(format!(
            "NTFS 扫描 [{} @ {}]：MFT 绝对偏移溢出。",
            volume_label, volume_offset
        ));
        return NtfsScanOutput::default();
    };
    if mft_offset >= file_size {
        notes.push(format!(
            "NTFS 扫描 [{} @ {}]：MFT 偏移 {} 超出容量 {}。",
            volume_label, volume_offset, mft_offset, file_size
        ));
        return NtfsScanOutput::default();
    }

    let record_size = u64::from(boot_info.record_size);
    let max_records_by_size = (file_size - mft_offset) / record_size;
    if max_records_by_size == 0 {
        notes.push("NTFS 扫描终止：没有足够空间读取 MFT 记录。".to_string());
        return NtfsScanOutput::default();
    }

    let scan_cap = max_mft_records(depth);
    let records_to_scan = std::cmp::min(max_records_by_size as usize, scan_cap);
    notes.push(format!(
        "NTFS 引导信息 [{} @ {}]：每扇区字节={}，每簇扇区={}，簇大小={}，总扇区={}，MFT LCN={}，记录大小={}",
        volume_label,
        volume_offset,
        boot_info.bytes_per_sector,
        boot_info.sectors_per_cluster,
        boot_info.cluster_size,
        boot_info.total_sectors,
        boot_info.mft_lcn,
        boot_info.record_size
    ));
    notes.push(format!(
        "NTFS MFT 扫描 [{} @ {}]：从偏移 {} 扫描 {} 条记录。",
        volume_label, volume_offset, mft_offset, records_to_scan
    ));

    let mut items = Vec::new();
    let mut summary = NtfsDataSummary::default();
    let mut record = vec![0_u8; boot_info.record_size as usize];

    for index in 0..records_to_scan {
        let record_offset = mft_offset + (index as u64 * record_size);
        if read_exact_at(&mut file, record_offset, &mut record).is_err() {
            continue;
        }

        if record.get(0..4) != Some(MFT_RECORD_SIGNATURE) {
            continue;
        }

        let Some((item, sparse)) = parse_deleted_record(
            &record,
            index as u64,
            record_offset,
            &boot_info,
            volume_offset,
        ) else {
            continue;
        };

        if item.source_segments.is_empty() {
            summary.metadata_only += 1;
        } else {
            summary.recoverable += 1;
            if sparse {
                summary.recoverable_with_sparse += 1;
            }
        }

        if item.note.contains("压缩") {
            summary.unsupported_compressed += 1;
        }
        if item.note.contains("加密") {
            summary.unsupported_encrypted += 1;
        }
        if item.note.contains("运行列表解析失败") {
            summary.runlist_failed += 1;
        }

        items.push(item);
        let max_findings = max_ntfs_findings();
        if items.len() >= max_findings {
            notes.push(format!(
                "NTFS 扫描 [{} @ {}]：结果已截断到 {} 条。",
                volume_label, volume_offset, max_findings
            ));
            break;
        }
    }

    notes.push(format!(
        "NTFS MFT 扫描完成 [{} @ {}]：发现 {} 个删除候选项。",
        volume_label,
        volume_offset,
        items.len()
    ));

    NtfsScanOutput { items, summary }
}

fn parse_boot_info(boot: &[u8]) -> Option<NtfsBootInfo> {
    if boot.len() < 80 || boot.get(3..11) != Some(NTFS_OEM_ID) {
        return None;
    }

    let bytes_per_sector = read_u16(boot, 11)?;
    let sectors_per_cluster = *boot.get(13)?;
    if bytes_per_sector == 0 || sectors_per_cluster == 0 {
        return None;
    }

    let cluster_size = u32::from(bytes_per_sector) * u32::from(sectors_per_cluster);
    if cluster_size == 0 {
        return None;
    }

    let total_sectors = read_u64(boot, 40)?;
    let mft_lcn = read_u64(boot, 48)?;
    let clusters_per_record = *boot.get(64)? as i8;
    let record_size = if clusters_per_record > 0 {
        cluster_size.checked_mul(clusters_per_record as u32)?
    } else if clusters_per_record < 0 {
        let shift = u32::from((-clusters_per_record) as u8);
        if shift >= 31 {
            return None;
        }
        1_u32.checked_shl(shift)?
    } else {
        return None;
    };
    if !(256..=65_536).contains(&record_size) {
        return None;
    }

    Some(NtfsBootInfo {
        bytes_per_sector,
        sectors_per_cluster,
        cluster_size,
        total_sectors,
        mft_lcn,
        record_size,
    })
}

fn parse_deleted_record(
    record: &[u8],
    record_number: u64,
    record_offset: u64,
    boot: &NtfsBootInfo,
    volume_offset: u64,
) -> Option<(RecoverableItem, bool)> {
    if record.len() < 64 || record.get(0..4) != Some(MFT_RECORD_SIGNATURE) {
        return None;
    }

    let flags = read_u16(record, 22)?;
    if flags & 0x0001 != 0 {
        return None;
    }
    let is_directory = flags & 0x0002 != 0;

    let first_attr = read_u16(record, 20)? as usize;
    if first_attr >= record.len() {
        return None;
    }

    let mut cursor = first_attr;
    let mut name: Option<String> = None;
    let mut file_size: Option<u64> = None;
    let mut data_info = DataInfo {
        size_bytes: None,
        segments: Vec::new(),
        compressed: false,
        encrypted: false,
        runlist_failed: false,
        sparse: false,
    };

    while cursor + 16 <= record.len() {
        let attr_type = read_u32(record, cursor)?;
        if attr_type == ATTR_TYPE_END {
            break;
        }
        let attr_len = read_u32(record, cursor + 4)? as usize;
        if attr_len < 16 || cursor + attr_len > record.len() {
            break;
        }

        if attr_type == ATTR_TYPE_FILE_NAME {
            if let Some((parsed_name, parsed_size)) = parse_file_name_attr(record, cursor, attr_len)
            {
                if name.is_none() {
                    name = Some(parsed_name);
                }
                if file_size.is_none() && parsed_size > 0 {
                    file_size = Some(parsed_size);
                }
            }
        } else if attr_type == ATTR_TYPE_DATA {
            parse_data_attr(
                record,
                cursor,
                attr_len,
                boot,
                volume_offset,
                &mut data_info,
            );
        }

        cursor += attr_len;
    }

    let suggested_name = name.unwrap_or_else(|| format!("mft_record_{}.bin", record_number));
    let mut note = if is_directory {
        format!("在 MFT 记录 {} 中发现已删除目录。", record_number)
    } else {
        format!("在 MFT 记录 {} 中发现已删除文件。", record_number)
    };
    if data_info.compressed {
        note.push_str(" 检测到压缩流。");
    }
    if data_info.encrypted {
        note.push_str(" 检测到加密流。");
    }
    if data_info.runlist_failed {
        note.push_str(" 运行列表解析失败。");
    }

    let category = if is_directory {
        "ntfs-mft-deleted-directory"
    } else {
        "ntfs-mft-deleted-file"
    };
    let first_offset = data_info
        .segments
        .iter()
        .find(|segment| !segment.sparse)
        .map(|segment| segment.offset);

    let size_bytes = data_info.size_bytes.or(file_size);
    let confidence = if data_info.segments.is_empty() {
        0.56
    } else {
        0.82
    };
    let item = RecoverableItem {
        id: format!("ntfs-mft-{record_number:08}"),
        category: category.to_string(),
        confidence,
        note,
        suggested_name,
        source_path: None,
        source_offset: first_offset,
        size_bytes,
        source_segments: data_info.segments.clone(),
    };

    let sparse = data_info.sparse;
    if item.category.ends_with("file") || item.category.ends_with("directory") {
        let _ = record_offset;
        return Some((item, sparse));
    }
    None
}

fn parse_file_name_attr(
    record: &[u8],
    attr_offset: usize,
    attr_len: usize,
) -> Option<(String, u64)> {
    let non_resident = *record.get(attr_offset + 8)?;
    if non_resident != 0 {
        return None;
    }

    let content_len = read_u32(record, attr_offset + 16)? as usize;
    let content_offset = read_u16(record, attr_offset + 20)? as usize;
    let content_start = attr_offset.checked_add(content_offset)?;
    let content_end = content_start.checked_add(content_len)?;
    let attr_end = attr_offset.checked_add(attr_len)?;
    if content_end > record.len() || content_end > attr_end || content_len < 66 {
        return None;
    }

    let name_len = *record.get(content_start + 64)? as usize;
    let name_start = content_start + 66;
    let name_end = name_start.checked_add(name_len.checked_mul(2)?)?;
    if name_end > content_end {
        return None;
    }

    let name = decode_utf16le(record.get(name_start..name_end)?)?;
    let size = read_u64(record, content_start + 48).unwrap_or(0);
    Some((name, size))
}

fn parse_data_attr(
    record: &[u8],
    attr_offset: usize,
    attr_len: usize,
    boot: &NtfsBootInfo,
    volume_offset: u64,
    data_info: &mut DataInfo,
) {
    let Some(non_resident) = record.get(attr_offset + 8).copied() else {
        return;
    };
    let flags = read_u16(record, attr_offset + 12).unwrap_or(0);
    data_info.compressed |= (flags & ATTR_FLAG_COMPRESSED) != 0;
    data_info.encrypted |= (flags & ATTR_FLAG_ENCRYPTED) != 0;

    if non_resident == 0 {
        let size = read_u32(record, attr_offset + 16).map(u64::from);
        if data_info.size_bytes.is_none() {
            data_info.size_bytes = size;
        }
        return;
    }

    let run_offset = read_u16(record, attr_offset + 32)
        .map(usize::from)
        .unwrap_or(0);
    let real_size = read_u64(record, attr_offset + 48);
    if data_info.size_bytes.is_none() {
        data_info.size_bytes = real_size;
    }

    let run_start = match attr_offset.checked_add(run_offset) {
        Some(value) => value,
        None => {
            data_info.runlist_failed = true;
            return;
        }
    };
    let attr_end = match attr_offset.checked_add(attr_len) {
        Some(value) => value,
        None => {
            data_info.runlist_failed = true;
            return;
        }
    };
    if run_start >= attr_end || attr_end > record.len() {
        data_info.runlist_failed = true;
        return;
    }

    match parse_data_runs(
        &record[run_start..attr_end],
        u64::from(boot.cluster_size),
        volume_offset,
    ) {
        Some(segments) => {
            data_info.sparse |= segments.iter().any(|segment| segment.sparse);
            data_info.segments.extend(segments);
        }
        None => data_info.runlist_failed = true,
    }
}

fn parse_data_runs(
    data: &[u8],
    cluster_size: u64,
    volume_offset: u64,
) -> Option<Vec<SourceSegment>> {
    let mut out = Vec::new();
    let mut cursor = 0_usize;
    let mut current_lcn: i64 = 0;

    while cursor < data.len() {
        let header = *data.get(cursor)?;
        cursor += 1;
        if header == 0 {
            break;
        }

        let len_size = (header & 0x0F) as usize;
        let off_size = (header >> 4) as usize;
        if len_size == 0 || len_size > 8 || off_size > 8 {
            return None;
        }

        let len_end = cursor.checked_add(len_size)?;
        let len_bytes = data.get(cursor..len_end)?;
        cursor = len_end;
        let mut clusters = 0_u64;
        for (idx, byte) in len_bytes.iter().enumerate() {
            clusters |= u64::from(*byte) << (idx * 8);
        }
        if clusters == 0 {
            return None;
        }

        let offset_delta = if off_size == 0 {
            0_i64
        } else {
            let off_end = cursor.checked_add(off_size)?;
            let off_bytes = data.get(cursor..off_end)?;
            cursor = off_end;

            let mut value: i64 = 0;
            for (idx, byte) in off_bytes.iter().enumerate() {
                value |= i64::from(*byte) << (idx * 8);
            }
            // 补符号位。
            let sign_bit = 1_i64 << ((off_size * 8) - 1);
            if value & sign_bit != 0 {
                let mask = !0_i64 << (off_size * 8);
                value |= mask;
            }
            value
        };

        if off_size == 0 {
            out.push(SourceSegment {
                offset: 0,
                length: clusters.checked_mul(cluster_size)?,
                sparse: true,
            });
            continue;
        }

        current_lcn = current_lcn.checked_add(offset_delta)?;
        if current_lcn < 0 {
            return None;
        }

        let absolute =
            volume_offset.checked_add((current_lcn as u64).checked_mul(cluster_size)?)?;
        out.push(SourceSegment {
            offset: absolute,
            length: clusters.checked_mul(cluster_size)?,
            sparse: false,
        });
    }

    Some(out)
}

fn decode_utf16le(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() % 2 != 0 {
        return None;
    }

    let mut words = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        words.push(u16::from_le_bytes([pair[0], pair[1]]));
    }
    let mut text = String::from_utf16_lossy(&words);
    text.retain(|ch| ch != '\0');
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn read_exact_at(file: &mut std::fs::File, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buf)
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(data: &[u8], offset: usize) -> Option<u64> {
    let bytes = data.get(offset..offset + 8)?;
    Some(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

#[cfg(test)]
mod tests {
    use super::{has_ntfs_signature_at, parse_data_runs};

    #[test]
    fn parse_data_runs_handles_fragmented_and_sparse_segments() {
        let runs = [0x11, 0x03, 0x0A, 0x01, 0x04, 0x11, 0x02, 0x02, 0x00];
        let segments = parse_data_runs(&runs, 4096, 1024).expect("parse runs");
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].offset, 1024 + (10 * 4096));
        assert_eq!(segments[0].length, 3 * 4096);
        assert!(!segments[0].sparse);
        assert_eq!(segments[1].offset, 0);
        assert_eq!(segments[1].length, 4 * 4096);
        assert!(segments[1].sparse);
        assert_eq!(segments[2].offset, 1024 + (12 * 4096));
        assert_eq!(segments[2].length, 2 * 4096);
        assert!(!segments[2].sparse);
    }

    #[test]
    fn detects_ntfs_signature() {
        let path = std::env::temp_dir().join("ntfs-signature-test.img");
        let mut boot = [0_u8; 512];
        boot[3..11].copy_from_slice(b"NTFS    ");
        std::fs::write(&path, boot).expect("write temp file");

        let detected = has_ntfs_signature_at(&path, 0).expect("detect signature");
        assert!(detected);
        let _ = std::fs::remove_file(path);
    }
}
