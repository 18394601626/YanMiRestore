//! 设备模型定义。
use std::path::PathBuf;

use super::TargetKind;

#[derive(Debug, Clone)]
/// 可选设备项：用于 `devices` 子命令输出。
pub struct DeviceCandidate {
    pub path: PathBuf,
    pub target_kind: TargetKind,
    pub note: String,
}
