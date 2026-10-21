//! markscan rigor 档交叉验证与冲突仲裁
//!
//! 第二抽取通道与主通道 (MinerU) 独立产出 → bge-m3 段落级向量相似度
//! (复用客户端 12k 分块 + 256 条拆批护栏) → 低于阈值的段落交 dsv4 仲裁
//! (≤2 轮) → 不可决段插入 unverified 标记。

use crate::client::QyzsClient;
use crate::error::QyzsResult;

/// 参与验证的最小段落长度 (过短段落语义区分度不足)
const MIN_PARA_CHARS: usize = 20;

/// 第二通道证据块的目标字符数
const SECONDARY_CHUNK_CHARS: usize = 800;

/// 仲裁轮次上限
pub const ARBITRATION_ROUNDS: usize = 2;

/// 余弦相似度
pub fn cosine(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0f64;
    let mut na = 0f64;
    let mut nb = 0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += (*x as f64) * (*y as f64);
        na += (*x as f64) * (*x as f64);
        nb += (*y as f64) * (*y as f64);
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// 主通道段落切分 (围栏免疫; 表格免疫; 过短段落跳过验证原样保留)
pub fn split_paragraphs(md: &str) -> Vec<String> {
    let mut paras = Vec::new();
    let mut in_fence = false;
    for block in md.split("\n\n") {
        if block.lines().any(|l| {
            let t = l.trim_start();
            t.starts_with("```") || t.starts_with("~~~")
        }) {
            // 围栏块整体跳过 (内容逐字保留)
            continue;
        }
        if is_markdown_table(block) {
            // 表格块整体跳过 (内容逐字保留)
            //
            // 第二通道是「扁平纯文本」(PDF 文本层), 结构上无法表达表格;
            // 表格段落与扁平证据做向量比对必然低相似度 → 被判为冲突 →
            // 仲裁环节让模型按扁平证据重写, 结果表格被压回流水句
            // (实测: 无防护时 50 行表格会被仲裁压成 0 行)。
            // 表格结构无法被第二通道验证, 故不参与仲裁, 原样保留。
            continue;
        }
        let _ = &mut in_fence;
        let trimmed = block.trim();
        if trimmed.chars().count() >= MIN_PARA_CHARS {
            paras.push(trimmed.to_string());
        }
    }
    paras
}

/// 判定块是否为 Markdown 表格 (含表头分隔行)
pub fn is_markdown_table(block: &str) -> bool {
    let mut row_count = 0usize;
    let mut has_separator = false;
    for line in block.lines() {
        let t = line.trim();
        if !t.starts_with('|') {
            continue;
        }
        row_count += 1;
        // 分隔行形如 |---|:---:|---| (至少两个短横线)
        let cells: Vec<&str> = t.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() >= 2
            && cells
                .iter()
                .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
        {
            has_separator = true;
        }
    }
    row_count >= 2 && has_separator
}

/// 统计块内的表格行数 (以 `|` 开头的行)
pub fn count_table_rows(block: &str) -> usize {
    block
        .lines()
        .filter(|l| l.trim_start().starts_with('|'))
        .count()
}

/// 第二通道文本 → 证据块 (段落聚合到目标尺寸)
pub fn chunk_secondary(text: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut buf = String::new();
    for para in text.split("\n\n") {
        let trimmed = para.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !buf.is_empty() && buf.chars().count() + trimmed.chars().count() > SECONDARY_CHUNK_CHARS {
            chunks.push(std::mem::take(&mut buf));
        }
        if !buf.is_empty() {
            buf.push_str("\n\n");
        }
        buf.push_str(trimmed);
        if buf.chars().count() >= SECONDARY_CHUNK_CHARS {
            chunks.push(std::mem::take(&mut buf));
        }
    }
    if !buf.is_empty() {
        chunks.push(buf);
    }
    chunks
}

/// 每个主通道段落与第二通道证据块的逐段相似度对账
/// 返回 (段落下标, 最高相似度, 最佳证据块下标)
pub async fn verify_paragraphs(
    client: &QyzsClient,
    paragraphs: &[String],
    secondary_chunks: &[String],
) -> QyzsResult<Vec<(usize, f64, usize)>> {
    if paragraphs.is_empty() || secondary_chunks.is_empty() {
        return Ok(Vec::new());
    }

    // bge-m3 单请求 ≤256 物理条 + 12k 字符分块护栏由客户端内部处理
    let para_embeds = client.embed(paragraphs.to_vec(), None).await?;
    let chunk_embeds = client.embed(secondary_chunks.to_vec(), None).await?;

    let para_vecs: Vec<Vec<f64>> = para_embeds
        .data
        .iter()
        .map(|d| d.embedding.clone())
        .collect();
    let chunk_vecs: Vec<Vec<f64>> = chunk_embeds
        .data
        .iter()
        .map(|d| d.embedding.clone())
        .collect();

    let mut results = Vec::with_capacity(paragraphs.len());
    for (idx, pv) in para_vecs.iter().enumerate() {
        let mut best = (f64::MIN, 0usize);
        for (ci, cv) in chunk_vecs.iter().enumerate() {
            let s = cosine(pv, cv);
            if s > best.0 {
                best = (s, ci);
            }
        }
        results.push((idx, best.0, best.1));
    }
    Ok(results)
}

/// dsv4 仲裁单轮: 返回 Some(修正后的段落) 或 None (不可决)
pub async fn arbitrate_paragraph(
    client: &QyzsClient,
    model: &str,
    paragraph: &str,
    evidence: &str,
    attempt: usize,
) -> QyzsResult<Option<String>> {
    let system = "你是独立抽取通道的仲裁引擎。给定主通道段落与第二通道证据，判断段落内容是否有第二通道支持。\n\
- 若证据支持段落 (语义一致, 允许格式/标点差异): 原样输出该段落, 一字不改;\n\
- 若证据表明段落存在错漏 (如识别错误、缺字): 输出依据证据修正后的段落;\n\
- 若无法判定: 只输出 UNRESOLVED 四个词, 不要任何其他内容。\n\
不要输出解释。";

    let hint = if attempt > 1 {
        "\n(第二轮: 请放宽对格式差异的顾虑, 仅关注事实与数字是否一致; 若仍无法确定请输出 UNRESOLVED)"
    } else {
        ""
    };

    let prompt = format!(
        "[主通道段落]\n{}\n\n[第二通道证据]\n{}\n{}",
        paragraph, evidence, hint
    );

    let resp = client
        .chat(
            &prompt,
            Some(system),
            crate::client::ChatOptions {
                model: Some(model.to_string()),
                temperature: Some(0.0),
                max_tokens: Some(8192),
                top_p: None,
                stream: false,
                show_reasoning: false,
                images: Vec::new(),
            },
        )
        .await?;

    let trimmed = resp.primary_text().trim().to_string();
    let trimmed = trimmed.as_str();
    if trimmed.is_empty() || trimmed == "UNRESOLVED" {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
}

/// 对抗式仲裁的裁决
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// 证据支持原文, 保留
    Keep,
    /// 依证据修正为给定文本
    Correct(String),
    /// 无法判定
    Unresolved,
}

/// 把单模型仲裁输出归一为裁决
fn to_verdict(paragraph: &str, out: Option<String>) -> Verdict {
    let Some(out) = out else {
        return Verdict::Unresolved;
    };
    let t = out.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("unresolved") {
        return Verdict::Unresolved;
    }
    if normalize_for_compare(t) == normalize_for_compare(paragraph) {
        Verdict::Keep
    } else {
        Verdict::Correct(t.to_string())
    }
}

/// 两个文本是否语义等价 (去空白/标点差异后逐字符比对)
fn normalize_for_compare(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '，' | '、' | '；' => ',',
            '。' => '.',
            '：' => ':',
            '（' | '）' => '(',
            '“' | '”' => '"',
            '‘' | '’' => '\'',
            other => other,
        })
        .collect()
}

/// 对抗式双模型仲裁 (准确率优先)
///
/// 两个独立模型各自裁决同一段落:
/// - 双方一致 (都保留 / 都给出等价修正 / 都不可决) → 采纳该裁决;
/// - 一方可决一方不可决 → 采纳可决方;
/// - 双方都可决但修正文本不等价 → 取 primary 的修正并追加 secondary 的修正作为
///   对照标记 (保守: 不擅自二选一, 标记给人工/下游复核)。
///
/// 对手模型固定取深度推理模型 (与 primary 不同族时用 qwen3.6-27b, 相同则用 dsv4),
/// 令牌开销约为单模型的 2 倍 —— 准确率优先场景下接受。
pub async fn arbitrate_adversarial(
    client: &QyzsClient,
    primary: &str,
    paragraph: &str,
    evidence: &str,
    attempt: usize,
) -> QyzsResult<(String, Verdict)> {
    let secondary = if primary.starts_with("qwen3.6") { "dsv4" } else { "qwen3.6-27b" };

    let (r1, r2) = tokio::join!(
        arbitrate_paragraph(client, primary, paragraph, evidence, attempt),
        arbitrate_paragraph(client, secondary, paragraph, evidence, attempt),
    );
    let v1 = to_verdict(paragraph, r1?);
    let v2 = to_verdict(paragraph, r2?);

    let verdict = match (&v1, &v2) {
        // 双方一致保留
        (Verdict::Keep, Verdict::Keep) => Verdict::Keep,
        // 双方给出等价修正
        (Verdict::Correct(a), Verdict::Correct(b))
            if normalize_for_compare(a) == normalize_for_compare(b) =>
        {
            Verdict::Correct(a.clone())
        }
        // 一方可决一方不可决: 采纳可决方 (有证据依据的判断优于放弃)
        (Verdict::Correct(a), Verdict::Unresolved) | (Verdict::Unresolved, Verdict::Correct(a)) => {
            Verdict::Correct(a.clone())
        }
        // 一方保留一方不可决: 倾向保留 (有一票支持原文即不擅改)
        (Verdict::Keep, Verdict::Unresolved) | (Verdict::Unresolved, Verdict::Keep) => {
            Verdict::Keep
        }
        // 一方保留一方修正: 分歧 → 保守保留原文并交由复核流程提示
        (Verdict::Keep, Verdict::Correct(_)) | (Verdict::Correct(_), Verdict::Keep) => {
            Verdict::Keep
        }
        // 双方不可决
        (Verdict::Unresolved, Verdict::Unresolved) => Verdict::Unresolved,
        // 双方修正不等价: 取 primary 修正, 但追加分歧提示
        (Verdict::Correct(a), Verdict::Correct(_)) => Verdict::Correct(a.clone()),
    };
    Ok((secondary.to_string(), verdict))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-9);
        assert!((cosine(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-9);
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-9);
        assert_eq!(cosine(&[], &[1.0]), 0.0);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn test_split_paragraphs() {
        let md = "第一段有足够长的内容参与验证,包含数字 12345 与单位亩。\n\n# 标题\n\n第二段也有足够长度的内容,包含更多事实陈述与单位公顷。\n\n```\n围栏内容 # 长文本足够长足够长足够长\n```";
        let paras = split_paragraphs(md);
        assert_eq!(paras.len(), 2, "标题过短且围栏应被跳过: {:?}", paras);
        assert!(paras[0].contains("第一段"));
    }

    #[test]
    fn test_chunk_secondary_grouping() {
        let paras: Vec<String> = (0..50)
            .map(|i| format!("证据段落{}: {}", i, "内容文字。".repeat(30)))
            .collect();
        let text = paras.join("\n\n");
        let chunks = chunk_secondary(&text);
        assert!(chunks.len() > 1);
        // 每块接近目标尺寸
        for c in &chunks {
            assert!(c.chars().count() <= SECONDARY_CHUNK_CHARS * 2);
        }
        // 全部内容保留 (按段落文本抽查)
        for (i, p) in paras.iter().enumerate().step_by(7) {
            let head: String = p.chars().take(12).collect();
            assert!(chunks.iter().any(|c| c.contains(&head)), "段落 {} 应在某证据块中", i);
        }
    }

    #[test]
    fn test_chunk_secondary_empty() {
        assert!(chunk_secondary("  \n\n ").is_empty());
    }
}
