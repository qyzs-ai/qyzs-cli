//! PDF 页面拆分与分块切片 (基于 lopdf)

use crate::error::{QyzsError, QyzsResult};
use lopdf::Document;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub struct PdfSplitter;

impl PdfSplitter {
    /// 解析页码范围表达式 (如 "1-3,5,7-9", "1-")，返回升序且去重的页码列表 (1-indexed)
    pub fn parse_page_range(range_str: &str, total_pages: u32) -> QyzsResult<Vec<u32>> {
        let mut selected = BTreeSet::new();

        for part in range_str.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            if let Some((start_str, end_str)) = part.split_once('-') {
                let start = if start_str.trim().is_empty() {
                    1
                } else {
                    start_str
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| QyzsError::Usage(format!("无效的起始页码: '{}'", start_str)))?
                };

                let end = if end_str.trim().is_empty() {
                    total_pages
                } else {
                    end_str
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| QyzsError::Usage(format!("无效的结束页码: '{}'", end_str)))?
                };

                if start == 0 || end == 0 {
                    return Err(QyzsError::Usage("页码必须从 1 开始计数".to_string()));
                }
                if start > end {
                    return Err(QyzsError::Usage(format!("页码范围起始 '{}' 不能大于结束 '{}'", start, end)));
                }

                for p in start..=end.min(total_pages) {
                    selected.insert(p);
                }
            } else {
                let p = part
                    .parse::<u32>()
                    .map_err(|_| QyzsError::Usage(format!("无效的页码: '{}'", part)))?;
                if p == 0 {
                    return Err(QyzsError::Usage("页码必须从 1 开始计数".to_string()));
                }
                if p <= total_pages {
                    selected.insert(p);
                }
            }
        }

        if selected.is_empty() {
            return Err(QyzsError::Usage("未选定任何有效页码".to_string()));
        }

        Ok(selected.into_iter().collect())
    }

    /// 将选定页码提取并保存为新的 PDF 文件
    pub fn split(input_path: &str, pages_to_keep: &[u32], output_path: &str) -> QyzsResult<()> {
        let in_p = Path::new(input_path);
        if !in_p.exists() {
            return Err(QyzsError::Io(format!("待拆分 PDF 文件不存在: {}", input_path)));
        }

        let mut doc = Document::load(in_p)
            .map_err(|e| QyzsError::Pdf(format!("加载 PDF 失败: {}", e)))?;

        let all_pages: Vec<u32> = doc.get_pages().keys().cloned().collect();
        let keep_set: BTreeSet<u32> = pages_to_keep.iter().cloned().collect();

        // 找出所有需要删除的页码
        let to_delete: Vec<u32> = all_pages
            .into_iter()
            .filter(|p| !keep_set.contains(p))
            .collect();

        if !to_delete.is_empty() {
            doc.delete_pages(&to_delete);
            doc.prune_objects();
        }

        let out_p = Path::new(output_path);
        if let Some(parent) = out_p.parent() {
            std::fs::create_dir_all(parent)?;
        }

        doc.save(out_p)
            .map_err(|e| QyzsError::Pdf(format!("保存拆分后的 PDF 失败 {}: {}", output_path, e)))?;

        Ok(())
    }

    /// 按固定批次大小（如 MinerU 限制的 15 页）切片拆分大文档
    pub fn chunk(input_path: &str, chunk_size: usize, output_dir: &str) -> QyzsResult<Vec<PathBuf>> {
        let in_p = Path::new(input_path);
        let doc = Document::load(in_p)
            .map_err(|e| QyzsError::Pdf(format!("加载 PDF 失败: {}", e)))?;

        let total_pages = doc.get_pages().len();
        if total_pages == 0 {
            return Err(QyzsError::Pdf("PDF 不包含任何有效页面".to_string()));
        }

        let stem = in_p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("document");

        let out_dir_p = Path::new(output_dir);
        std::fs::create_dir_all(out_dir_p)?;

        let mut created_files = Vec::new();
        let mut start_page = 1;

        while start_page <= total_pages {
            let end_page = (start_page + chunk_size - 1).min(total_pages);
            let pages: Vec<u32> = (start_page as u32..=end_page as u32).collect();

            let chunk_name = format!("{}_part_{:02}_to_{:02}.pdf", stem, start_page, end_page);
            let chunk_path = out_dir_p.join(chunk_name);

            Self::split(input_path, &pages, chunk_path.to_str().unwrap_or_default())?;
            created_files.push(chunk_path);

            start_page = end_page + 1;
        }

        Ok(created_files)
    }
}
