//! PDF 元数据解析与纯文本提取 (lopdf)

use crate::error::{QyzsError, QyzsResult};
use lopdf::Document;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct PdfMetadata {
    pub page_count: usize,
    pub title: Option<String>,
    pub author: Option<String>,
}

pub struct PdfExtractor;

impl PdfExtractor {
    /// 载入并提取 PDF 元数据与页数
    pub fn read_metadata(path_str: &str) -> QyzsResult<PdfMetadata> {
        let path = Path::new(path_str);
        if !path.exists() {
            return Err(QyzsError::Io(format!("PDF 文件不存在: {}", path_str)));
        }

        let doc = Document::load(path)
            .map_err(|e| QyzsError::Pdf(format!("解析 PDF 文档失败 {}: {}", path_str, e)))?;

        let pages = doc.get_pages();
        let page_count = pages.len();

        let mut title = None;
        let mut author = None;

        if let Ok(info_dict) = doc
            .trailer
            .get(b"Info")
            .and_then(|info| doc.get_object(info.as_reference()?))
        {
            if let Ok(dict) = info_dict.as_dict() {
                if let Ok(t) = dict.get(b"Title").and_then(|o| o.as_str()) {
                    title = Some(String::from_utf8_lossy(t).to_string());
                }
                if let Ok(a) = dict.get(b"Author").and_then(|o| o.as_str()) {
                    author = Some(String::from_utf8_lossy(a).to_string());
                }
            }
        }

        Ok(PdfMetadata {
            page_count,
            title,
            author,
        })
    }

    /// 提取指定页面范围的纯文本内容 (1-indexed)
    pub fn extract_text(path_str: &str, page_numbers: &[u32]) -> QyzsResult<String> {
        let path = Path::new(path_str);
        let doc = Document::load(path)
            .map_err(|e| QyzsError::Pdf(format!("读取 PDF 失败 {}: {}", path_str, e)))?;

        let text = doc
            .extract_text(page_numbers)
            .map_err(|e| QyzsError::Pdf(format!("提取 PDF 文字失败: {}", e)))?;

        Ok(text)
    }
}
