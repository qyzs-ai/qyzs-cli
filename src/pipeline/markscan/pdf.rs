//! markscan PDF 抽取通道 (本地按批切片 + MinerU 逐片同步抽取 + 合并)
//!
//! 切片阈值与吞吐实测校准值 (capability_matrix.toml):
//! - 同步通道实测 30 页 (0.2 MB) 7.7s 无断点, 约 0.26 s/页;
//! - 每批 15 页切片, 超 15 页即走本地切片 + 逐片聚合。
//!
//! 单片失败降级: 占位符 + manifest 警告 + 继续后续切片 (拿"绝大部分内容 +
//! 缺页提示"而非空文档)。
//!
//! 切片缝修复 (2026-10-26 重构引入): 15 页缝会切断段落与跨页表格。
//! 合并时检测缝两侧 —— 上片末行无终结标点且下片首行非结构标记 → 段落拼回;
//! 上片尾表与下片首表表头相同 → 视为同一跨缝表格, 去掉重复表头续接行。

use super::cross_verify::is_markdown_table;
use super::reflow;
use super::workspace::Workspace;
use crate::client::QyzsClient;
use crate::error::QyzsResult;
use crate::pdf::{PdfExtractor, PdfSplitter};

/// 单批切片页数
pub const SLICE_BATCH_PAGES: usize = 15;

/// 一个切片的溯源信息 (页范围 + 该片抽取产物, 供验证锚定)
#[derive(Debug, Clone)]
pub struct SliceInfo {
    pub first_page: u32,
    pub last_page: u32,
    pub md: String,
}

/// 抽取结果: 合并后的全文 + 切片溯源 (单批文档为 1 片; 其他通道为空)
#[derive(Debug, Clone, Default)]
pub struct Extracted {
    pub md: String,
    pub slices: Vec<SliceInfo>,
}

/// PDF → Markdown 抽取 (含分片状态机、切片缝修复与流式清理)
pub async fn extract(
    client: &QyzsClient,
    ws: &mut Workspace,
    pdf_path: &str,
    is_ocr: bool,
    on_progress: Option<&dyn Fn(usize, usize)>,
) -> QyzsResult<Extracted> {
    let meta = PdfExtractor::read_metadata(pdf_path)?;

    if meta.page_count <= SLICE_BATCH_PAGES {
        // 单批: 整文件直通网关 (无法分片降级, 失败如实抛)
        let md = client.parse_document(pdf_path, is_ocr).await?;
        ws.write_segment_text("document", &md)?;
        ws.complete_stage("document", "extract", &[])?;
        return Ok(Extracted {
            md,
            slices: vec![SliceInfo {
                first_page: 1,
                last_page: meta.page_count as u32,
                md: String::new(),
            }],
        });
    }

    // 超长文档: 本地切片 → 逐片抽取 → 合并 (段完成即流式清理切片二进制)
    let slices_dir = ws.slices_dir().to_string_lossy().to_string();
    let chunks = PdfSplitter::chunk(pdf_path, SLICE_BATCH_PAGES, &slices_dir)?;
    let total = chunks.len();

    let mut parts: Vec<SliceInfo> = Vec::with_capacity(total);
    for (idx, chunk_path) in chunks.iter().enumerate() {
        let segment = chunk_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("chunk")
            .to_string();
        let (first_page, last_page) = slice_page_range(idx, total, meta.page_count);

        let md = if ws.stage_done(&segment, "extract") {
            // --resume: 复用已完成片段的文本产物
            ws.read_segment_text(&segment)?
        } else {
            let chunk_str = chunk_path.to_string_lossy();
            match client.parse_document(&chunk_str, is_ocr).await {
                Ok(text) => {
                    ws.write_segment_text(&segment, &text)?;
                    ws.complete_stage(&segment, "extract", std::slice::from_ref(chunk_path))?;
                    text
                }
                Err(e) => {
                    // 单片失败降级: 占位符 + 警告 + 继续
                    let warn_msg = format!("切片 {}/{} 解析失败已跳过: {}", idx + 1, total, e);
                    let _ = ws.add_warning(warn_msg.clone());
                    format!(
                        "\n\n<!-- qyzs:skip-slice {}/{} -->\n\n_(切片 {}/{} 解析失败, 已跳过: {})_\n",
                        idx + 1,
                        total,
                        idx + 1,
                        total,
                        e
                    )
                }
            }
        };
        if let Some(progress) = on_progress {
            progress(idx + 1, total);
        }
        parts.push(SliceInfo {
            first_page,
            last_page,
            md,
        });
    }
    Ok(merge_slices(parts))
}

/// 单文件文档 (docx) → Markdown 抽取
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

/// 切片 idx (0-based) 的页范围
fn slice_page_range(idx: usize, total: usize, page_count: usize) -> (u32, u32) {
    let first = (idx * SLICE_BATCH_PAGES + 1) as u32;
    let last = if idx + 1 == total {
        page_count as u32
    } else {
        ((idx + 1) * SLICE_BATCH_PAGES) as u32
    };
    (first, last)
}

/// 合并切片, 修复切片缝 (段落拼接 / 重复表头表格续接)
fn merge_slices(parts: Vec<SliceInfo>) -> Extracted {
    let mut merged = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            merged.push_str(part.md.trim_start_matches('\n'));
            continue;
        }
        // 统一收尾: 已累积文本的尾部换行剥掉, 由连接符决定接缝形态
        merged = merged.trim_end().to_string();
        merged.push_str(&seam_connector(&merged, &part.md));
    }
    Extracted {
        md: merged,
        slices: parts,
    }
}

/// 计算切片 i-1 与 i 之间的连接文本 (含修复决策)
fn seam_connector(prev_merged: &str, next: &str) -> String {
    let pv = prev_merged.trim_end();
    let nx = next.trim_start_matches('\n');
    if pv.is_empty() || nx.is_empty() {
        return format!("\n\n{nx}");
    }
    // 围栏未闭合时不做修复 (代码块跨缝, 原样续接)
    if fence_unbalanced(pv) || fence_unbalanced(nx) {
        return format!("\n\n{nx}");
    }
    let pv_blocks: Vec<&str> = pv.split("\n\n").collect();
    let nx_blocks: Vec<&str> = nx.split("\n\n").collect();
    let pv_tail = pv_blocks.last().copied().unwrap_or("");
    let nx_head = nx_blocks.first().copied().unwrap_or("");

    // 表格续接: 尾表 + 首表且表头 (前两行) 相同 → 去重复表头 (保留下片剩余块)
    if is_markdown_table(pv_tail) && is_markdown_table(nx_head) {
        if let Some(rest) = strip_repeated_header(pv_tail, nx_head) {
            let tail_blocks = nx_blocks
                .iter()
                .skip(1)
                .copied()
                .collect::<Vec<_>>()
                .join("\n\n");
            let sep = if tail_blocks.trim().is_empty() {
                String::new()
            } else {
                format!("\n\n{tail_blocks}")
            };
            return format!("\n{rest}{sep}");
        }
        return format!("\n\n{nx}");
    }
    if is_markdown_table(pv_tail) || is_markdown_table(nx_head) {
        return format!("\n\n{nx}");
    }

    // 段落拼接: 上片末行无终结标点且下片首行非结构标记
    let pv_last = pv_tail.lines().last().unwrap_or("");
    let nx_first = nx_head.lines().next().unwrap_or("");
    if !reflow::ends_terminal(pv_last)
        && !reflow::starts_structural(nx_first)
        && !pv_last.trim().is_empty()
        && !nx_first.trim().is_empty()
    {
        // 直拼成一行的接缝: 消费下片首行, 保留其块内剩余行与后续块
        let head_rest = nx_head.lines().skip(1).collect::<Vec<_>>().join("\n");
        let mut pieces = Vec::new();
        if !head_rest.trim().is_empty() {
            pieces.push(head_rest);
        }
        pieces.extend(nx_blocks.iter().skip(1).map(|b| b.to_string()));
        let sep = if pieces.is_empty() {
            String::new()
        } else {
            format!("\n\n{}", pieces.join("\n\n"))
        };
        return format!("{nx_first}{sep}");
    }
    format!("\n\n{nx}")
}

/// 围栏开闭计数是否不平衡
fn fence_unbalanced(text: &str) -> bool {
    let n = text
        .lines()
        .filter(|l| l.trim_start().starts_with("```") || l.trim_start().starts_with("~~~"))
        .count();
    n % 2 == 1
}

/// 两表表头 (前两行) 完全相同时, 返回去掉下表表头后的续接部分
fn strip_repeated_header(pv_table: &str, nx_table: &str) -> Option<String> {
    let pv: Vec<&str> = pv_table.lines().take(2).collect();
    let nx: Vec<&str> = nx_table.lines().take(2).collect();
    if pv.is_empty() || nx.len() < 2 || pv != nx {
        return None; // 表头不同 → 视为两张不同的表, 不合并
    }
    let rest: Vec<&str> = nx_table.lines().skip(2).collect();
    if rest.is_empty() {
        return None;
    }
    Some(rest.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_slices() {
        assert_eq!(estimate_slices(0), 1);
        assert_eq!(estimate_slices(15), 1);
        assert_eq!(estimate_slices(16), 2);
        assert_eq!(estimate_slices(31), 3);
        assert_eq!(estimate_slices(300), 20);
    }

    fn slice(md: &str, first: u32, last: u32) -> SliceInfo {
        SliceInfo {
            first_page: first,
            last_page: last,
            md: md.to_string(),
        }
    }

    /// 段落被切片缝切断: 末行无终结标点 + 首行非结构标记 → 拼回
    #[test]
    fn test_seam_paragraph_join() {
        let out = merge_slices(vec![
            slice("# 标题\n\n本办法所称大型基础设施, 是指", 1, 15),
            slice("下列工程建设项目:", 16, 30),
        ]);
        assert!(
            out.md
                .contains("本办法所称大型基础设施, 是指下列工程建设项目:"),
            "应拼回一段: {}",
            out.md
        );
    }

    /// 语义边界不拼: 上片以句号收束
    #[test]
    fn test_seam_terminal_not_joined() {
        let out = merge_slices(vec![
            slice("第一条 内容。\n", 1, 15),
            slice("第二条 内容。\n", 16, 30),
        ]);
        assert!(out.md.contains("第一条 内容。\n\n第二条"));
    }

    /// 跨缝表格: 重复表头 → 去重续接
    #[test]
    fn test_seam_table_merge() {
        let out = merge_slices(vec![
            slice("|列1|列2|\n|--|--|\n|a|b|", 1, 15),
            slice("|列1|列2|\n|--|--|\n|c|d|\n\ne后续正文", 16, 30),
        ]);
        assert!(
            out.md.contains("|a|b|\n|c|d|"),
            "应去重表头续接: {}",
            out.md
        );
        assert!(!out.md.contains("|a|b|\n\n|列1|列2|"));
        assert!(out.md.contains("e后续正文"));
    }

    /// 切片溯源保留
    #[test]
    fn test_merge_keeps_slices() {
        let out = merge_slices(vec![slice("A", 1, 15), slice("B", 16, 30)]);
        assert_eq!(out.slices.len(), 2);
        assert_eq!(out.slices[0].first_page, 1);
        assert_eq!(out.slices[1].last_page, 30);
    }

    /// reflow 模块在编译层面可用 (防止误删依赖)
    #[test]
    fn test_reflow_module_linked() {
        assert!(!reflow::reflow("行一\n行二").is_empty());
    }
}
