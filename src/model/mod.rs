//! 数据模型层入口。
//!
//! 此模块负责聚合子模块并统一导出公共类型，避免业务代码直接依赖文件布局。

mod device;
mod enums;
mod fs;
mod plan;
mod recovery;
mod request;
mod scan;

pub use device::DeviceCandidate;
pub use enums::{FsHint, ScanDepth, TargetKind};
pub use fs::{Ext4ScanOutput, FatVolumeKind, NtfsScanOutput, PartitionCandidate};
pub use plan::{PlanStage, ScanPlan};
pub use recovery::{RecoveryAction, RecoverySession};
pub use request::{PlanInput, RecoveryRequest, ScanRequest};
pub use scan::{
    CarveResult, DeviceSnapshot, Ext4DataSummary, FatDataSummary, FsMetrics, FsScanResult,
    NtfsDataSummary, RecoverableItem, ScanReport, SourceSegment,
};
