//! 文件系统扫描相关模型。
use serde::{Deserialize, Serialize};

use super::{Ext4DataSummary, NtfsDataSummary, RecoverableItem};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
/// FAT 家族卷类型。
pub enum FatVolumeKind {
    Fat12,
    Fat16,
    Fat32,
    ExFat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 分区候选信息。
pub struct PartitionCandidate {
    pub offset: u64,
    pub size: u64,
    pub label: String,
    pub scheme: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// NTFS 扫描输出。
pub struct NtfsScanOutput {
    pub items: Vec<RecoverableItem>,
    pub summary: NtfsDataSummary,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// ext4 扫描输出。
pub struct Ext4ScanOutput {
    pub items: Vec<RecoverableItem>,
    pub summary: Ext4DataSummary,
}
