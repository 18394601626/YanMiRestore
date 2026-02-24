//! 文件雕刻模块：按签名在镜像或原始卷中定位可恢复片段。

use std::io::Read;

use crate::config;
use crate::model::{CarveResult, RecoverableItem, ScanRequest};

const SQLITE_HEADER: &[u8] = b"SQLite format 3\0";
const ZIP_LOCAL_FILE_HEADER: &[u8] = b"PK\x03\x04";
const ZIP_EOCD: &[u8] = b"PK\x05\x06";
const MP4_FTYP: &[u8] = b"ftyp";

fn max_scan_bytes() -> usize {
    config::settings().carver.max_scan_bytes.max(1024 * 1024)
}

fn max_items_per_signature() -> usize {
    config::settings().carver.max_items_per_signature.max(1)
}

fn sqlite_fallback_size_bytes() -> usize {
    config::settings()
        .carver
        .sqlite_fallback_size_bytes
        .max(4 * 1024)
}

fn zip_search_window_bytes() -> usize {
    config::settings().carver.zip_search_window_bytes.max(1024)
}

fn mp4_search_window_bytes() -> usize {
    config::settings().carver.mp4_search_window_bytes.max(1024)
}

pub trait FileCarver {
    fn carve(&self, request: &ScanRequest) -> CarveResult;
}

#[derive(Debug, Default)]
pub struct SignatureCarver;

impl FileCarver for SignatureCarver {
    fn carve(&self, request: &ScanRequest) -> CarveResult {
        if !request.include_carving {
            return CarveResult {
                enabled: false,
                signatures: Vec::new(),
                carved_candidates: 0,
                notes: vec!["本次任务未启用签名雕刻。".to_string()],
                items: Vec::new(),
            };
        }

        if request.source.is_dir() {
            return CarveResult {
                enabled: true,
                signatures: supported_signatures(),
                carved_candidates: 0,
                notes: vec!["签名雕刻需要原始镜像/设备文件，目录类型源已跳过。".to_string()],
                items: Vec::new(),
            };
        }

        let mut notes = Vec::new();
        let bytes = match read_source_prefix(&request.source, &mut notes) {
            Some(value) => value,
            None => {
                return CarveResult {
                    enabled: true,
                    signatures: supported_signatures(),
                    carved_candidates: 0,
                    notes,
                    items: Vec::new(),
                };
            }
        };

        let mut items = Vec::new();
        for spec in signature_specs() {
            let (mut found, truncated) = carve_signature(&bytes, &spec);
            if truncated {
                notes.push(format!(
                    "签名 {} 命中数量达到上限 {}。",
                    spec.ext,
                    max_items_per_signature()
                ));
            }
            items.append(&mut found);
        }

        let extra_detectors: [(&str, fn(&[u8]) -> (Vec<RecoverableItem>, bool)); 3] = [
            ("sqlite", carve_sqlite),
            ("office-zip", carve_zip_office),
            ("mp4", carve_mp4),
        ];
        for (label, detector) in extra_detectors {
            let (mut found, truncated) = detector(&bytes);
            if truncated {
                notes.push(format!(
                    "签名 {label} 命中数量达到上限 {}。",
                    max_items_per_signature()
                ));
            }
            items.append(&mut found);
        }

        if items.is_empty() {
            notes.push("在当前扫描范围内未发现签名命中。".to_string());
        }

        CarveResult {
            enabled: true,
            signatures: supported_signatures(),
            carved_candidates: items.len() as u64,
            notes,
            items,
        }
    }
}

#[derive(Clone, Copy)]
struct SignatureSpec {
    kind: &'static str,
    ext: &'static str,
    header: &'static [u8],
    footer: &'static [u8],
    min_size: usize,
    max_size: usize,
}

fn signature_specs() -> [SignatureSpec; 3] {
    let config = &config::settings().carver;
    let (jpeg_min_size, jpeg_max_size) = normalize_size_range(
        config.jpeg_min_size_bytes,
        config.jpeg_max_size_bytes,
        1_024,
        32 * 1024 * 1024,
    );
    let (png_min_size, png_max_size) = normalize_size_range(
        config.png_min_size_bytes,
        config.png_max_size_bytes,
        256,
        32 * 1024 * 1024,
    );
    let (pdf_min_size, pdf_max_size) = normalize_size_range(
        config.pdf_min_size_bytes,
        config.pdf_max_size_bytes,
        1_024,
        128 * 1024 * 1024,
    );

    [
        SignatureSpec {
            kind: "jpeg",
            ext: "jpg",
            header: b"\xFF\xD8\xFF",
            footer: b"\xFF\xD9",
            min_size: jpeg_min_size,
            max_size: jpeg_max_size,
        },
        SignatureSpec {
            kind: "png",
            ext: "png",
            header: b"\x89PNG\r\n\x1A\n",
            footer: b"IEND\xAE\x42\x60\x82",
            min_size: png_min_size,
            max_size: png_max_size,
        },
        SignatureSpec {
            kind: "pdf",
            ext: "pdf",
            header: b"%PDF-",
            footer: b"%%EOF",
            min_size: pdf_min_size,
            max_size: pdf_max_size,
        },
    ]
}

fn normalize_size_range(
    min_size: usize,
    max_size: usize,
    default_min_size: usize,
    default_max_size: usize,
) -> (usize, usize) {
    let min_size = if min_size == 0 {
        default_min_size
    } else {
        min_size
    }
    .max(1);
    let max_size = if max_size == 0 {
        default_max_size
    } else {
        max_size
    }
    .max(min_size);
    (min_size, max_size)
}

fn supported_signatures() -> Vec<String> {
    let mut out: Vec<String> = signature_specs()
        .iter()
        .map(|spec| spec.ext.to_string())
        .collect();
    out.push("sqlite".to_string());
    out.push("docx".to_string());
    out.push("xlsx".to_string());
    out.push("mp4".to_string());
    out
}

fn read_source_prefix(source: &std::path::Path, notes: &mut Vec<String>) -> Option<Vec<u8>> {
    let mut file = match std::fs::File::open(source) {
        Ok(value) => value,
        Err(error) => {
            notes.push(format!("打开源文件失败：{error}"));
            return None;
        }
    };

    let metadata = std::fs::metadata(source).ok();
    let scan_limit = max_scan_bytes();
    let read_len = metadata
        .as_ref()
        .map(|value| std::cmp::min(value.len(), scan_limit as u64) as usize)
        .unwrap_or(scan_limit);

    if let Some(value) = &metadata {
        if value.len() > scan_limit as u64 {
            notes.push(format!(
                "源文件大于 {} MiB，本次仅扫描前 {} MiB。",
                scan_limit / (1024 * 1024),
                scan_limit / (1024 * 1024)
            ));
        }
    } else {
        notes.push(format!(
            "无法读取源文件元数据，已按上限读取前 {} MiB。",
            scan_limit / (1024 * 1024)
        ));
    }

    if read_len == 0 {
        notes.push("源文件为空。".to_string());
        return None;
    }

    let mut buffer = vec![0_u8; read_len];
    let mut total_read = 0_usize;
    while total_read < read_len {
        match file.read(&mut buffer[total_read..]) {
            Ok(0) => break,
            Ok(n) => total_read += n,
            Err(error) => {
                notes.push(format!("读取源文件字节失败：{error}"));
                return None;
            }
        }
    }
    buffer.truncate(total_read);

    if buffer.is_empty() {
        notes.push("源文件为空。".to_string());
        return None;
    }

    Some(buffer)
}

fn carve_signature(data: &[u8], spec: &SignatureSpec) -> (Vec<RecoverableItem>, bool) {
    let mut items = Vec::new();
    let mut cursor = 0_usize;
    let mut truncated = false;

    while cursor + spec.header.len() <= data.len() {
        let Some(rel_start) = find_subslice(&data[cursor..], spec.header) else {
            break;
        };
        let start = cursor + rel_start;
        let search_start = start + spec.header.len();
        let search_end = std::cmp::min(start + spec.max_size, data.len());
        if search_start >= search_end {
            break;
        }

        let Some(rel_end) = find_subslice(&data[search_start..search_end], spec.footer) else {
            cursor = start + 1;
            continue;
        };

        let end = search_start + rel_end + spec.footer.len();
        let size = end.saturating_sub(start);
        if size < spec.min_size {
            cursor = start + 1;
            continue;
        }

        let ordinal = items.len() + 1;
        items.push(RecoverableItem {
            id: format!("carve-{}-{ordinal:05}", spec.ext),
            category: "signature-carved".to_string(),
            confidence: 0.85,
            note: format!(
                "{} 签名命中，偏移 {}，估计长度 {} 字节。",
                spec.kind, start, size
            ),
            suggested_name: format!("carved_{}_{start}.{}", ordinal, spec.ext),
            source_path: None,
            source_offset: Some(start as u64),
            size_bytes: Some(size as u64),
            source_segments: Vec::new(),
        });

        if items.len() >= max_items_per_signature() {
            truncated = true;
            break;
        }

        cursor = end;
    }

    (items, truncated)
}

fn carve_sqlite(data: &[u8]) -> (Vec<RecoverableItem>, bool) {
    let mut items = Vec::new();
    let mut cursor = 0_usize;
    let mut truncated = false;

    while cursor + SQLITE_HEADER.len() <= data.len() {
        let Some(rel_start) = find_subslice(&data[cursor..], SQLITE_HEADER) else {
            break;
        };
        let start = cursor + rel_start;
        let Some(header) = data.get(start..start + 100) else {
            break;
        };

        let page_size_raw = read_be_u16(header, 16).unwrap_or(0);
        let page_size = if page_size_raw == 1 {
            65_536_u64
        } else {
            u64::from(page_size_raw)
        };
        let page_count = read_be_u32(header, 28).unwrap_or(0) as u64;
        let mut size = page_size.saturating_mul(page_count);
        if size == 0 {
            size = sqlite_fallback_size_bytes() as u64;
        }
        let max_end = std::cmp::min(start + size as usize, data.len());
        let size = max_end.saturating_sub(start);

        let ordinal = items.len() + 1;
        items.push(RecoverableItem {
            id: format!("carve-sqlite-{ordinal:05}"),
            category: "signature-carved".to_string(),
            confidence: 0.88,
            note: format!("sqlite 签名命中，偏移 {}，估计长度 {} 字节。", start, size),
            suggested_name: format!("carved_{}_{start}.sqlite", ordinal),
            source_path: None,
            source_offset: Some(start as u64),
            size_bytes: Some(size as u64),
            source_segments: Vec::new(),
        });

        if items.len() >= max_items_per_signature() {
            truncated = true;
            break;
        }
        cursor = start + 1;
    }

    (items, truncated)
}

fn carve_zip_office(data: &[u8]) -> (Vec<RecoverableItem>, bool) {
    let mut items = Vec::new();
    let mut cursor = 0_usize;
    let mut truncated = false;
    let search_limit = zip_search_window_bytes();

    while cursor + ZIP_LOCAL_FILE_HEADER.len() <= data.len() {
        let Some(rel_start) = find_subslice(&data[cursor..], ZIP_LOCAL_FILE_HEADER) else {
            break;
        };
        let start = cursor + rel_start;
        let search_end = std::cmp::min(start + search_limit, data.len());
        let Some(rel_eocd) = find_subslice(&data[start..search_end], ZIP_EOCD) else {
            cursor = start + 1;
            continue;
        };
        let end = start + rel_eocd + ZIP_EOCD.len();
        let window = &data[start..end];
        let is_office = window
            .windows("[Content_Types].xml".len())
            .any(|w| w == b"[Content_Types].xml");
        if !is_office {
            cursor = start + 1;
            continue;
        }

        let ext = if window.windows(3).any(|w| w == b"xl/") {
            "xlsx"
        } else if window.windows(5).any(|w| w == b"word/") {
            "docx"
        } else {
            "docx"
        };

        let size = end.saturating_sub(start);
        let ordinal = items.len() + 1;
        items.push(RecoverableItem {
            id: format!("carve-{ext}-{ordinal:05}"),
            category: "signature-carved".to_string(),
            confidence: 0.86,
            note: format!(
                "{ext} 压缩包签名命中，偏移 {}，估计长度 {} 字节。",
                start, size
            ),
            suggested_name: format!("carved_{}_{start}.{ext}", ordinal),
            source_path: None,
            source_offset: Some(start as u64),
            size_bytes: Some(size as u64),
            source_segments: Vec::new(),
        });

        if items.len() >= max_items_per_signature() {
            truncated = true;
            break;
        }
        cursor = end;
    }

    (items, truncated)
}

fn carve_mp4(data: &[u8]) -> (Vec<RecoverableItem>, bool) {
    let mut items = Vec::new();
    let mut cursor = 0_usize;
    let mut truncated = false;
    let search_limit = mp4_search_window_bytes();

    while cursor + 12 <= data.len() {
        let Some(rel_ftyp) = find_subslice(&data[cursor..], MP4_FTYP) else {
            break;
        };
        let ftyp_pos = cursor + rel_ftyp;
        if ftyp_pos < 4 {
            cursor = ftyp_pos + 1;
            continue;
        }
        let start = ftyp_pos - 4;

        let search_end = std::cmp::min(start + search_limit, data.len());
        let Some((end, has_payload)) = parse_mp4_box_range(data, start, search_end) else {
            cursor = ftyp_pos + 1;
            continue;
        };
        if !has_payload || end <= start {
            cursor = ftyp_pos + 1;
            continue;
        }

        let size = end - start;
        let ordinal = items.len() + 1;
        items.push(RecoverableItem {
            id: format!("carve-mp4-{ordinal:05}"),
            category: "signature-carved".to_string(),
            confidence: 0.84,
            note: format!("mp4 盒结构命中，偏移 {}，估计长度 {} 字节。", start, size),
            suggested_name: format!("carved_{}_{start}.mp4", ordinal),
            source_path: None,
            source_offset: Some(start as u64),
            size_bytes: Some(size as u64),
            source_segments: Vec::new(),
        });

        if items.len() >= max_items_per_signature() {
            truncated = true;
            break;
        }
        cursor = end;
    }

    (items, truncated)
}

fn parse_mp4_box_range(data: &[u8], start: usize, search_end: usize) -> Option<(usize, bool)> {
    if start + 8 > search_end {
        return None;
    }

    let mut cursor = start;
    let mut saw_ftyp = false;
    let mut has_payload = false;

    while cursor + 8 <= search_end {
        let size32 = read_be_u32(data, cursor)? as u64;
        let box_type = data.get(cursor + 4..cursor + 8)?;

        let (box_size, header_size) = if size32 == 1 {
            let size64 = read_be_u64(data, cursor + 8)?;
            (size64 as usize, 16_usize)
        } else if size32 == 0 {
            (search_end.saturating_sub(cursor), 8_usize)
        } else {
            (size32 as usize, 8_usize)
        };

        if box_size < header_size {
            return None;
        }
        let next = cursor.checked_add(box_size)?;
        if next > search_end {
            return None;
        }

        if box_type == MP4_FTYP {
            saw_ftyp = true;
        } else if box_type == b"moov" || box_type == b"mdat" || box_type == b"moof" {
            has_payload = true;
        }

        cursor = next;
        if saw_ftyp && has_payload {
            return Some((cursor, true));
        }
    }

    if saw_ftyp && cursor > start {
        Some((cursor, has_payload))
    } else {
        None
    }
}

fn read_be_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset + 2)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_be_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset + 4)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_be_u64(data: &[u8], offset: usize) -> Option<u64> {
    let bytes = data.get(offset..offset + 8)?;
    Some(u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::{carve_mp4, carve_sqlite, carve_zip_office};

    #[test]
    fn carve_sqlite_from_header() {
        let mut data = vec![0_u8; 8192];
        let start = 512;
        data[start..start + 16].copy_from_slice(b"SQLite format 3\0");
        data[start + 16..start + 18].copy_from_slice(&4096_u16.to_be_bytes());
        data[start + 28..start + 32].copy_from_slice(&2_u32.to_be_bytes());

        let (items, truncated) = carve_sqlite(&data);
        assert!(!truncated);
        assert_eq!(items.len(), 1);
        assert!(items[0].suggested_name.ends_with(".sqlite"));
        assert_eq!(items[0].source_offset, Some(start as u64));
    }

    #[test]
    fn carve_zip_office_docx() {
        let mut data = vec![0_u8; 16384];
        let start = 1024;
        data[start..start + 4].copy_from_slice(b"PK\x03\x04");
        let body = b"[Content_Types].xmlword/document.xml";
        data[start + 4..start + 4 + body.len()].copy_from_slice(body);
        let eocd = start + 512;
        data[eocd..eocd + 4].copy_from_slice(b"PK\x05\x06");

        let (items, truncated) = carve_zip_office(&data);
        assert!(!truncated);
        assert_eq!(items.len(), 1);
        assert!(items[0].suggested_name.ends_with(".docx"));
        assert_eq!(items[0].source_offset, Some(start as u64));
    }

    #[test]
    fn carve_mp4_box_sequence() {
        let mut data = Vec::new();

        data.extend_from_slice(&24_u32.to_be_bytes());
        data.extend_from_slice(b"ftyp");
        data.extend_from_slice(b"isom");
        data.extend_from_slice(&[0_u8; 12]);

        data.extend_from_slice(&16_u32.to_be_bytes());
        data.extend_from_slice(b"mdat");
        data.extend_from_slice(&[0_u8; 8]);

        let (items, truncated) = carve_mp4(&data);
        assert!(!truncated);
        assert_eq!(items.len(), 1);
        assert!(items[0].suggested_name.ends_with(".mp4"));
        assert_eq!(items[0].source_offset, Some(0));
    }
}
