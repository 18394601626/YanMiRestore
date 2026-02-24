//! 文案标签工具。
use crate::model::TargetKind;

/// 将设备类型枚举转换为可读中文标签。
pub fn target_kind_label(kind: TargetKind) -> &'static str {
    match kind {
        TargetKind::Auto => "自动判断",
        TargetKind::PcDisk => "电脑硬盘",
        TargetKind::UsbDisk => "移动硬盘",
        TargetKind::Phone => "手机",
        TargetKind::Other => "其他设备",
    }
}
