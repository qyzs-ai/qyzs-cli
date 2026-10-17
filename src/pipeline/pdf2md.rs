//! PDF 结构化转 Markdown 综合流水线 (结合本地切片与 MinerU 专线)

use crate::client::QyzsClient;
use crate::error::QyzsResult;
use crate::pdf::{PdfExtractor, PdfSplitter};

pub struct Pdf2MdPipeline;

impl Pdf2MdPipeline {
    /// 执行 PDF 转 Markdown 工作流 (超长文档自动分块处理并合并)
    pub async fn process(
        client: &QyzsClient,
        pdf_path: &str,
        is_ocr: bool,
        batch_pages: usize,
    ) -> QyzsResult<String> {
        let meta = PdfExtractor::read_metadata(pdf_path)?;

        // 如果文档页数在单次批次限制之内 (<= 15 页)，直接调用 MinerU 专线
        if meta.page_count <= batch_pages {
            return client.parse_document(pdf_path, is_ocr).await;
        }

        // 超长文档：使用临时目录分块切片
        let temp_dir = tempfile::tempdir()?;
        let temp_dir_str = temp_dir.path().to_string_lossy().to_string();

        let chunks = PdfSplitter::chunk(pdf_path, batch_pages, &temp_dir_str)?;
        let mut merged_markdown = String::new();

        for (i, chunk_path) in chunks.iter().enumerate() {
            let chunk_str = chunk_path.to_string_lossy();
            let md = client.parse_document(&chunk_str, is_ocr).await?;

            if i > 0 {
                merged_markdown.push_str("\n\n---\n\n");
            }
            merged_markdown.push_str(&md);
        }

        Ok(merged_markdown)
    }
}
