//! 运行时配置加载与访问。
//!
//! 默认读取当前目录下的 `YanMiRestore.toml`，也支持通过命令行参数指定。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use serde::Deserialize;

static SETTINGS: OnceLock<AppConfig> = OnceLock::new();

/// 初始化配置。
pub fn init(path: Option<&Path>) -> Result<()> {
    if SETTINGS.get().is_some() {
        return Ok(());
    }

    let loaded = if let Some(path) = path {
        load_from_file(path)?
    } else {
        match default_config_path() {
            Some(path) => load_from_file(&path)?,
            None => AppConfig::default(),
        }
    };

    let _ = SETTINGS.set(loaded);
    Ok(())
}

/// 获取全局配置。
pub fn settings() -> &'static AppConfig {
    SETTINGS.get_or_init(AppConfig::default)
}

fn default_config_path() -> Option<PathBuf> {
    let current = std::env::current_dir().ok()?;
    let path = current.join("YanMiRestore.toml");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn load_from_file(path: &Path) -> Result<AppConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("读取配置文件失败：{}", path.display()))?;
    toml::from_str::<AppConfig>(&raw)
        .with_context(|| format!("解析配置文件失败：{}", path.display()))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub ui: UiConfig,
    pub scan: ScanConfig,
    pub carver: CarverConfig,
    pub fs: FsConfig,
    pub partition: PartitionConfig,
    pub recovery: RecoveryConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ui: UiConfig::default(),
            scan: ScanConfig::default(),
            carver: CarverConfig::default(),
            fs: FsConfig::default(),
            partition: PartitionConfig::default(),
            recovery: RecoveryConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub progress_refresh_hz: u8,
    pub progress_tick_ms: u64,
    pub startup_hold_seconds: u64,
    pub scan_progress_template: String,
    pub recover_progress_template: String,
    pub progress_chars: String,
    pub spinner_frames: Vec<String>,
    pub scan_progress_prefix: String,
    pub recover_progress_prefix: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            progress_refresh_hz: 20,
            progress_tick_ms: 120,
            startup_hold_seconds: 8,
            scan_progress_template:
                "{spinner:.green} {prefix:.bold} [{elapsed_precise}] [{bar:32.cyan/blue}] {pos}/{len} {percent:>3}% {msg}"
                    .to_string(),
            recover_progress_template:
                "{spinner:.green} {prefix:.bold} [{elapsed_precise}] [{bar:32.yellow/blue}] {pos}/{len} {percent:>3}% {msg}"
                    .to_string(),
            progress_chars: "=>-".to_string(),
            spinner_frames: vec![
                "|".to_string(),
                "/".to_string(),
                "-".to_string(),
                "\\".to_string(),
            ],
            scan_progress_prefix: "扫描".to_string(),
            recover_progress_prefix: "恢复".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    pub max_logical_findings: usize,
    pub recycle_dirs: Vec<String>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            max_logical_findings: 2_000,
            recycle_dirs: vec![
                "$Recycle.Bin".to_string(),
                "RECYCLER".to_string(),
                "Recycler".to_string(),
                "RECYCLED".to_string(),
                ".Trash".to_string(),
                ".Trashes".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CarverConfig {
    pub max_scan_bytes: usize,
    pub max_items_per_signature: usize,
    pub jpeg_min_size_bytes: usize,
    pub jpeg_max_size_bytes: usize,
    pub png_min_size_bytes: usize,
    pub png_max_size_bytes: usize,
    pub pdf_min_size_bytes: usize,
    pub pdf_max_size_bytes: usize,
    pub sqlite_fallback_size_bytes: usize,
    pub zip_search_window_bytes: usize,
    pub mp4_search_window_bytes: usize,
}

impl Default for CarverConfig {
    fn default() -> Self {
        Self {
            max_scan_bytes: 512 * 1024 * 1024,
            max_items_per_signature: 200,
            jpeg_min_size_bytes: 1_024,
            jpeg_max_size_bytes: 32 * 1024 * 1024,
            png_min_size_bytes: 256,
            png_max_size_bytes: 32 * 1024 * 1024,
            pdf_min_size_bytes: 1_024,
            pdf_max_size_bytes: 128 * 1024 * 1024,
            sqlite_fallback_size_bytes: 1_048_576,
            zip_search_window_bytes: 64 * 1024 * 1024,
            mp4_search_window_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FsConfig {
    pub ntfs: NtfsConfig,
    pub fat: FatConfig,
    pub ext4: Ext4Config,
}

impl Default for FsConfig {
    fn default() -> Self {
        Self {
            ntfs: NtfsConfig::default(),
            fat: FatConfig::default(),
            ext4: Ext4Config::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NtfsConfig {
    pub max_findings: usize,
    pub max_mft_records_metadata: usize,
    pub max_mft_records_deep: usize,
}

impl Default for NtfsConfig {
    fn default() -> Self {
        Self {
            max_findings: 5_000,
            max_mft_records_metadata: 20_000,
            max_mft_records_deep: 100_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FatConfig {
    pub max_findings_metadata: usize,
    pub max_findings_deep: usize,
    pub max_cluster_chain_metadata: usize,
    pub max_cluster_chain_deep: usize,
    pub max_directory_visits_metadata: usize,
    pub max_directory_visits_deep: usize,
}

impl Default for FatConfig {
    fn default() -> Self {
        Self {
            max_findings_metadata: 5_000,
            max_findings_deep: 20_000,
            max_cluster_chain_metadata: 16_384,
            max_cluster_chain_deep: 131_072,
            max_directory_visits_metadata: 4_096,
            max_directory_visits_deep: 32_768,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Ext4Config {
    pub max_deleted_items_per_volume: usize,
    pub max_inodes_metadata: u64,
    pub max_inodes_deep: u64,
    pub max_extent_leaf_items: usize,
    pub max_extent_tree_nodes: usize,
}

impl Default for Ext4Config {
    fn default() -> Self {
        Self {
            max_deleted_items_per_volume: 8_000,
            max_inodes_metadata: 250_000,
            max_inodes_deep: 2_000_000,
            max_extent_leaf_items: 32_000,
            max_extent_tree_nodes: 16_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PartitionConfig {
    pub max_gpt_entries: usize,
    pub max_ebr_chain_entries: usize,
}

impl Default for PartitionConfig {
    fn default() -> Self {
        Self {
            max_gpt_entries: 4_096,
            max_ebr_chain_entries: 128,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RecoveryConfig {
    pub copy_buffer_size: usize,
    pub raw_io_alignment_bytes: u64,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            copy_buffer_size: 64 * 1024,
            raw_io_alignment_bytes: 512,
        }
    }
}
