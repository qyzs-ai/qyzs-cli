//! markscan rigor 档润色与分段 (dsv4 单轮排版, 术语一致性约束)
//!
//! 分段优先: 任何规模文档都按 24k 字符目标段处理; 每段携带标题路径上下文;
//! 全文级术语表在分段前提取, 注入每次润色调用防止跨段术语漂移。

use crate::client::{ChatOptions, QyzsClient};
use crate::error::{QyzsError, QyzsResult};

/// 分段目标字符数 (约 12k Tokens 输入)
pub const SEGMENT_TARGET_CHARS: usize = 24_000;

/// 润色输出预算 (≈2 倍于段输入的字符冗余, 上下文忠实重写不压缩)
pub const POLISH_MAX_TOKENS: u32 = 16_384;

/// 术语表条目上限
const TERM_TABLE_LIMIT: usize = 32;

/// 提取高频候选术语 (CJK n-gram 频次统计, 功能字边缘过滤)
pub fn extract_term_table(text: &str) -> Vec<String> {
    const FUNCTION_CHARS: &[char] = &[
        '的', '了', '在', '是', '和', '与', '及', '或', '等', '对', '为', '以', '于', '由', '从',
        '向', '并', '按', '其', '各', '该', '一',
    ];

    let cjk: Vec<char> = text
        .chars()
        .filter(|c| ('\u{4E00}'..='\u{9FFF}').contains(c))
        .collect();

    if cjk.len() < 8 {
        return Vec::new();
    }

    let mut counts: std::collections::HashMap<String, usize> = Default::default();
    for n in 2..=6usize {
        if cjk.len() < n {
            break;
        }
        for window in cjk.windows(n) {
            // 边缘功能字剔除 (降低无意义搭配)
            if FUNCTION_CHARS.contains(&window[0]) || FUNCTION_CHARS.contains(&window[n - 1]) {
                continue;
            }
            *counts.entry(window.iter().collect()).or_insert(0) += 1;
        }
    }

    // 频次 ≥3 的候选按 (频次×长度) 降序, 去重叠取 Top-K
    let mut candidates: Vec<(String, usize)> =
        counts.into_iter().filter(|(_, c)| *c >= 3).collect();
    candidates.sort_by_key(|a| std::cmp::Reverse(a.1 * a.0.len()));

    let mut selected: Vec<String> = Vec::new();
    for (gram, _) in candidates {
        // 高频长词已覆盖的短词不再入选 (如 "评审专家" 已选则跳过 "专家")
        if selected
            .iter()
            .any(|t| t.contains(&gram) || gram.contains(t.as_str()))
        {
            continue;
        }
        selected.push(gram);
        if selected.len() >= TERM_TABLE_LIMIT {
            break;
        }
    }
    selected
}

/// 按标题边界分段 (≤ segment_chars); 无标题文档按行聚合到段。
/// 返回 (标题路径上下文, 段文本)
pub fn segment_markdown(input: &str, segment_chars: usize) -> Vec<(String, String)> {
    if segment_chars == 0 {
        return vec![(String::new(), input.to_string())];
    }

    // 切分单元: 标题行开启新节
    let mut results: Vec<(String, String)> = Vec::new();
    let mut current_path = String::new();
    let mut buf = String::new();
    let mut in_fence = false;
    for line in input.lines() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            in_fence = !in_fence;
        }
        let is_heading = !in_fence
            && line.starts_with('#')
            && line.chars().nth(1).is_some_and(|c| c == ' ' || c == '#');

        if is_heading && !buf.trim().is_empty() {
            results.push((current_path.clone(), std::mem::take(&mut buf)));
        }
        if is_heading {
            // 标题行自身开启新节, 记录路径 (含当前标题)
            let level = line.chars().take_while(|&c| c == '#').count();
            let title = line.trim_start_matches('#').trim();
            // 简化路径: 保留层级数字前缀, 供模型感知上下文
            current_path = format!("{}> {}", "#".repeat(level.min(6)), title);
        }
        buf.push_str(line);
        buf.push('\n');

        // 超长无标题节: 按行累积到目标即切段
        if !is_heading && buf.len() >= segment_chars * 3 {
            let mut cut = buf.len();
            for (idx, _) in buf.rmatch_indices('\n') {
                if idx < segment_chars * 2 {
                    cut = idx + 1;
                    break;
                }
            }
            let head: String = buf.drain(..cut).collect();
            if !head.trim().is_empty() {
                results.push((current_path.clone(), head));
            }
        }
    }
    if !buf.trim().is_empty() {
        results.push((current_path, buf));
    }

    // 合并过小的相邻段 (≤ 目标一半时尽量凑满目标)
    let mut merged: Vec<(String, String)> = Vec::new();
    for (path, body) in results {
        if let Some((_, last_body)) = merged.last_mut() {
            if last_body.len() + body.len() <= segment_chars {
                last_body.push_str(&body);
                continue;
            }
        }
        merged.push((path, body));
    }
    merged
}

/// 单段润色 (dsv4 默认): 结构排版, 不增删信息; 术语表约束
pub async fn polish_segment(
    client: &QyzsClient,
    model: &str,
    segment: &str,
    heading_path: &str,
    term_table: &[String],
) -> QyzsResult<String> {
    let system = format!(
        "你是公文与专业文档的 Markdown 排版引擎。对输入文本做且仅做以下处理:\n\
1. 修正 Markdown 结构: 标题层级、列表标记、表格对齐、代码围栏;\n\
2. 规整空白与换行;\n\
3. 保留全部信息: 不改写句子、不增删内容、不改数字/单位/编号/日期;\n\
4. 保持原文语言与措辞风格;\n\
{}\n\
只输出排版后的 Markdown 正文, 不输出任何解释。",
        if term_table.is_empty() {
            "5. 全文术语必须保持一致。".to_string()
        } else {
            format!(
                "5. 以下术语在全文中必须保持原样、不得翻译或改写: {}。",
                term_table.join("、")
            )
        }
    );

    let prompt = if heading_path.is_empty() {
        segment.to_string()
    } else {
        format!("[当前章节上下文: {}]\n\n{}", heading_path, segment)
    };

    let resp = client
        .chat(
            &prompt,
            Some(&system),
            ChatOptions {
                model: Some(model.to_string()),
                temperature: Some(0.1),
                max_tokens: Some(POLISH_MAX_TOKENS),
                top_p: None,
                stream: false,
                show_reasoning: false,
                images: Vec::new(),
            },
        )
        .await?;

    let trimmed = resp.primary_text();
    if trimmed.is_empty() {
        return Err(QyzsError::Server(format!(
            "润色模型 '{}' 对 {} 字符的段返回了空内容 (思维链模型可能耗尽输出预算)",
            model,
            segment.len()
        )));
    }
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_term_table_extraction() {
        let text = "评审专家管理办法规定评审专家应当遵守评审专家管理要求。\
                    评审专家抽取使用评审专家库。自然资源资产清查涉及自然资源资产清查工作。";
        let terms = extract_term_table(text);
        assert!(
            terms.iter().any(|t| t.contains("评审专家")),
            "应提取高频术语, 实际: {:?}",
            terms
        );
        // 无 CJK 文本 → 空表
        assert!(extract_term_table("plain english only").is_empty());
    }

    #[test]
    fn test_segment_by_headings() {
        let doc = "# 章 A\n内容 a\n\n## 节 A1\n内容 a1 很长...".repeat(10);
        let segments = segment_markdown(&doc, 200);
        assert!(segments.len() > 1, "长文档应被分段");
        assert!(segments[0].1.starts_with("# 章 A"));
        // 标题路径非空
        assert!(!segments[0].0.is_empty() || !segments[1].0.is_empty());
    }

    #[test]
    fn test_segment_small_doc_single() {
        let doc = "# 标题\n短内容";
        let segments = segment_markdown(doc, 24_000);
        assert_eq!(segments.len(), 1);
        // 行级聚合会补齐末尾换行 (空行规一化, 内容不变)
        assert_eq!(segments[0].1.trim_end(), doc);
    }

    #[test]
    fn test_segment_zero_disables() {
        let doc = "# A\n内容".repeat(1000);
        let segments = segment_markdown(&doc, 0);
        assert_eq!(segments.len(), 1);
    }

    #[test]
    fn test_segment_oversize_flat_doc() {
        // 无标题超长文档: 按行聚合切段
        let line = "这是一行没有标题的普通文本内容。\n";
        let doc = line.repeat(500);
        let segments = segment_markdown(&doc, 1000);
        assert!(segments.len() > 1);
        for (path, body) in &segments {
            assert!(
                body.len() <= 3000 + 100,
                "段应接近目标尺寸, 实际 {}",
                body.len()
            );
            let _ = path;
        }
    }

    #[test]
    fn test_segment_respects_fence() {
        // 围栏内的 # 不是标题
        let doc = "```\n# not heading\n```\n# real\ncontent\n".repeat(20);
        let segments = segment_markdown(&doc, 200);
        // 内容能合并时不乱切
        assert!(segments
            .iter()
            .all(|(_, b)| b.contains("```") || b.contains("# real")));
    }
}
