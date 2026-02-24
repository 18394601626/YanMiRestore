//! 恢复阶段输出模型。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 单个候选项的恢复结果。
pub struct RecoveryAction {
    /// 候选项 ID。
    pub item_id: String,
    /// 执行状态（成功/失败/跳过/计划）。
    pub status: String,
    /// 执行说明或错误信息。
    pub note: String,
    /// 输出文件路径（若有）。
    pub output_path: Option<String>,
    /// 实际写入字节数（若有）。
    pub bytes_written: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 一次恢复任务的完整结果清单。
pub struct RecoverySession {
    /// 生成时间（RFC3339）。
    pub generated_at: String,
    /// 案件编号。
    pub case_id: String,
    /// 输出目录。
    pub destination: String,
    /// 是否为预演模式。
    pub dry_run: bool,
    /// 动作总数。
    pub action_count: u64,
    /// 条目级恢复动作。
    pub actions: Vec<RecoveryAction>,
    /// 会话说明与告警。
    pub notes: Vec<String>,
    /// 恢复清单文件路径。
    pub manifest_path: String,
}
