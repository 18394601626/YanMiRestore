//! 分区解析模块：识别 MBR/GPT 以及扩展分区（EBR）链。

use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::config;
use crate::model::PartitionCandidate;
/// GPT 头签名字节。
const GPT_HEADER_SIGNATURE: &[u8; 8] = b"EFI PART";
/// GPT 中 Microsoft Basic Data 分区类型 GUID（小端字节序）。
const MICROSOFT_BASIC_DATA_GUID_LE: [u8; 16] = [
    0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7,
];
/// 扇区大小（字节）。
const SECTOR_SIZE: u64 = 512;

fn max_gpt_entries() -> usize {
    config::settings().partition.max_gpt_entries.max(1)
}

fn max_ebr_chain_entries() -> usize {
    config::settings().partition.max_ebr_chain_entries.max(1)
}

/// 发现候选对象并返回列表。
pub fn discover_partitions(source: &Path, notes: &mut Vec<String>) -> Vec<PartitionCandidate> {
    let mut file = match std::fs::File::open(source) {
        Ok(value) => value,
        Err(error) => {
            notes.push(format!("分区扫描：无法打开源文件：{error}"));
            return Vec::new();
        }
    };

    let file_size = match file.metadata() {
        Ok(value) => value.len(),
        Err(error) => {
            if is_windows_raw_device_path(source) {
                return Vec::new();
            }
            notes.push(format!("分区扫描：无法读取源文件元数据：{error}"));
            return Vec::new();
        }
    };

    if file_size < SECTOR_SIZE {
        notes.push("分区扫描：源文件小于 512 字节。".to_string());
        return Vec::new();
    }

    let mut mbr = [0_u8; 512];
    if let Err(error) = read_exact_at(&mut file, 0, &mut mbr) {
        notes.push(format!("分区扫描：无法读取 MBR 扇区：{error}"));
        return Vec::new();
    }

    let mut collected = Vec::new();

    let mbr_signature_ok = mbr[510] == 0x55 && mbr[511] == 0xAA;
    if mbr_signature_ok {
        let (mbr_candidates, has_protective) = parse_mbr_entries(&mut file, &mbr, file_size, notes);
        if !mbr_candidates.is_empty() {
            notes.push(format!(
                "分区扫描：检测到 {} 个 MBR 分区候选项。",
                mbr_candidates.len()
            ));
        }
        collected.extend(mbr_candidates);

        if has_protective {
            let gpt_candidates = parse_gpt_entries(&mut file, file_size, notes);
            if !gpt_candidates.is_empty() {
                notes.push(format!(
                    "分区扫描：检测到 {} 个 GPT 分区候选项。",
                    gpt_candidates.len()
                ));
            }
            collected.extend(gpt_candidates);
        }
    } else {
        notes.push("分区扫描：在 LBA0 未发现 MBR 签名。".to_string());

        // 在无有效 MBR 的情况下仍尝试解析 GPT（常见于部分镜像导出场景）。
        let gpt_candidates = parse_gpt_entries(&mut file, file_size, notes);
        if !gpt_candidates.is_empty() {
            notes.push(format!(
                "分区扫描：在无有效 MBR 情况下检测到 {} 个 GPT 分区候选项。",
                gpt_candidates.len()
            ));
        }
        collected.extend(gpt_candidates);
    }

    dedupe_partitions(collected)
}

/// 判断是否为 Windows 原始卷路径。
fn is_windows_raw_device_path(path: &Path) -> bool {
    let text = path.as_os_str().to_string_lossy().to_ascii_lowercase();
    let chars: Vec<char> = text.chars().collect();
    chars.len() >= 6
        && chars[0] == '\\'
        && chars[1] == '\\'
        && (chars[2] == '.' || chars[2] == '?')
        && chars[3] == '\\'
        && chars[4].is_ascii_alphabetic()
        && chars[5] == ':'
}

/// 解析原始字节并生成结构化数据。
fn parse_mbr_entries(
    file: &mut std::fs::File,
    mbr: &[u8; 512],
    file_size: u64,
    notes: &mut Vec<String>,
) -> (Vec<PartitionCandidate>, bool) {
    let mut candidates = Vec::new();
    let mut has_protective = false;

    for index in 0..4_usize {
        let entry_offset = 446 + (index * 16);
        let entry = &mbr[entry_offset..entry_offset + 16];
        let type_code = entry[4];
        let first_lba = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as u64;
        let sectors = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as u64;

        if type_code == 0 || sectors == 0 {
            continue;
        }

        if type_code == 0xEE {
            has_protective = true;
            continue;
        }

        if matches!(type_code, 0x05 | 0x0F | 0x85) {
            let logical_partitions =
                parse_ebr_chain(file, file_size, first_lba, sectors, index + 1, notes);
            if logical_partitions.is_empty() {
                notes.push(format!(
                    "分区扫描：MBR 表项 {} 为扩展分区（类型 0x{:02X}），但未解析出逻辑分区。",
                    index + 1,
                    type_code
                ));
            } else {
                notes.push(format!(
                    "分区扫描：MBR 表项 {} 扩展出 {} 个逻辑分区。",
                    index + 1,
                    logical_partitions.len()
                ));
            }
            candidates.extend(logical_partitions);
            continue;
        }

        let Some(offset) = first_lba.checked_mul(SECTOR_SIZE) else {
            notes.push(format!("分区扫描：MBR 表项 {} 偏移发生溢出。", index + 1));
            continue;
        };
        let Some(size) = sectors.checked_mul(SECTOR_SIZE) else {
            notes.push(format!("分区扫描：MBR 表项 {} 大小发生溢出。", index + 1));
            continue;
        };

        if offset >= file_size {
            notes.push(format!(
                "分区扫描：MBR 表项 {} 的偏移 {} 超出文件大小 {}。",
                index + 1,
                offset,
                file_size
            ));
            continue;
        }

        let kind = mbr_partition_type_name(type_code);
        candidates.push(PartitionCandidate {
            offset,
            size,
            label: format!("mbr-{}-{}", index + 1, kind),
            scheme: "MBR".to_string(),
        });
    }

    (candidates, has_protective)
}

/// 解析原始字节并生成结构化数据。
fn parse_ebr_chain(
    file: &mut std::fs::File,
    file_size: u64,
    extended_base_lba: u64,
    extended_sectors: u64,
    mbr_entry_index: usize,
    notes: &mut Vec<String>,
) -> Vec<PartitionCandidate> {
    let Some(base_offset) = extended_base_lba.checked_mul(SECTOR_SIZE) else {
        notes.push(format!(
            "分区扫描：来自 MBR 表项 {} 的扩展分区偏移发生溢出。",
            mbr_entry_index
        ));
        return Vec::new();
    };

    if base_offset >= file_size {
        notes.push(format!(
            "分区扫描：来自 MBR 表项 {} 的扩展分区起始位置超出源范围（偏移 {}，大小 {}）。",
            mbr_entry_index, base_offset, file_size
        ));
        return Vec::new();
    }

    let extended_last_lba = if extended_sectors > 0 {
        extended_base_lba
            .checked_add(extended_sectors)
            .and_then(|value| value.checked_sub(1))
    } else {
        None
    };

    let mut candidates = Vec::new();
    let mut visited_ebr_lbas = HashSet::new();
    let mut current_ebr_lba = extended_base_lba;
    let mut logical_index = 0_usize;

    for step in 0..max_ebr_chain_entries() {
        if !visited_ebr_lbas.insert(current_ebr_lba) {
            notes.push(format!(
                "分区扫描：在 MBR 表项 {} 的 LBA {} 处检测到 EBR 链循环。",
                mbr_entry_index, current_ebr_lba
            ));
            break;
        }

        let Some(ebr_offset) = current_ebr_lba.checked_mul(SECTOR_SIZE) else {
            notes.push(format!(
                "分区扫描：MBR 表项 {} 的 EBR {} 偏移发生溢出。",
                step + 1,
                mbr_entry_index
            ));
            break;
        };
        if ebr_offset >= file_size {
            notes.push(format!(
                "分区扫描：MBR 表项 {} 的 EBR {} 超出源范围（偏移 {}，大小 {}）。",
                step + 1,
                mbr_entry_index,
                ebr_offset,
                file_size
            ));
            break;
        }

        let mut ebr = [0_u8; 512];
        if let Err(error) = read_exact_at(file, ebr_offset, &mut ebr) {
            notes.push(format!(
                "分区扫描：无法读取 MBR 表项 {} 的 EBR {}：{}",
                step + 1,
                mbr_entry_index,
                error
            ));
            break;
        }

        if ebr[510] != 0x55 || ebr[511] != 0xAA {
            notes.push(format!(
                "分区扫描：MBR 表项 {} 的 EBR {} 签名无效。",
                step + 1,
                mbr_entry_index
            ));
            break;
        }

        let mut next_ebr_lba: Option<u64> = None;
        for entry_index in 0..4_usize {
            let entry_offset = 446 + (entry_index * 16);
            let entry = &ebr[entry_offset..entry_offset + 16];
            let type_code = entry[4];
            let rel_lba = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as u64;
            let sectors = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as u64;

            if type_code == 0 || sectors == 0 {
                continue;
            }

            if matches!(type_code, 0x05 | 0x0F | 0x85) {
                let candidate_lba = extended_base_lba.checked_add(rel_lba);
                if candidate_lba.is_none() {
                    notes.push(format!(
                        "分区扫描：MBR 表项 {} 的 EBR {} 下一跳链接发生溢出。",
                        step + 1,
                        mbr_entry_index
                    ));
                }
                next_ebr_lba = candidate_lba;
                continue;
            }

            let Some(start_lba) = current_ebr_lba.checked_add(rel_lba) else {
                notes.push(format!(
                    "分区扫描：EBR {} 逻辑分区表项 {} 起始位置溢出。",
                    step + 1,
                    entry_index + 1
                ));
                continue;
            };
            let Some(offset) = start_lba.checked_mul(SECTOR_SIZE) else {
                notes.push(format!(
                    "分区扫描：EBR {} 逻辑分区表项 {} 偏移溢出。",
                    step + 1,
                    entry_index + 1
                ));
                continue;
            };
            let Some(size) = sectors.checked_mul(SECTOR_SIZE) else {
                notes.push(format!(
                    "分区扫描：EBR {} 逻辑分区表项 {} 大小溢出。",
                    step + 1,
                    entry_index + 1
                ));
                continue;
            };

            if offset >= file_size {
                notes.push(format!(
                    "分区扫描：EBR {} 逻辑分区表项 {} 的偏移 {} 超出源大小 {}。",
                    step + 1,
                    entry_index + 1,
                    offset,
                    file_size
                ));
                continue;
            }

            if let Some(last_lba) = extended_last_lba {
                let Some(end_lba) = start_lba.checked_add(sectors.saturating_sub(1)) else {
                    notes.push(format!(
                        "分区扫描：EBR {} 逻辑分区表项 {} 结束位置溢出。",
                        step + 1,
                        entry_index + 1
                    ));
                    continue;
                };
                if start_lba < extended_base_lba || end_lba > last_lba {
                    notes.push(format!(
                        "分区扫描：EBR {} 逻辑分区表项 {} 超出扩展分区边界。",
                        step + 1,
                        entry_index + 1
                    ));
                    continue;
                }
            }

            logical_index += 1;
            candidates.push(PartitionCandidate {
                offset,
                size,
                label: format!(
                    "mbr-logical-{}-{}",
                    logical_index,
                    mbr_partition_type_name(type_code)
                ),
                scheme: "MBR".to_string(),
            });
        }

        let Some(next_lba) = next_ebr_lba else {
            break;
        };
        if next_lba == current_ebr_lba {
            notes.push(format!(
                "分区扫描：MBR 表项 {} 在 LBA {} 处出现 EBR 链自引用。",
                mbr_entry_index, current_ebr_lba
            ));
            break;
        }
        if let Some(last_lba) = extended_last_lba {
            if next_lba < extended_base_lba || next_lba > last_lba {
                notes.push(format!(
                    "分区扫描：MBR 表项 {} 的 EBR 下一跳 {} 超出扩展分区边界。",
                    next_lba, mbr_entry_index
                ));
                break;
            }
        }

        current_ebr_lba = next_lba;
    }

    if visited_ebr_lbas.len() == max_ebr_chain_entries() {
        notes.push(format!(
            "分区扫描：MBR 表项 {} 的 EBR 链达到上限 {}，结果已截断。",
            mbr_entry_index,
            max_ebr_chain_entries()
        ));
    }

    candidates
}

/// 解析原始字节并生成结构化数据。
fn parse_gpt_entries(
    file: &mut std::fs::File,
    file_size: u64,
    notes: &mut Vec<String>,
) -> Vec<PartitionCandidate> {
    let mut header = [0_u8; 512];
    if let Err(error) = read_exact_at(file, 512, &mut header) {
        notes.push(format!("分区扫描：无法读取 LBA1 的 GPT 头：{error}"));
        return Vec::new();
    }

    if header.get(0..8) != Some(GPT_HEADER_SIGNATURE) {
        return Vec::new();
    }

    let header_size = read_u32(&header, 12).unwrap_or(0) as usize;
    if header_size < 92 {
        notes.push("分区扫描：GPT 头大小无效。".to_string());
        return Vec::new();
    }

    let entry_lba = read_u64(&header, 72).unwrap_or(0);
    let entry_count = read_u32(&header, 80).unwrap_or(0) as usize;
    let entry_size = read_u32(&header, 84).unwrap_or(0) as usize;

    if entry_lba == 0 || entry_count == 0 || entry_size < 128 {
        notes.push("分区扫描：GPT 分区表字段无效。".to_string());
        return Vec::new();
    }

    let scan_count = std::cmp::min(entry_count, max_gpt_entries());
    if entry_count > scan_count {
        notes.push(format!(
            "分区扫描：GPT 分区项已从 {} 条截断到 {} 条。",
            entry_count, scan_count
        ));
    }

    let Some(entry_table_offset) = entry_lba.checked_mul(SECTOR_SIZE) else {
        notes.push("分区扫描：GPT 分区表偏移发生溢出。".to_string());
        return Vec::new();
    };

    let mut candidates = Vec::new();
    let mut entry_buf = vec![0_u8; entry_size];

    for index in 0..scan_count {
        let Some(entry_offset) = entry_table_offset.checked_add((index * entry_size) as u64) else {
            break;
        };
        if entry_offset >= file_size {
            break;
        }

        if let Err(error) = read_exact_at(file, entry_offset, &mut entry_buf) {
            notes.push(format!(
                "分区扫描：无法读取 GPT 分区项 {}：{}",
                index + 1,
                error
            ));
            continue;
        }

        let type_guid = &entry_buf[0..16];
        if type_guid.iter().all(|byte| *byte == 0) {
            continue;
        }

        let first_lba = read_u64(&entry_buf, 32).unwrap_or(0);
        let last_lba = read_u64(&entry_buf, 40).unwrap_or(0);
        if first_lba == 0 || last_lba < first_lba {
            continue;
        }

        let Some(offset) = first_lba.checked_mul(SECTOR_SIZE) else {
            continue;
        };
        let sectors = last_lba - first_lba + 1;
        let Some(size) = sectors.checked_mul(SECTOR_SIZE) else {
            continue;
        };
        if offset >= file_size {
            continue;
        }

        let name = decode_utf16_name(&entry_buf[56..]);
        let type_name = gpt_type_name(type_guid);
        let label = if !name.is_empty() {
            name
        } else {
            format!("gpt-{}-{}", index + 1, type_name)
        };

        candidates.push(PartitionCandidate {
            offset,
            size,
            label,
            scheme: "GPT".to_string(),
        });
    }

    candidates
}

/// 内部辅助方法：dedupe_partitions。
fn dedupe_partitions(candidates: Vec<PartitionCandidate>) -> Vec<PartitionCandidate> {
    let mut map: BTreeMap<u64, PartitionCandidate> = BTreeMap::new();

    for candidate in candidates {
        map.entry(candidate.offset)
            .and_modify(|existing| {
                if candidate.size > existing.size {
                    existing.size = candidate.size;
                }
                if !existing.label.contains(&candidate.label) {
                    existing.label = format!("{},{}", existing.label, candidate.label);
                }
                if existing.scheme != candidate.scheme {
                    existing.scheme = format!("{}/{}", existing.scheme, candidate.scheme);
                }
            })
            .or_insert(candidate);
    }

    map.into_values().collect()
}

/// 内部辅助方法：mbr_partition_type_name。
fn mbr_partition_type_name(type_code: u8) -> &'static str {
    match type_code {
        0x01 => "fat12",
        0x04 | 0x06 | 0x0E => "fat16",
        0x07 => "ntfs-exfat",
        0x0B | 0x0C => "fat32",
        0x27 => "win-recovery",
        0x82 => "linux-swap",
        0x83 => "linux",
        0xAF => "hfs-apfs",
        _ => "unknown",
    }
}

/// 内部辅助方法：gpt_type_name。
fn gpt_type_name(type_guid: &[u8]) -> &'static str {
    if type_guid == MICROSOFT_BASIC_DATA_GUID_LE {
        "ms-basic-data"
    } else {
        "unknown"
    }
}

/// 内部辅助方法：decode_utf16_name。
fn decode_utf16_name(bytes: &[u8]) -> String {
    let mut words = Vec::new();
    for pair in bytes.chunks_exact(2) {
        let value = u16::from_le_bytes([pair[0], pair[1]]);
        if value == 0 {
            break;
        }
        words.push(value);
    }
    String::from_utf16_lossy(&words).trim().to_string()
}

/// 从源数据中读取指定内容。
fn read_exact_at(file: &mut std::fs::File, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buf)
}

/// 从源数据中读取指定内容。
fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// 从源数据中读取指定内容。
fn read_u64(data: &[u8], offset: usize) -> Option<u64> {
    let bytes = data.get(offset..offset + 8)?;
    Some(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

#[cfg(test)]
mod tests {
    use super::discover_partitions;

    #[test]
    /// 发现候选对象并返回列表。
    fn discover_mbr_partition() {
        let path = std::env::temp_dir().join("partition-mbr-test.img");
        let mut image = vec![0_u8; 2 * 1024 * 1024];
        image[510] = 0x55;
        image[511] = 0xAA;

        // 用于当前解析或测试步骤。
        let entry = 446;
        image[entry + 4] = 0x07;
        write_u32(&mut image, entry + 8, 2048);
        write_u32(&mut image, entry + 12, 1000);

        std::fs::write(&path, &image).expect("write image");
        let mut notes = Vec::new();
        let partitions = discover_partitions(&path, &mut notes);
        let _ = std::fs::remove_file(&path);

        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].offset, 2048 * 512);
        assert!(partitions[0].label.contains("mbr-1"));
    }

    #[test]
    /// 发现候选对象并返回列表。
    fn discover_gpt_partition() {
        let path = std::env::temp_dir().join("partition-gpt-test.img");
        let mut image = vec![0_u8; 2 * 1024 * 1024];

        // 用于当前解析或测试步骤。
        image[510] = 0x55;
        image[511] = 0xAA;
        let mbr_entry = 446;
        image[mbr_entry + 4] = 0xEE;
        write_u32(&mut image, mbr_entry + 8, 1);
        write_u32(&mut image, mbr_entry + 12, 4096);

        // 用于当前解析或测试步骤。
        let header = 512;
        image[header..header + 8].copy_from_slice(b"EFI PART");
        write_u32(&mut image, header + 12, 92);
        write_u64(&mut image, header + 72, 2); // 用于当前解析或测试步骤。
        write_u32(&mut image, header + 80, 1); // 用于当前解析或测试步骤。
        write_u32(&mut image, header + 84, 128); // 用于当前解析或测试步骤。

        // 用于当前解析或测试步骤。
        let entry = 1024;
        let basic_guid = [
            0xA2, 0xA0, 0xD0, 0xEB, 0xE5, 0xB9, 0x33, 0x44, 0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26,
            0x99, 0xC7,
        ];
        image[entry..entry + 16].copy_from_slice(&basic_guid);
        write_u64(&mut image, entry + 32, 2048); // 用于当前解析或测试步骤。
        write_u64(&mut image, entry + 40, 4095); // 用于当前解析或测试步骤。
        let name = "Data";
        let utf16: Vec<u16> = name.encode_utf16().collect();
        for (idx, ch) in utf16.iter().enumerate() {
            let pos = entry + 56 + idx * 2;
            image[pos..pos + 2].copy_from_slice(&ch.to_le_bytes());
        }

        std::fs::write(&path, &image).expect("write image");
        let mut notes = Vec::new();
        let partitions = discover_partitions(&path, &mut notes);
        let _ = std::fs::remove_file(&path);

        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].offset, 2048 * 512);
        assert_eq!(partitions[0].scheme, "GPT");
        assert_eq!(partitions[0].label, "Data");
    }

    #[test]
    /// 发现候选对象并返回列表。
    fn discover_extended_logical_partitions() {
        let path = std::env::temp_dir().join("partition-ebr-test.img");
        let mut image = vec![0_u8; 6 * 1024 * 1024];

        // 用于当前解析或测试步骤。
        image[510] = 0x55;
        image[511] = 0xAA;
        let mbr_entry = 446;
        image[mbr_entry + 4] = 0x0F; // 用于当前解析或测试步骤。
        write_u32(&mut image, mbr_entry + 8, 2048); // 用于当前解析或测试步骤。
        write_u32(&mut image, mbr_entry + 12, 4096); // 用于当前解析或测试步骤。

        // 用于当前解析或测试步骤。
        let ebr1 = (2048 * 512) as usize;
        image[ebr1 + 510] = 0x55;
        image[ebr1 + 511] = 0xAA;
        let ebr1_logical = ebr1 + 446;
        image[ebr1_logical + 4] = 0x07; // 用于当前解析或测试步骤。
        write_u32(&mut image, ebr1_logical + 8, 63); // 用于当前解析或测试步骤。
        write_u32(&mut image, ebr1_logical + 12, 1000);
        let ebr1_next = ebr1 + 462;
        image[ebr1_next + 4] = 0x0F; // 用于当前解析或测试步骤。
        write_u32(&mut image, ebr1_next + 8, 2000); // 用于当前解析或测试步骤。
        write_u32(&mut image, ebr1_next + 12, 2000);

        // 用于当前解析或测试步骤。
        let ebr2_lba = 4048_u32;
        let ebr2 = (ebr2_lba as usize) * 512;
        image[ebr2 + 510] = 0x55;
        image[ebr2 + 511] = 0xAA;
        let ebr2_logical = ebr2 + 446;
        image[ebr2_logical + 4] = 0x0B; // 用于当前解析或测试步骤。
        write_u32(&mut image, ebr2_logical + 8, 63); // 用于当前解析或测试步骤。
        write_u32(&mut image, ebr2_logical + 12, 500);

        std::fs::write(&path, &image).expect("write image");
        let mut notes = Vec::new();
        let partitions = discover_partitions(&path, &mut notes);
        let _ = std::fs::remove_file(&path);

        assert_eq!(partitions.len(), 2);
        assert_eq!(partitions[0].offset, 2111 * 512);
        assert!(partitions[0].label.contains("mbr-logical-1"));
        assert_eq!(partitions[1].offset, 4111 * 512);
        assert!(partitions[1].label.contains("mbr-logical-2"));
    }

    /// 写入结构化或二进制内容。
    fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
        let bytes = value.to_le_bytes();
        buf[offset] = bytes[0];
        buf[offset + 1] = bytes[1];
        buf[offset + 2] = bytes[2];
        buf[offset + 3] = bytes[3];
    }

    /// 写入结构化或二进制内容。
    fn write_u64(buf: &mut [u8], offset: usize, value: u64) {
        let bytes = value.to_le_bytes();
        buf[offset] = bytes[0];
        buf[offset + 1] = bytes[1];
        buf[offset + 2] = bytes[2];
        buf[offset + 3] = bytes[3];
        buf[offset + 4] = bytes[4];
        buf[offset + 5] = bytes[5];
        buf[offset + 6] = bytes[6];
        buf[offset + 7] = bytes[7];
    }
}
