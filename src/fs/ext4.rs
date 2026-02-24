//! ext4 扫描模块：当前提供签名识别与占位式扫描输出。
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::config;
use crate::model::{Ext4DataSummary, Ext4ScanOutput, ScanDepth};

/// superblock 偏移（相对卷起点）。
const EXT4_SUPERBLOCK_OFFSET: u64 = 1024;
/// superblock 魔数在 superblock 内的偏移。
const EXT4_SUPER_MAGIC_OFFSET: u64 = 56;
/// ext4 superblock 魔数。
const EXT4_SUPER_MAGIC: u16 = 0xEF53;

fn max_deleted_items_per_volume() -> usize {
    config::settings()
        .fs
        .ext4
        .max_deleted_items_per_volume
        .max(1)
}

/// 判断指定偏移是否存在 ext4 superblock 魔数。
pub fn has_ext4_signature_at(source: &Path, volume_offset: u64) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(source)?;
    let Some(superblock_pos) = volume_offset.checked_add(EXT4_SUPERBLOCK_OFFSET) else {
        return Ok(false);
    };
    let Some(magic_pos) = superblock_pos.checked_add(EXT4_SUPER_MAGIC_OFFSET) else {
        return Ok(false);
    };

    file.seek(SeekFrom::Start(magic_pos))?;
    let mut bytes = [0_u8; 2];
    if file.read_exact(&mut bytes).is_err() {
        return Ok(false);
    }
    Ok(u16::from_le_bytes(bytes) == EXT4_SUPER_MAGIC)
}

/// 扫描 ext4 删除 inode。
///
/// 说明：
/// 1. 当前实现优先保证稳定性，先返回统一结构与提示信息。
/// 2. 后续可在此基础上继续扩展 inode/extent 深度解析。
pub fn scan_deleted_inodes_at(
    source: &Path,
    depth: ScanDepth,
    notes: &mut Vec<String>,
    volume_offset: u64,
    volume_label: &str,
) -> Ext4ScanOutput {
    match has_ext4_signature_at(source, volume_offset) {
        Ok(true) => {
            notes.push(format!(
                "ext4 扫描 [{} @ {}]：已识别 ext4 签名，当前版本暂未启用深度 inode 解析。",
                volume_label, volume_offset
            ));
            notes.push(format!(
                "ext4 扫描配置：depth={:?}，max_deleted_items_per_volume={}",
                depth,
                max_deleted_items_per_volume()
            ));

            Ext4ScanOutput {
                items: Vec::new(),
                summary: Ext4DataSummary {
                    volumes_scanned: 1,
                    ..Ext4DataSummary::default()
                },
            }
        }
        Ok(false) => {
            notes.push(format!(
                "ext4 扫描 [{} @ {}]：未识别为 ext4 卷。",
                volume_label, volume_offset
            ));
            Ext4ScanOutput::default()
        }
        Err(error) => {
            notes.push(format!(
                "ext4 扫描 [{} @ {}]：读取签名失败（{}）。",
                volume_label, volume_offset, error
            ));
            Ext4ScanOutput::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{has_ext4_signature_at, scan_deleted_inodes_at};
    use crate::model::ScanDepth;

    fn write_u16_le(buf: &mut [u8], offset: usize, value: u16) {
        buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn detects_ext4_signature() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!("ext4-detect-{}.img", std::process::id()));

        let mut image = vec![0_u8; 16 * 1024];
        write_u16_le(&mut image, 1024 + 56, 0xEF53);
        std::fs::write(&path, &image).expect("write image");

        assert!(has_ext4_signature_at(&path, 0).expect("signature check"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn scan_ext4_placeholder_output() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!("ext4-scan-{}.img", std::process::id()));

        let mut image = vec![0_u8; 16 * 1024];
        write_u16_le(&mut image, 1024 + 56, 0xEF53);
        std::fs::write(&path, &image).expect("write image");

        let mut notes = Vec::new();
        let output = scan_deleted_inodes_at(&path, ScanDepth::Deep, &mut notes, 0, "unit");
        assert_eq!(output.summary.volumes_scanned, 1);
        assert!(output.items.is_empty());
        assert!(!notes.is_empty());

        let _ = std::fs::remove_file(path);
    }
}
