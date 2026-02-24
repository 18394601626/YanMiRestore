//! 扫描与恢复流程共用的枚举定义。

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
/// 目标介质类型。
pub enum TargetKind {
    /// 自动判断介质类型。
    Auto,
    /// 电脑本地磁盘（HDD/SSD）。
    PcDisk,
    /// 移动硬盘或 U 盘等可移动存储。
    UsbDisk,
    /// 手机导出的备份目录或镜像。
    Phone,
    /// 无法归类时使用的兜底类型。
    Other,
}

impl Default for TargetKind {
    /// 默认启用自动识别。
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
/// 扫描深度。
pub enum ScanDepth {
    /// 元数据优先，速度更快。
    Metadata,
    /// 深度扫描，覆盖更多恢复线索。
    Deep,
}

impl Default for ScanDepth {
    /// 默认采用元数据扫描。
    fn default() -> Self {
        Self::Metadata
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
/// 文件系统提示。
pub enum FsHint {
    /// 自动探测文件系统。
    Auto,
    Ntfs,
    Fat32,
    Exfat,
    Ext4,
    Apfs,
    F2fs,
}

impl Default for FsHint {
    /// 默认采用自动探测。
    fn default() -> Self {
        Self::Auto
    }
}
