//! 命令输入在业务层的请求模型。

use std::path::PathBuf;

use super::{FsHint, ScanDepth, TargetKind};

#[derive(Debug, Clone)]
/// 方案生成阶段的输入参数。
pub struct PlanInput {
    /// 案件编号。
    pub case_id: String,
    /// 用户指定或自动识别的目标介质类型。
    pub target_kind: TargetKind,
    /// 扫描深度。
    pub depth: ScanDepth,
    /// 文件系统提示。
    pub fs_hint: FsHint,
    /// 是否纳入签名雕刻阶段。
    pub include_carving: bool,
}

#[derive(Debug, Clone)]
/// 扫描阶段请求。
pub struct ScanRequest {
    /// 方案输入快照，便于在报告中回溯参数。
    pub plan: PlanInput,
    /// 数据源路径（目录、镜像或原始卷）。
    pub source: PathBuf,
    /// 扫描报告输出目录。
    pub output_dir: PathBuf,
    /// 目标介质类型。
    pub target_kind: TargetKind,
    /// 扫描深度。
    pub depth: ScanDepth,
    /// 文件系统提示。
    pub fs_hint: FsHint,
    /// 是否启用签名雕刻。
    pub include_carving: bool,
}

#[derive(Debug, Clone)]
/// 恢复阶段请求。
pub struct RecoveryRequest {
    /// 扫描报告路径（JSON）。
    pub report_path: PathBuf,
    /// 恢复输出目录。
    pub destination: PathBuf,
    /// 是否为预演模式。
    pub dry_run: bool,
    /// 是否优先沿用原文件名。
    pub keep_original_name: bool,
    /// 是否同步源文件时间戳。
    pub preserve_timestamps: bool,
    /// 是否跳过签名雕刻候选项。
    pub skip_carved: bool,
}
