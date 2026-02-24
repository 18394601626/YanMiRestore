//! 方案阶段模型。

use serde::{Deserialize, Serialize};

use super::{FsHint, ScanDepth, TargetKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 扫描执行计划。
pub struct ScanPlan {
    /// 案件编号。
    pub case_id: String,
    /// 目标介质类型。
    pub target_kind: TargetKind,
    /// 扫描深度。
    pub depth: ScanDepth,
    /// 文件系统提示。
    pub fs_hint: FsHint,
    /// 阶段列表。
    pub stages: Vec<PlanStage>,
    /// 安全约束。
    pub safety_rules: Vec<String>,
    /// 前置假设。
    pub assumptions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 计划中的单个执行阶段。
pub struct PlanStage {
    /// 阶段编号。
    pub id: String,
    /// 阶段标题。
    pub title: String,
    /// 阶段说明。
    pub detail: String,
}
