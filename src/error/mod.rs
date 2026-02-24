//! 错误类型定义。

use thiserror::Error;

/// 项目统一结果类型。
pub type RecoveryResult<T> = std::result::Result<T, RecoveryError>;

#[derive(Debug, Error)]
/// 业务流程可能出现的错误。
pub enum RecoveryError {
    #[error("源路径不存在：{0}")]
    SourceNotFound(String),
    #[error("目标路径不安全：{0}")]
    UnsafeDestination(String),
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 序列化错误：{0}")]
    Json(#[from] serde_json::Error),
}
