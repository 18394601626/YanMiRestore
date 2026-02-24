//! FAT 扫描模块：当前提供签名识别与占位式扫描输出。
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::config;
use crate::model::{RecoverableItem, ScanDepth};

pub use crate::model::FatVolumeKind;

const EXFAT_SIGNATURE: &[u8; 8] = b"EXFAT   ";

fn max_fat_findings(depth: ScanDepth) -> usize {
    match depth {
        ScanDepth::Metadata => config::settings().fs.fat.max_findings_metadata.max(1),
        ScanDepth::Deep => config::settings().fs.fat.max_findings_deep.max(1),
    }
}

/// 判断指定偏移是否存在 FAT/exFAT 引导特征。
pub fn detect_fat_signature_at(
    source: &Path,
    volume_offset: u64,
) -> std::io::Result<Option<FatVolumeKind>> {
    let mut file = std::fs::File::open(source)?;
    let mut boot = [0_u8; 512];
    read_exact_at(&mut file, volume_offset, &mut boot)?;

    if boot.get(3..11) == Some(EXFAT_SIGNATURE) {
        return Ok(Some(FatVolumeKind::ExFat));
    }

    if is_likely_fat(&boot) {
        let kind = infer_fat_kind(&boot);
        return Ok(Some(kind));
    }

    Ok(None)
}

/// 扫描 FAT 删除目录项。
///
/// 说明：
/// 1. 当前版本先保证稳定可运行，保留统一输出结构。
/// 2. 后续可继续扩展 FAT12/16/32 与 exFAT 的深度解析。
pub fn scan_deleted_entries_at(
    source: &Path,
    depth: ScanDepth,
    notes: &mut Vec<String>,
    volume_offset: u64,
    volume_label: &str,
) -> Vec<RecoverableItem> {
    match detect_fat_signature_at(source, volume_offset) {
        Ok(Some(kind)) => {
            notes.push(format!(
                "FAT 扫描 [{} @ {}]：识别到卷类型 {:?}，当前版本暂未启用深度删除目录项解析。",
                volume_label, volume_offset, kind
            ));
            notes.push(format!(
                "FAT 扫描配置：depth={:?}，max_findings={}",
                depth,
                max_fat_findings(depth)
            ));
        }
        Ok(None) => notes.push(format!(
            "FAT 扫描 [{} @ {}]：未识别为 FAT/exFAT 卷。",
            volume_label, volume_offset
        )),
        Err(error) => notes.push(format!(
            "FAT 扫描 [{} @ {}]：读取签名失败（{}）。",
            volume_label, volume_offset, error
        )),
    }
    Vec::new()
}

fn is_likely_fat(boot: &[u8; 512]) -> bool {
    let bytes_per_sector = read_u16(boot, 11).unwrap_or(0);
    let sectors_per_cluster = boot[13];
    let reserved_sectors = read_u16(boot, 14).unwrap_or(0);
    let fat_count = boot[16];

    if bytes_per_sector == 0 || sectors_per_cluster == 0 || reserved_sectors == 0 || fat_count == 0
    {
        return false;
    }
    if boot[510] != 0x55 || boot[511] != 0xAA {
        return false;
    }

    let fs_type_16 = boot.get(54..62).unwrap_or(&[]);
    let fs_type_32 = boot.get(82..90).unwrap_or(&[]);
    fs_type_16.starts_with(b"FAT") || fs_type_32.starts_with(b"FAT")
}

fn infer_fat_kind(boot: &[u8; 512]) -> FatVolumeKind {
    let root_entry_count = read_u16(boot, 17).unwrap_or(0);
    let fat16_size = read_u16(boot, 22).unwrap_or(0);
    let fat32_size = read_u32(boot, 36).unwrap_or(0);

    if root_entry_count == 0 && fat16_size == 0 && fat32_size > 0 {
        FatVolumeKind::Fat32
    } else {
        // 在占位实现中将 FAT12/FAT16 统一归类到 FAT16。
        FatVolumeKind::Fat16
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

#[cfg(test)]
mod tests {
    use super::{detect_fat_signature_at, scan_deleted_entries_at, FatVolumeKind};
    use crate::model::ScanDepth;

    fn write_u16(buf: &mut [u8], offset: usize, value: u16) {
        let bytes = value.to_le_bytes();
        buf[offset] = bytes[0];
        buf[offset + 1] = bytes[1];
    }

    fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
        let bytes = value.to_le_bytes();
        buf[offset] = bytes[0];
        buf[offset + 1] = bytes[1];
        buf[offset + 2] = bytes[2];
        buf[offset + 3] = bytes[3];
    }

    #[test]
    fn detect_exfat_boot() {
        let path = std::env::temp_dir().join("fat-detect-exfat-test.img");
        let mut image = vec![0_u8; 1024 * 1024];
        image[3..11].copy_from_slice(b"EXFAT   ");
        image[510] = 0x55;
        image[511] = 0xAA;
        std::fs::write(&path, &image).expect("write test image");

        let kind = detect_fat_signature_at(&path, 0)
            .expect("detect")
            .expect("expected kind");
        assert_eq!(kind, FatVolumeKind::ExFat);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn detect_fat32_boot() {
        let path = std::env::temp_dir().join("fat-detect-fat32-test.img");
        let mut image = vec![0_u8; 1024 * 1024];
        write_u16(&mut image, 11, 512);
        image[13] = 1;
        write_u16(&mut image, 14, 32);
        image[16] = 2;
        write_u16(&mut image, 17, 0);
        write_u16(&mut image, 22, 0);
        write_u32(&mut image, 36, 4096);
        image[82..87].copy_from_slice(b"FAT32");
        image[510] = 0x55;
        image[511] = 0xAA;
        std::fs::write(&path, &image).expect("write test image");

        let kind = detect_fat_signature_at(&path, 0)
            .expect("detect")
            .expect("expected kind");
        assert_eq!(kind, FatVolumeKind::Fat32);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn scan_returns_placeholder_output() {
        let path = std::env::temp_dir().join("fat-scan-placeholder-test.img");
        let mut image = vec![0_u8; 1024 * 1024];
        image[3..11].copy_from_slice(b"EXFAT   ");
        image[510] = 0x55;
        image[511] = 0xAA;
        std::fs::write(&path, &image).expect("write test image");

        let mut notes = Vec::new();
        let items = scan_deleted_entries_at(&path, ScanDepth::Deep, &mut notes, 0, "unit");
        assert!(items.is_empty());
        assert!(!notes.is_empty());

        let _ = std::fs::remove_file(path);
    }
}
