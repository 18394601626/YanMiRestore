//! 设备探测模块：识别输入源并给出设备类型判断。
use std::fs::Metadata;
use std::path::{Path, PathBuf};

use crate::error::{RecoveryError, RecoveryResult};
use crate::model::{DeviceCandidate, DeviceSnapshot, ScanRequest, TargetKind};
use crate::utils::label::target_kind_label;

#[cfg(target_os = "windows")]
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

#[cfg(target_os = "windows")]
const DRIVE_UNKNOWN: u32 = 0;
#[cfg(target_os = "windows")]
const DRIVE_NO_ROOT_DIR: u32 = 1;
#[cfg(target_os = "windows")]
const DRIVE_REMOVABLE: u32 = 2;
#[cfg(target_os = "windows")]
const DRIVE_FIXED: u32 = 3;
#[cfg(target_os = "windows")]
const DRIVE_REMOTE: u32 = 4;
#[cfg(target_os = "windows")]
const DRIVE_CDROM: u32 = 5;
#[cfg(target_os = "windows")]
const DRIVE_RAMDISK: u32 = 6;

/// 设备探测能力接口。
pub trait DeviceInspector {
    /// 执行设备探测并返回快照信息。
    fn inspect(&self, request: &ScanRequest) -> RecoveryResult<DeviceSnapshot>;
}

#[derive(Debug, Default)]
/// 本地设备探测器。
pub struct LocalDeviceInspector;

impl DeviceInspector for LocalDeviceInspector {
    fn inspect(&self, request: &ScanRequest) -> RecoveryResult<DeviceSnapshot> {
        if !request.source.exists() {
            return Err(RecoveryError::SourceNotFound(
                request.source.display().to_string(),
            ));
        }

        let metadata = std::fs::metadata(&request.source)?;
        let source_path = request
            .source
            .canonicalize()
            .unwrap_or_else(|_| request.source.clone());

        let mut notes = vec!["源数据将以只读流程处理。".to_string()];
        let mut low_level_source_path = None;
        let source_type = if metadata.is_dir() {
            notes.push("目录输入将按挂载卷或逻辑备份处理。".to_string());
            #[cfg(target_os = "windows")]
            {
                if let Some(raw_path) = try_windows_raw_volume_path(&source_path) {
                    notes.push(format!(
                        "检测到盘符根目录，已启用底层卷扫描路径：{}。",
                        raw_path.display()
                    ));
                    low_level_source_path = Some(raw_path.display().to_string());
                } else if windows_volume_root(&source_path).is_some() {
                    notes.push(
                        "检测到盘符根目录，但未获取到底层卷访问权限（请尝试以管理员身份运行）。"
                            .to_string(),
                    );
                }
            }
            "mounted-path".to_string()
        } else {
            notes.push("文件输入将按原始镜像或设备导出转储处理。".to_string());
            "image-file".to_string()
        };

        let (resolved_kind, device_hint) =
            resolve_target_kind(&source_path, &metadata, request.target_kind, &source_type);
        if request.target_kind == TargetKind::Auto {
            notes.push(format!(
                "自动识别设备类型：{}。",
                target_kind_label(resolved_kind)
            ));
        } else {
            notes.push(format!(
                "按参数指定设备类型：{}。",
                target_kind_label(resolved_kind)
            ));
        }
        notes.push(format!("识别依据：{device_hint}"));

        if resolved_kind == TargetKind::Phone {
            notes.push("手机恢复通常需要 ADB/iTunes 备份或 Root 后物理转储。".to_string());
        }

        Ok(DeviceSnapshot {
            source: source_path.display().to_string(),
            source_type,
            size_bytes: metadata.len(),
            detected_target_kind: Some(resolved_kind),
            device_hint: Some(device_hint),
            low_level_source_path,
            notes,
        })
    }
}

/// 列出当前系统可用的数据源路径。
pub fn list_available_devices() -> Vec<DeviceCandidate> {
    #[cfg(target_os = "windows")]
    {
        return list_windows_devices();
    }

    #[cfg(not(target_os = "windows"))]
    {
        list_unix_devices()
    }
}

fn resolve_target_kind(
    source: &Path,
    metadata: &Metadata,
    requested_kind: TargetKind,
    source_type: &str,
) -> (TargetKind, String) {
    if requested_kind != TargetKind::Auto {
        return (requested_kind, "命令行参数已显式指定目标类型".to_string());
    }

    if looks_like_phone_source(source, metadata.is_dir()) {
        return (
            TargetKind::Phone,
            "命中手机备份/目录特征（如 DCIM、MobileSync、Android）".to_string(),
        );
    }

    #[cfg(target_os = "windows")]
    if let Some((kind, hint)) = classify_windows_path(source) {
        return (kind, hint);
    }

    #[cfg(not(target_os = "windows"))]
    if let Some((kind, hint)) = classify_unix_path(source) {
        return (kind, hint);
    }

    if source_type == "image-file" && is_image_like_extension(source) {
        return (
            TargetKind::PcDisk,
            "检测到镜像文件扩展名（img/raw/dd/e01/vhd/vhdx）".to_string(),
        );
    }

    if source_type == "mounted-path" {
        return (
            TargetKind::Other,
            "目录来源未匹配已知盘符规则，按其他设备处理".to_string(),
        );
    }

    (
        TargetKind::Other,
        "未识别设备类型，按其他设备处理".to_string(),
    )
}

fn looks_like_phone_source(path: &Path, is_dir: bool) -> bool {
    let text = path.to_string_lossy().to_ascii_lowercase();
    let path_tokens = [
        "mobilesync\\backup",
        "mobilesync/backup",
        "dcim",
        "android",
        "iphone",
        "itunes",
    ];
    if path_tokens.iter().any(|token| text.contains(token)) {
        return true;
    }

    if !is_dir {
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase());
        return matches!(ext.as_deref(), Some("ab") | Some("ipsw"));
    }

    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.take(64).flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy().to_ascii_lowercase();
            if matches!(
                name.as_str(),
                "dcim" | "android" | "mobilesync" | "apple computer"
            ) {
                return true;
            }
        }
    }

    false
}

fn is_image_like_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .is_some_and(|ext| matches!(ext.as_str(), "img" | "raw" | "dd" | "e01" | "vhd" | "vhdx"))
}

#[cfg(target_os = "windows")]
fn classify_windows_path(path: &Path) -> Option<(TargetKind, String)> {
    let root = windows_volume_root(path)?;
    classify_windows_root(&root)
}

#[cfg(target_os = "windows")]
fn classify_windows_root(root: &str) -> Option<(TargetKind, String)> {
    let drive_type = drive_type_for_root(root);
    let (kind, hint) = match drive_type {
        DRIVE_REMOVABLE => (TargetKind::UsbDisk, "Windows 盘符类型为可移动存储"),
        DRIVE_FIXED => (TargetKind::PcDisk, "Windows 盘符类型为固定磁盘"),
        DRIVE_REMOTE => (TargetKind::Other, "Windows 盘符类型为网络存储"),
        DRIVE_CDROM => (TargetKind::Other, "Windows 盘符类型为光驱"),
        DRIVE_RAMDISK => (TargetKind::Other, "Windows 盘符类型为内存盘"),
        DRIVE_NO_ROOT_DIR => return None,
        DRIVE_UNKNOWN => (TargetKind::Other, "Windows 盘符类型未知"),
        _ => (TargetKind::Other, "Windows 盘符类型未定义"),
    };

    Some((kind, hint.to_string()))
}

#[cfg(target_os = "windows")]
fn drive_type_for_root(root: &str) -> u32 {
    let mut wide: Vec<u16> = root.encode_utf16().collect();
    wide.push(0);
    // 调用系统 API 查询盘符类型，仅读取元信息。
    unsafe { GetDriveTypeW(wide.as_ptr()) }
}

#[cfg(target_os = "windows")]
fn windows_volume_root(path: &Path) -> Option<String> {
    let mut text = path.to_string_lossy().replace('/', "\\");
    if let Some(stripped) = text.strip_prefix(r"\\?\") {
        text = stripped.to_string();
    }

    let bytes = text.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        let drive = text.chars().next()?.to_ascii_uppercase();
        return Some(format!("{drive}:\\"));
    }

    if let Some(rest) = text.strip_prefix(r"\\") {
        let mut parts = rest.split('\\').filter(|value| !value.is_empty());
        if let (Some(server), Some(share)) = (parts.next(), parts.next()) {
            return Some(format!("\\\\{}\\{}\\", server, share));
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn try_windows_raw_volume_path(path: &Path) -> Option<PathBuf> {
    let root = windows_volume_root(path)?;
    let bytes = root.as_bytes();
    if bytes.len() < 2 || bytes[1] != b':' {
        return None;
    }

    let drive = root.chars().next()?.to_ascii_uppercase();
    let raw = PathBuf::from(format!(r"\\.\{drive}:"));
    if std::fs::File::open(&raw).is_ok() {
        Some(raw)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn list_windows_devices() -> Vec<DeviceCandidate> {
    let mut devices = Vec::new();
    let mask = unsafe { GetLogicalDrives() };
    if mask == 0 {
        return devices;
    }

    for index in 0..26_u32 {
        if mask & (1_u32 << index) == 0 {
            continue;
        }

        let letter = (b'A' + (index as u8)) as char;
        let root = format!("{letter}:\\");
        if std::fs::metadata(&root).is_err() {
            continue;
        }

        let Some((target_kind, note)) = classify_windows_root(&root) else {
            continue;
        };

        devices.push(DeviceCandidate {
            path: PathBuf::from(root),
            target_kind,
            note,
        });
    }

    devices
}

#[cfg(not(target_os = "windows"))]
fn classify_unix_path(path: &Path) -> Option<(TargetKind, String)> {
    let text = path.to_string_lossy();
    if text.starts_with("/run/media/")
        || text.starts_with("/media/")
        || text.starts_with("/Volumes/")
    {
        return Some((
            TargetKind::UsbDisk,
            "命中可移动设备挂载目录规则".to_string(),
        ));
    }
    if text.starts_with("/mnt/") {
        return Some((TargetKind::Other, "命中 /mnt 挂载目录规则".to_string()));
    }
    if text.starts_with('/') {
        return Some((TargetKind::PcDisk, "命中系统本地路径规则".to_string()));
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn list_unix_devices() -> Vec<DeviceCandidate> {
    let mut devices = Vec::new();
    for base in ["/run/media", "/media", "/Volumes"] {
        let base_path = Path::new(base);
        if !base_path.is_dir() {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(base_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    devices.push(DeviceCandidate {
                        path,
                        target_kind: TargetKind::UsbDisk,
                        note: format!("挂载于 {base}"),
                    });
                }
            }
        }
    }

    if devices.is_empty() && Path::new("/").exists() {
        devices.push(DeviceCandidate {
            path: PathBuf::from("/"),
            target_kind: TargetKind::PcDisk,
            note: "系统根卷".to_string(),
        });
    }

    devices
}

#[cfg(test)]
mod tests {
    use super::{is_image_like_extension, looks_like_phone_source};
    use std::path::Path;

    #[test]
    fn detect_image_extension() {
        assert!(is_image_like_extension(Path::new("disk.img")));
        assert!(is_image_like_extension(Path::new("evidence.raw")));
        assert!(is_image_like_extension(Path::new("dump.vhdx")));
        assert!(!is_image_like_extension(Path::new("note.txt")));
    }

    #[test]
    fn detect_phone_path_tokens() {
        assert!(looks_like_phone_source(
            Path::new(r"C:\Users\A\AppData\Roaming\Apple Computer\MobileSync\Backup"),
            false
        ));
        assert!(looks_like_phone_source(Path::new(r"D:\Phone\DCIM"), false));
        assert!(!looks_like_phone_source(
            Path::new(r"D:\Evidence\disk.img"),
            false
        ));
    }
}
