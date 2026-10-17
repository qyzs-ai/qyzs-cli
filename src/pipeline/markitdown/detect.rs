//! markitdown 输入格式嗅探 (阶段①)
//!
//! 识别优先级：内容魔数 > 容器内部结构 (ZIP 条目名 / OLE2 目录流名) > 扩展名兜底。
//! 零新增依赖：ZIP 家族在头部字节内直接解析本地文件头条目名；
//! OLE2 (CFB) 按规范解析 FAT 链与目录项流名区分 Word/Excel/PowerPoint/Visio。

use crate::error::{QyzsError, QyzsResult};
use std::io::Read;
use std::path::Path;

/// 嗅探读取的头部字节数 (OLE2 目录链通常位于文件前部)
const SNIFF_HEAD_BYTES: u64 = 1024 * 1024;

/// PDF 魔数扫描窗口 (规范允许文件头存在少量前导字节)
const PDF_MAGIC_SCAN: usize = 1024;

/// CFB 目录链最大遍历扇区数 (防御 FAT 环)
const CFB_MAX_DIR_SECTORS: usize = 4096;

/// CFB 特殊扇区值: ENDOFCHAIN
const CFB_END_OF_CHAIN: u32 = 0xFFFF_FFFE;

const OLE2_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// 输入源格式族
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFormat {
    Pdf,
    Doc,
    Docx,
    Xls,
    Xlsx,
    Ppt,
    Pptx,
    Vsdx,
    Vs,
    Image(ImageKind),
    Html,
    Unknown,
}

/// 图像子格式 (Unspecified 仅用于 --type=image 手动覆盖场景)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Jpeg,
    Gif,
    Bmp,
    Tiff,
    Webp,
    Unspecified,
}

impl ImageKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Gif => "GIF",
            Self::Bmp => "BMP",
            Self::Tiff => "TIFF",
            Self::Webp => "WebP",
            Self::Unspecified => "未指定子格式",
        }
    }

    fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "gif" => Some(Self::Gif),
            "bmp" => Some(Self::Bmp),
            "tif" | "tiff" => Some(Self::Tiff),
            "webp" => Some(Self::Webp),
            _ => None,
        }
    }
}

impl SourceFormat {
    /// 规范键名 (--type 取值域)
    pub fn key(&self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Doc => "doc",
            Self::Docx => "docx",
            Self::Xls => "xls",
            Self::Xlsx => "xlsx",
            Self::Ppt => "ppt",
            Self::Pptx => "pptx",
            Self::Vsdx => "vsdx",
            Self::Vs => "vsd",
            Self::Image(_) => "image",
            Self::Html => "html",
            Self::Unknown => "unknown",
        }
    }

    /// 中文标签 (dry-run 展示)
    pub fn label(&self) -> String {
        match self {
            Self::Pdf => "PDF 文档".into(),
            Self::Doc => "Word 文档 (.doc, OLE2)".into(),
            Self::Docx => "Word 文档 (.docx, OOXML)".into(),
            Self::Xls => "Excel 工作簿 (.xls, OLE2)".into(),
            Self::Xlsx => "Excel 工作簿 (.xlsx, OOXML)".into(),
            Self::Ppt => "PowerPoint 演示文稿 (.ppt, 旧版二进制)".into(),
            Self::Pptx => "PowerPoint 演示文稿 (.pptx, OOXML)".into(),
            Self::Vsdx => "Visio 绘图 (.vsdx, OOXML)".into(),
            Self::Vs => "Visio 绘图 (.vsd, 旧版二进制)".into(),
            Self::Image(k) => format!("图像 ({})", k.label()),
            Self::Html => "HTML 网页".into(),
            Self::Unknown => "未知格式".into(),
        }
    }

    /// 解析 --type 手动覆盖值
    pub fn parse(s: &str) -> QyzsResult<Self> {
        match s.to_ascii_lowercase().as_str() {
            "pdf" => Ok(Self::Pdf),
            "doc" => Ok(Self::Doc),
            "docx" => Ok(Self::Docx),
            "xls" => Ok(Self::Xls),
            "xlsx" => Ok(Self::Xlsx),
            "ppt" => Ok(Self::Ppt),
            "pptx" => Ok(Self::Pptx),
            "vsd" => Ok(Self::Vs),
            "vsdx" => Ok(Self::Vsdx),
            "image" => Ok(Self::Image(ImageKind::Unspecified)),
            "html" => Ok(Self::Html),
            other => Err(QyzsError::Usage(format!(
                "未知输入格式 '{}'，可选: pdf|doc|docx|xls|xlsx|ppt|pptx|vsd|vsdx|image|html",
                other
            ))),
        }
    }
}

/// 格式判定来源
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectMethod {
    /// 内容魔数/容器结构命中
    ContentSniff,
    /// 内容未命中，按扩展名推断
    ExtensionFallback,
    /// --type 手动指定
    ManualOverride,
}

impl DetectMethod {
    pub fn label(self) -> &'static str {
        match self {
            Self::ContentSniff => "内容嗅探",
            Self::ExtensionFallback => "扩展名兜底",
            Self::ManualOverride => "手动指定",
        }
    }
}

/// 一次格式判定结果
#[derive(Debug, Clone)]
pub struct Detection {
    pub format: SourceFormat,
    pub method: DetectMethod,
    /// 判定过程中的异常信号 (如 OLE2 容器配 OOXML 扩展名 → 疑似加密文档)
    pub warning: Option<String>,
}

pub struct FormatDetector;

impl FormatDetector {
    /// 判定本地文件格式：内容嗅探优先，扩展名兜底
    pub fn detect_file(path: &Path) -> QyzsResult<Detection> {
        let head = Self::read_head(path)?;
        let content = Self::detect_bytes(&head);
        if content != SourceFormat::Unknown {
            return Ok(Detection {
                format: content,
                method: DetectMethod::ContentSniff,
                warning: None,
            });
        }

        // 内容未命中 → 扩展名兜底
        match Self::extension_format(path) {
            Some(format) => {
                // OLE2 容器但扩展名指向 OOXML：密码保护的 Office 文档以 OLE2 封装
                let warning = if head.starts_with(&OLE2_MAGIC)
                    && matches!(
                        format,
                        SourceFormat::Docx | SourceFormat::Xlsx | SourceFormat::Pptx
                    ) {
                    Some(
                        "OLE2 复合容器却携带 OOXML 扩展名：该文件可能已设置打开密码 (加密文档以 OLE2 封装)"
                            .to_string(),
                    )
                } else {
                    None
                };
                Ok(Detection {
                    format,
                    method: DetectMethod::ExtensionFallback,
                    warning,
                })
            }
            None => Ok(Detection {
                format: SourceFormat::Unknown,
                method: DetectMethod::ExtensionFallback,
                warning: None,
            }),
        }
    }

    /// 纯内容级判定 (仅凭字节，不访问文件系统)
    pub fn detect_bytes(head: &[u8]) -> SourceFormat {
        if Self::is_pdf(head) {
            return SourceFormat::Pdf;
        }
        if head.starts_with(b"PK\x03\x04") {
            return Self::zip_entry_kind(head).unwrap_or(SourceFormat::Unknown);
        }
        if head.starts_with(&OLE2_MAGIC) {
            return Self::ole_stream_kind(head).unwrap_or(SourceFormat::Unknown);
        }
        if let Some(kind) = Self::image_kind(head) {
            return SourceFormat::Image(kind);
        }
        if Self::is_html(head) {
            return SourceFormat::Html;
        }
        SourceFormat::Unknown
    }

    fn read_head(path: &Path) -> QyzsResult<Vec<u8>> {
        let file = std::fs::File::open(path)
            .map_err(|e| QyzsError::Io(format!("无法打开输入文件 {}: {}", path.display(), e)))?;
        let mut buf = Vec::new();
        file.take(SNIFF_HEAD_BYTES).read_to_end(&mut buf)?;
        Ok(buf)
    }

    fn is_pdf(head: &[u8]) -> bool {
        let scan = head.len().min(PDF_MAGIC_SCAN);
        head[..scan].windows(5).any(|w| w == b"%PDF-")
    }

    fn is_html(head: &[u8]) -> bool {
        let start = usize::from(head.starts_with(&[0xEF, 0xBB, 0xBF]));
        let window = &head[start..head.len().min(start + 512)];
        let lower: Vec<u8> = window.to_ascii_lowercase();
        lower
            .windows(14)
            .any(|w| w == b"<!doctype html")
            || lower.windows(5).any(|w| w == b"<html")
    }

    fn image_kind(head: &[u8]) -> Option<ImageKind> {
        if head.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(ImageKind::Png)
        } else if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some(ImageKind::Jpeg)
        } else if head.starts_with(b"GIF8") {
            Some(ImageKind::Gif)
        } else if head.starts_with(b"BM") && head.len() >= 6 && u32::from_le_bytes([head[2], head[3], head[4], head[5]]) != 0 {
            Some(ImageKind::Bmp)
        } else if head.starts_with(b"II\x2a\x00") || head.starts_with(b"MM\x00\x2a") {
            Some(ImageKind::Tiff)
        } else if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
            Some(ImageKind::Webp)
        } else {
            None
        }
    }

    /// 在头部字节窗口内扫描 ZIP 本地文件头 (PK\x03\x04) 条目名，
    /// 按各 OOXML 包的核心部件名判定家族。
    fn zip_entry_kind(head: &[u8]) -> Option<SourceFormat> {
        let mut i = 0usize;
        while let Some(pos) = find_subslice(&head[i..], b"PK\x03\x04") {
            let base = i + pos;
            if base + 30 > head.len() {
                break;
            }
            let name_len = u16::from_le_bytes([head[base + 26], head[base + 27]]) as usize;
            let extra_len = u16::from_le_bytes([head[base + 28], head[base + 29]]) as usize;
            let name_end = base + 30 + name_len;
            if name_end > head.len() {
                break;
            }
            let name = String::from_utf8_lossy(&head[base + 30..name_end]);
            match name.as_ref() {
                "word/document.xml" => return Some(SourceFormat::Docx),
                "xl/workbook.xml" => return Some(SourceFormat::Xlsx),
                "ppt/presentation.xml" => return Some(SourceFormat::Pptx),
                "visio/document.xml" => return Some(SourceFormat::Vsdx),
                _ => {}
            }
            // 数据段尺寸未知 (流式标志位) 时无法精确跳过，继续搜索下一个本地头
            i = (name_end + extra_len).min(head.len());
        }
        None
    }

    /// 解析 CFB (OLE2) 目录链，按特征流名判定旧版二进制格式。
    /// 结构超出头部窗口或无法解析时返回 None (交由扩展名兜底)。
    fn ole_stream_kind(head: &[u8]) -> Option<SourceFormat> {
        if head.len() < 512 {
            return None;
        }
        let sector_size = match u16::from_le_bytes([head[0x1E], head[0x1F]]) {
            9 => 512usize,
            12 => 4096,
            _ => return None,
        };
        let fat_count = u32::from_le_bytes([head[0x2C], head[0x2D], head[0x2E], head[0x2F]]) as usize;
        let dir_first = u32::from_le_bytes([head[0x30], head[0x31], head[0x32], head[0x33]]);
        let fat_first = u32::from_le_bytes([head[0x4C], head[0x4D], head[0x4E], head[0x4F]]);
        if fat_count == 0 || fat_count > 64 || dir_first == CFB_END_OF_CHAIN {
            return None;
        }

        // FAT: 位于 DIFAT[0] 指向的连续扇区
        let fat_off = sector_offset(fat_first, sector_size)?;
        let fat_len = fat_count.checked_mul(sector_size)?;
        if fat_off + fat_len > head.len() {
            return None;
        }
        let fat: Vec<u32> = head[fat_off..fat_off + fat_len]
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();

        // 遍历目录链，收集流名 (对象类型 2)
        let mut names: Vec<String> = Vec::new();
        let mut sector = dir_first;
        for _ in 0..CFB_MAX_DIR_SECTORS {
            let off = sector_offset(sector, sector_size)?;
            if off + sector_size > head.len() {
                break;
            }
            for entry in head[off..off + sector_size].chunks_exact(128) {
                if entry[66] != 2 {
                    continue; // 仅 stream 条目
                }
                let name_len =
                    u16::from_le_bytes([entry[64], entry[65]]) as usize;
                if name_len >= 4 && name_len <= 64 && name_len % 2 == 0 {
                    let units: Vec<u16> = entry[..name_len - 2]
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();
                    names.push(String::from_utf16_lossy(&units));
                }
            }
            let next = *fat.get(sector as usize)?;
            if next == CFB_END_OF_CHAIN || next >= 0xFFFF_FFFA {
                break;
            }
            sector = next;
        }

        for name in &names {
            match name.as_str() {
                "WordDocument" => return Some(SourceFormat::Doc),
                "Workbook" | "Book" => return Some(SourceFormat::Xls),
                "PowerPoint Document" => return Some(SourceFormat::Ppt),
                "VisioDocument" => return Some(SourceFormat::Vs),
                _ => {}
            }
        }
        None
    }

    fn extension_format(path: &Path) -> Option<SourceFormat> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        let format = match ext.as_str() {
            "pdf" => SourceFormat::Pdf,
            "doc" => SourceFormat::Doc,
            "docx" => SourceFormat::Docx,
            "xls" => SourceFormat::Xls,
            "xlsx" => SourceFormat::Xlsx,
            "ppt" => SourceFormat::Ppt,
            "pptx" => SourceFormat::Pptx,
            "vsd" => SourceFormat::Vs,
            "vsdx" => SourceFormat::Vsdx,
            "htm" | "html" => SourceFormat::Html,
            other => SourceFormat::Image(ImageKind::from_extension(other)?),
        };
        Some(format)
    }
}

/// 扇区 n 的文件内偏移 (文件头独占虚拟扇区 -1)
fn sector_offset(sector: u32, sector_size: usize) -> Option<usize> {
    (sector as usize)
        .checked_add(1)?
        .checked_mul(sector_size)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, content) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    /// 构造最小合法 CFB (OLE2) 容器: 扇区 0 = 目录, 扇区 1 = FAT
    fn make_ole(stream_name: &str) -> Vec<u8> {
        let ss = 512usize;
        let mut buf = vec![0u8; ss * 3];
        buf[0..8].copy_from_slice(&OLE2_MAGIC);
        buf[0x1E..0x20].copy_from_slice(&9u16.to_le_bytes());
        buf[0x2C..0x30].copy_from_slice(&1u32.to_le_bytes());
        buf[0x30..0x34].copy_from_slice(&0u32.to_le_bytes());
        buf[0x4C..0x50].copy_from_slice(&1u32.to_le_bytes());
        // FAT: 扇区 0 (目录) → 链结束; 扇区 1 (FAT 自身) → FATSECT
        buf[ss * 2..ss * 2 + 4].copy_from_slice(&CFB_END_OF_CHAIN.to_le_bytes());
        buf[ss * 2 + 4..ss * 2 + 8].copy_from_slice(&0xFFFF_FFFDu32.to_le_bytes());
        // 目录扇区: entry 0 = Root Entry (storage), entry 1 = 目标流
        let dir = &mut buf[ss..ss * 2];
        let write_entry = |entry: &mut [u8], name: &str, obj_type: u8| {
            let units: Vec<u16> = name.encode_utf16().collect();
            for (i, u) in units.iter().enumerate() {
                entry[i * 2..i * 2 + 2].copy_from_slice(&u.to_le_bytes());
            }
            let name_len = (units.len() as u16 + 1) * 2;
            entry[64..66].copy_from_slice(&name_len.to_le_bytes());
            entry[66] = obj_type;
        };
        write_entry(&mut dir[..128], "Root Entry", 5);
        write_entry(&mut dir[128..256], stream_name, 2);
        buf
    }

    #[test]
    fn test_pdf_magic() {
        assert_eq!(FormatDetector::detect_bytes(b"%PDF-1.7\njunk"), SourceFormat::Pdf);
        // 前导垃圾仍在扫描窗口内
        let prefixed = [vec![0xEF, 0xBB, 0xBF], b"%PDF-1.5 rest".to_vec()].concat();
        assert_eq!(FormatDetector::detect_bytes(&prefixed), SourceFormat::Pdf);
    }

    #[test]
    fn test_ooxml_zip_family() {
        assert_eq!(
            FormatDetector::detect_bytes(&make_zip(&[("word/document.xml", "<x/>")])),
            SourceFormat::Docx
        );
        assert_eq!(
            FormatDetector::detect_bytes(&make_zip(&[("xl/workbook.xml", "<x/>")])),
            SourceFormat::Xlsx
        );
        assert_eq!(
            FormatDetector::detect_bytes(&make_zip(&[("ppt/presentation.xml", "<x/>")])),
            SourceFormat::Pptx
        );
        assert_eq!(
            FormatDetector::detect_bytes(&make_zip(&[("visio/document.xml", "<x/>")])),
            SourceFormat::Vsdx
        );
        // 非 OOXML 的普通 zip 不误判
        assert_eq!(
            FormatDetector::detect_bytes(&make_zip(&[("random.txt", "hello")])),
            SourceFormat::Unknown
        );
    }

    #[test]
    fn test_ole2_stream_names() {
        assert_eq!(FormatDetector::detect_bytes(&make_ole("WordDocument")), SourceFormat::Doc);
        assert_eq!(FormatDetector::detect_bytes(&make_ole("Workbook")), SourceFormat::Xls);
        assert_eq!(FormatDetector::detect_bytes(&make_ole("Book")), SourceFormat::Xls);
        assert_eq!(
            FormatDetector::detect_bytes(&make_ole("PowerPoint Document")),
            SourceFormat::Ppt
        );
        assert_eq!(FormatDetector::detect_bytes(&make_ole("VisioDocument")), SourceFormat::Vs);
        // 无特征流名的 OLE2 → 未知 (交由扩展名兜底)
        assert_eq!(FormatDetector::detect_bytes(&make_ole("UnknownStream")), SourceFormat::Unknown);
    }

    #[test]
    fn test_image_magics() {
        assert_eq!(
            FormatDetector::detect_bytes(b"\x89PNG\r\n\x1a\n...."),
            SourceFormat::Image(ImageKind::Png)
        );
        assert_eq!(
            FormatDetector::detect_bytes(&[0xFF, 0xD8, 0xFF, 0xE0]),
            SourceFormat::Image(ImageKind::Jpeg)
        );
        assert_eq!(
            FormatDetector::detect_bytes(b"GIF89a...."),
            SourceFormat::Image(ImageKind::Gif)
        );
        let bmp = {
            let mut b = b"BM".to_vec();
            b.extend_from_slice(&12345u32.to_le_bytes());
            b
        };
        assert_eq!(FormatDetector::detect_bytes(&bmp), SourceFormat::Image(ImageKind::Bmp));
        assert_eq!(
            FormatDetector::detect_bytes(b"II\x2a\x00\x00"),
            SourceFormat::Image(ImageKind::Tiff)
        );
        let webp = {
            let mut b = b"RIFF".to_vec();
            b.extend_from_slice(&16u32.to_le_bytes());
            b.extend_from_slice(b"WEBP");
            b
        };
        assert_eq!(FormatDetector::detect_bytes(&webp), SourceFormat::Image(ImageKind::Webp));
    }

    #[test]
    fn test_html_detection() {
        assert_eq!(FormatDetector::detect_bytes(b"<!DOCTYPE html><html>"), SourceFormat::Html);
        assert_eq!(FormatDetector::detect_bytes(b"  <HTML><BODY>x"), SourceFormat::Html);
        let bom_html = [vec![0xEF, 0xBB, 0xBF], b"<!doctype html>x".to_vec()].concat();
        assert_eq!(FormatDetector::detect_bytes(&bom_html), SourceFormat::Html);
    }

    #[test]
    fn test_extension_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mystery.xlsx");
        std::fs::write(&path, b"not a zip at all").unwrap();
        let det = FormatDetector::detect_file(&path).unwrap();
        assert_eq!(det.format, SourceFormat::Xlsx);
        assert_eq!(det.method, DetectMethod::ExtensionFallback);

        // 加密的 Office 文档: OLE2 容器 + OOXML 扩展名 → 触发警告
        let path2 = dir.path().join("encrypted.docx");
        std::fs::write(&path2, make_ole("EncryptedPackage")).unwrap();
        let det2 = FormatDetector::detect_file(&path2).unwrap();
        assert_eq!(det2.format, SourceFormat::Docx);
        assert!(det2.warning.is_some());
    }

    #[test]
    fn test_type_parse() {
        assert_eq!(SourceFormat::parse("PDF").unwrap(), SourceFormat::Pdf);
        assert_eq!(
            SourceFormat::parse("image").unwrap(),
            SourceFormat::Image(ImageKind::Unspecified)
        );
        assert!(SourceFormat::parse("docxx").is_err());
    }

    #[test]
    fn test_real_fixtures() {
        let root = env!("CARGO_MANIFEST_DIR");
        let pdf = Path::new(root).join("tests/data/mineru/text_pdf/REF-0056_政府采购评审专家管理办法.pdf");
        let det = FormatDetector::detect_file(&pdf).unwrap();
        assert_eq!(det.format, SourceFormat::Pdf);
        assert_eq!(det.method, DetectMethod::ContentSniff);

        let docx = Path::new(root).join("tests/data/mineru/public_doc/REF-0106_项目验收前符合性审查表.docx");
        assert_eq!(FormatDetector::detect_file(&docx).unwrap().format, SourceFormat::Docx);

        let doc = Path::new(root).join("tests/data/mineru/public_doc/REF-0017_国土空间规划实施监测网络工作方案.doc");
        let det = FormatDetector::detect_file(&doc).unwrap();
        assert_eq!(det.format, SourceFormat::Doc);
        assert_eq!(det.method, DetectMethod::ContentSniff);

        let png = Path::new(root).join("tests/data/ocr/images/ref0101_p1_redhead_scan.png");
        assert_eq!(
            FormatDetector::detect_file(&png).unwrap().format,
            SourceFormat::Image(ImageKind::Png)
        );
    }
}
