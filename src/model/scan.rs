//! 扫描阶段结果模型。

use serde::{Deserialize, Serialize};

use super::{ScanPlan, TargetKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 扫描时采集到的设备信息快照。
pub struct DeviceSnapshot {
    /// 用户输入的源路径。
    pub source: String,
    /// 源类型（逻辑目录/镜像/原始卷等）。
    pub source_type: String,
    /// 源大小（字节）。
    pub size_bytes: u64,
    /// 自动识别到的设备类型。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_target_kind: Option<TargetKind>,
    /// 自动识别依据说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_hint: Option<String>,
    /// 可用时记录底层扫描路径（如 Windows 原始卷路径）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_level_source_path: Option<String>,
    /// 探测过程备注。
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 文件系统扫描结果。
pub struct FsScanResult {
    /// 识别到的文件系统名称。
    pub detected_fs: Option<String>,
    /// 删除条目候选数量。
    pub deleted_entry_candidates: u64,
    /// 扫描备注。
    pub notes: Vec<String>,
    /// 可恢复候选列表。
    pub items: Vec<RecoverableItem>,
    /// 文件系统维度统计。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<FsMetrics>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// 各类文件系统的聚合统计。
pub struct FsMetrics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ntfs: Option<NtfsDataSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fat: Option<FatDataSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext4: Option<Ext4DataSummary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// NTFS 扫描统计。
pub struct NtfsDataSummary {
    /// 可恢复条目数量。
    pub recoverable: u64,
    /// 仅有元数据、缺少可恢复数据段的条目数量。
    pub metadata_only: u64,
    /// 压缩流（暂不支持）条目数量。
    pub unsupported_compressed: u64,
    /// 加密流（暂不支持）条目数量。
    pub unsupported_encrypted: u64,
    /// 同时压缩+加密（暂不支持）条目数量。
    pub unsupported_compressed_encrypted: u64,
    /// 运行列表解析失败条目数量。
    pub runlist_failed: u64,
    /// 可恢复但包含稀疏段条目数量。
    pub recoverable_with_sparse: u64,
}

impl NtfsDataSummary {
    /// 将另一份统计累加到当前对象。
    pub fn add_assign(&mut self, other: &NtfsDataSummary) {
        self.recoverable = self.recoverable.saturating_add(other.recoverable);
        self.metadata_only = self.metadata_only.saturating_add(other.metadata_only);
        self.unsupported_compressed = self
            .unsupported_compressed
            .saturating_add(other.unsupported_compressed);
        self.unsupported_encrypted = self
            .unsupported_encrypted
            .saturating_add(other.unsupported_encrypted);
        self.unsupported_compressed_encrypted = self
            .unsupported_compressed_encrypted
            .saturating_add(other.unsupported_compressed_encrypted);
        self.runlist_failed = self.runlist_failed.saturating_add(other.runlist_failed);
        self.recoverable_with_sparse = self
            .recoverable_with_sparse
            .saturating_add(other.recoverable_with_sparse);
    }

    /// 判断统计项是否全部为 0。
    pub fn is_zero(&self) -> bool {
        self.recoverable == 0
            && self.metadata_only == 0
            && self.unsupported_compressed == 0
            && self.unsupported_encrypted == 0
            && self.unsupported_compressed_encrypted == 0
            && self.runlist_failed == 0
            && self.recoverable_with_sparse == 0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// FAT/exFAT 扫描统计。
pub struct FatDataSummary {
    /// 扫描到的卷数量。
    pub volumes_scanned: u64,
    pub fat12_volumes: u64,
    pub fat16_volumes: u64,
    pub fat32_volumes: u64,
    pub exfat_volumes: u64,
    /// 删除文件候选数量。
    pub deleted_files: u64,
    /// 删除目录候选数量。
    pub deleted_directories: u64,
    /// 带有效恢复段的条目数量。
    pub with_recovery_segments: u64,
    /// 仅有元数据的条目数量。
    pub metadata_only: u64,
}

impl FatDataSummary {
    /// 将另一份统计累加到当前对象。
    pub fn add_assign(&mut self, other: &FatDataSummary) {
        self.volumes_scanned = self.volumes_scanned.saturating_add(other.volumes_scanned);
        self.fat12_volumes = self.fat12_volumes.saturating_add(other.fat12_volumes);
        self.fat16_volumes = self.fat16_volumes.saturating_add(other.fat16_volumes);
        self.fat32_volumes = self.fat32_volumes.saturating_add(other.fat32_volumes);
        self.exfat_volumes = self.exfat_volumes.saturating_add(other.exfat_volumes);
        self.deleted_files = self.deleted_files.saturating_add(other.deleted_files);
        self.deleted_directories = self
            .deleted_directories
            .saturating_add(other.deleted_directories);
        self.with_recovery_segments = self
            .with_recovery_segments
            .saturating_add(other.with_recovery_segments);
        self.metadata_only = self.metadata_only.saturating_add(other.metadata_only);
    }

    /// 判断统计项是否全部为 0。
    pub fn is_zero(&self) -> bool {
        self.volumes_scanned == 0
            && self.fat12_volumes == 0
            && self.fat16_volumes == 0
            && self.fat32_volumes == 0
            && self.exfat_volumes == 0
            && self.deleted_files == 0
            && self.deleted_directories == 0
            && self.with_recovery_segments == 0
            && self.metadata_only == 0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// ext4 扫描统计。
pub struct Ext4DataSummary {
    /// 扫描到的卷数量。
    pub volumes_scanned: u64,
    /// 删除文件候选数量。
    pub deleted_files: u64,
    /// 删除目录候选数量。
    pub deleted_directories: u64,
    /// 带有效恢复段的条目数量。
    pub with_recovery_segments: u64,
    /// 包含稀疏段的条目数量。
    pub with_sparse_segments: u64,
    /// 仅有元数据的条目数量。
    pub metadata_only: u64,
    /// extent 深度超出当前实现能力的条目数量。
    pub extents_depth_unsupported: u64,
    /// 旧式指针块条目数量。
    pub legacy_pointer_files: u64,
}

impl Ext4DataSummary {
    /// 将另一份统计累加到当前对象。
    pub fn add_assign(&mut self, other: &Ext4DataSummary) {
        self.volumes_scanned = self.volumes_scanned.saturating_add(other.volumes_scanned);
        self.deleted_files = self.deleted_files.saturating_add(other.deleted_files);
        self.deleted_directories = self
            .deleted_directories
            .saturating_add(other.deleted_directories);
        self.with_recovery_segments = self
            .with_recovery_segments
            .saturating_add(other.with_recovery_segments);
        self.with_sparse_segments = self
            .with_sparse_segments
            .saturating_add(other.with_sparse_segments);
        self.metadata_only = self.metadata_only.saturating_add(other.metadata_only);
        self.extents_depth_unsupported = self
            .extents_depth_unsupported
            .saturating_add(other.extents_depth_unsupported);
        self.legacy_pointer_files = self
            .legacy_pointer_files
            .saturating_add(other.legacy_pointer_files);
    }

    /// 判断统计项是否全部为 0。
    pub fn is_zero(&self) -> bool {
        self.volumes_scanned == 0
            && self.deleted_files == 0
            && self.deleted_directories == 0
            && self.with_recovery_segments == 0
            && self.with_sparse_segments == 0
            && self.metadata_only == 0
            && self.extents_depth_unsupported == 0
            && self.legacy_pointer_files == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 签名雕刻阶段结果。
pub struct CarveResult {
    /// 是否启用了雕刻阶段。
    pub enabled: bool,
    /// 启用的签名列表。
    pub signatures: Vec<String>,
    /// 雕刻候选数量。
    pub carved_candidates: u64,
    /// 阶段备注。
    pub notes: Vec<String>,
    /// 雕刻得到的可恢复条目。
    pub items: Vec<RecoverableItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 可恢复候选项。
pub struct RecoverableItem {
    /// 条目唯一标识。
    pub id: String,
    /// 条目类别。
    pub category: String,
    /// 置信度（0~1）。
    pub confidence: f32,
    /// 条目说明。
    pub note: String,
    /// 建议输出文件名。
    pub suggested_name: String,
    /// 逻辑路径来源（如回收站路径）。
    pub source_path: Option<String>,
    /// 连续源偏移（适用于雕刻）。
    pub source_offset: Option<u64>,
    /// 连续源长度。
    pub size_bytes: Option<u64>,
    /// 分段源坐标（适用于碎片恢复）。
    #[serde(default)]
    pub source_segments: Vec<SourceSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 源介质中的一个可读分段。
pub struct SourceSegment {
    /// 分段起始偏移。
    pub offset: u64,
    /// 分段长度。
    pub length: u64,
    /// 是否为稀疏段（全 0）。
    #[serde(default)]
    pub sparse: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 一次扫描输出的完整报告。
pub struct ScanReport {
    /// 报告生成时间（RFC3339）。
    pub generated_at: String,
    /// 扫描计划快照。
    pub plan: ScanPlan,
    /// 扫描源路径。
    pub source: String,
    /// 设备探测快照。
    pub device_snapshot: DeviceSnapshot,
    /// 文件系统扫描结果。
    pub fs_result: FsScanResult,
    /// 签名雕刻结果。
    pub carve_result: CarveResult,
    /// 合并后的候选项列表。
    pub findings: Vec<RecoverableItem>,
    /// 全局告警。
    pub warnings: Vec<String>,
}
