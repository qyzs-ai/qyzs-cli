//! markitdown PDF 抽取通道 (本地按批切片 + MinerU 逐片同步抽取 + 合并)
//!
//! 切片阈值与吞吐实测校准值 (capability_matrix.toml):
//! - 同步通道实测 30 页/0.2MB 无断点，约 0.26 s/页；
//! - 每批 15 页切片，超 15 页即走本地切片 + 逐片聚合。

use super::workspace::Workspace;
use crate::client::QyzsClient;
use crate::error::QyzsResult;
use crate::pdf::{PdfExtractor, PdfSplitter};

/// 单批切片页数
pub const SLICE_BATCH_PAGES: usize = 15;

/// PDF → Markdown 抽取 (含分片状态机与流式清理；返回合并后的切片原文)
pub async fn extract(
    client: &QyzsClient,
    ws: &mut Workspace,
    pdf_path: &str,
    is_ocr: bool,
    on_progress: Option<&dyn Fn(usize, usize)>,
) -> QyzsResult<String> {
    let meta = PdfExtractor::read_metadata(pdf_path)?;

    if meta.page_count <= SLICE_BATCH_PAGES {
        // 单批: 整文件直通网关
        let md = client.parse_document(pdf_path, is_ocr).await?;
        ws.write_segment_text("document", &md)?;
        ws.complete_stage("document", "extract", &[])?;
        return Ok(md);
    }

    // 超长文档: 本地切片 → 逐片抽取 → 合并 (段完成即流式清理切片二进制)
    let slices_dir = ws.slices_dir().to_string_lossy().to_string();
    let chunks = PdfSplitter::chunk(pdf_path, SLICE_BATCH_PAGES, &slices_dir)?;
    let total = chunks.len();

    let mut merged = String::new();
    for (idx, chunk_path) in chunks.iter().enumerate() {
        let segment = chunk_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("chunk")
            .to_string();

        let md = if ws.stage_done(&segment, "extract") {
            // --resume: 复用已完成片段的文本产物
            ws.read_segment_text(&segment)?
        } else {
            let chunk_str = chunk_path.to_string_lossy();
            let text = client.parse_document(&chunk_str, is_ocr).await?;
            ws.write_segment_text(&segment, &text)?;
            ws.complete_stage(&segment, "extract", std::slice::from_ref(chunk_path))?;
            text
        };

        if let Some(progress) = on_progress {
            progress(idx + 1, total);
        }

        if idx > 0 {
            merged.push_str("\n\n");
        }
        merged.push_str(&md);
    }
    Ok(merged)
}

/// 单文件文档 (doc/docx) → Markdown 抽取
pub async fn extract_document(
    client: &QyzsClient,
    ws: &mut Workspace,
    file_path: &str,
    is_ocr: bool,
) -> QyzsResult<String> {
    if ws.stage_done("document", "extract") {
        return ws.read_segment_text("document");
    }
    let md = client.parse_document(file_path, is_ocr).await?;
    ws.write_segment_text("document", &md)?;
    ws.complete_stage("document", "extract", &[])?;
    Ok(md)
}

/// 切片批数预估 (dry-run 规划用)
pub fn estimate_slices(page_count: usize) -> usize {
    if page_count <= SLICE_BATCH_PAGES {
        1
    } else {
        page_count.div_ceil(SLICE_BATCH_PAGES)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_slices() {
        assert_eq!(estimate_slices(0), 1);
        assert_eq!(estimate_slices(1), 1);
        assert_eq!(estimate_slices(15), 1);
        assert_eq!(estimate_slices(16), 2);
        assert_eq!(estimate_slices(31), 3);
        assert_eq!(estimate_slices(300), 20);
    }
}
