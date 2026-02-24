//! 命令行定义：声明参数、子命令与默认值。

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::model::{FsHint, ScanDepth, TargetKind};

#[derive(Debug, Parser)]
#[command(name = "YanMiRestore", version, about = "安全优先的数据恢复命令行工具")]
pub struct Cli {
    /// 配置文件路径（TOML）。未指定时会尝试读取当前目录下的 YanMiRestore.toml。
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// 根据目标介质生成恢复设计方案。
    Design(DesignArgs),
    /// 列出当前系统可用设备，并显示自动识别类型。
    Devices,
    /// 执行只读扫描并输出 JSON 报告。
    Scan(ScanArgs),
    /// 基于扫描报告生成恢复清单。
    Recover(RecoverArgs),
}

#[derive(Debug, Args)]
pub struct DesignArgs {
    /// 报告使用的案件编号。
    #[arg(long, default_value = "CASE-0001")]
    pub case_id: String,
    /// 目标介质类型。
    #[arg(long, value_enum, default_value_t = TargetKind::Auto)]
    pub target_kind: TargetKind,
    /// 扫描深度。
    #[arg(long, value_enum, default_value_t = ScanDepth::Metadata)]
    pub depth: ScanDepth,
    /// 可选的文件系统提示。
    #[arg(long, value_enum, default_value_t = FsHint::Auto)]
    pub fs_hint: FsHint,
    /// 是否在方案中启用签名雕刻阶段。
    #[arg(long, default_value_t = false)]
    pub include_carving: bool,
}

#[derive(Debug, Args)]
pub struct ScanArgs {
    /// 报告使用的案件编号。
    #[arg(long, default_value = "CASE-0001")]
    pub case_id: String,
    /// 源路径（镜像文件、原始转储或挂载目录）。
    #[arg(long)]
    pub source: PathBuf,
    /// 报告输出目录。
    #[arg(long, default_value = "./output")]
    pub output: PathBuf,
    /// 目标介质类型。
    #[arg(long, value_enum, default_value_t = TargetKind::Auto)]
    pub target_kind: TargetKind,
    /// 扫描深度。
    #[arg(long, value_enum, default_value_t = ScanDepth::Metadata)]
    pub depth: ScanDepth,
    /// 可选的文件系统提示。
    #[arg(long, value_enum, default_value_t = FsHint::Auto)]
    pub fs_hint: FsHint,
    /// 是否启用签名雕刻启发式扫描。
    #[arg(long, default_value_t = false)]
    pub include_carving: bool,
    /// 自动识别模式下跳过键入确认，直接继续扫描。
    #[arg(long, default_value_t = false)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct RecoverArgs {
    /// 扫描报告 JSON 路径。
    #[arg(long)]
    pub report: PathBuf,
    /// 恢复输出目录。
    #[arg(long)]
    pub destination: PathBuf,
    /// 执行实际恢复（不传该参数时为 dry-run）。
    #[arg(long, default_value_t = false)]
    pub execute: bool,
    /// 禁用原文件名优先策略（默认优先使用原文件名，冲突时自动追加序号）。
    #[arg(long, default_value_t = false)]
    pub no_keep_original_name: bool,
    /// 禁用时间戳同步（默认在可获取时同步源文件访问/修改时间）。
    #[arg(long, default_value_t = false)]
    pub no_preserve_timestamps: bool,
    /// 跳过签名雕刻候选项，仅恢复文件系统候选项。
    #[arg(long, default_value_t = false)]
    pub skip_carved: bool,
}
