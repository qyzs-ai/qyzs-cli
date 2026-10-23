//! 语义向量表征接口 (/api/aiCenterChat/v1/embeddings)
//!
//! 内置实测边界调度（见 tests/docs/QYZS-ANALYZE.md §5.1）：
//! - 单条输入超过模型字符边界时自动分块，响应端均值池化回单向量；
//! - 单请求物理条数不超过 256，超出自动拆分为多个请求。

use super::QyzsClient;
use crate::error::{QyzsError, QyzsResult};
use serde::{Deserialize, Serialize};

/// 单请求批量条数上限（实测 ≥256 条/请求可用，越界一律 HTTP 500）
const MAX_BATCH_ITEMS: usize = 256;

/// 各模型单条输入的字符分块阈值。
///
/// 注意单位: 网关限制的是 **token** 数 (bge-m3 为 8192), 而此处按**字符**切分。
/// 中文一字约一 token, 纯中文 12,000 字符即约 12,000 token —— 远超 8192。
/// 2026-10-21 极限测试实测: 12,000 字符的中文证据块报
/// `input_tokens=8193` 越界 (HTTP 400)。故按最坏比��� (1 字符 = 1 token) 取阈值:
/// bge-m3 / qwen3-embadding-8b 取 6,000 字符 → 最多 6,000 token, 留足余量。
/// bge-large-zh-v1.5 上下文小 (512 token 级), 仍按 500 字符。
fn char_limit_for(model: &str) -> usize {
    match model {
        "bge-large-zh-v1.5" => 500,
        // bge-m3 / qwen3-embadding-8b: 8192 token 上限 → 中文按 6,000 字符封顶
        _ => 6_000,
    }
}

/// 按 Unicode 字符数上限切分文本（空文本返回单个空串，保持 1:1 映射）
pub(crate) fn chunk_text(text: &str, limit: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    if limit == 0 {
        return vec![text.to_string()];
    }
    chars.chunks(limit).map(|c| c.iter().collect()).collect()
}

/// 规划物理请求项：返回 (物理项列表 (原输入下标, 分块文本), 各原输入的分块数)
pub(crate) fn plan_chunks(inputs: &[String], limit: usize) -> (Vec<(usize, String)>, Vec<usize>) {
    let mut items = Vec::new();
    let mut counts = Vec::with_capacity(inputs.len());
    for (idx, text) in inputs.iter().enumerate() {
        let chunks = chunk_text(text, limit);
        counts.push(chunks.len());
        for chunk in chunks {
            items.push((idx, chunk));
        }
    }
    (items, counts)
}

/// 将同一条输入各分块的向量均值池化为单一向量
pub(crate) fn mean_pool(vectors: &[Vec<f64>]) -> Vec<f64> {
    let n = vectors.len() as f64;
    let dim = vectors.first().map(Vec::len).unwrap_or(0);
    let mut pooled = vec![0.0; dim];
    for vec in vectors {
        for (acc, v) in pooled.iter_mut().zip(vec) {
            *acc += v;
        }
    }
    for acc in &mut pooled {
        *acc /= n;
    }
    pooled
}

#[derive(Debug, Clone, Serialize)]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    pub model: Option<String>,
    pub data: Vec<EmbeddingItem>,
    pub usage: Option<EmbeddingUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingItem {
    pub index: usize,
    pub embedding: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingUsage {
    pub prompt_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

impl QyzsClient {
    /// 执行向量表征请求（长文本自动分块、批量自动拆分，结果按输入顺序回填）
    pub async fn embed(
        &self,
        texts: Vec<String>,
        model_override: Option<&str>,
    ) -> QyzsResult<EmbeddingResponse> {
        let key = self.config.ensure_auth()?;
        let model = model_override
            .unwrap_or_else(|| self.config.get_default_model("embed"))
            .to_string();

        let (items, _) = plan_chunks(&texts, char_limit_for(&model));
        if items.len() == texts.len() && items.len() <= MAX_BATCH_ITEMS {
            // 常规路径：无分块且批量在界内，单请求原样透传
            let resp = self.embed_request(&model, texts, key).await?;
            self.record_embed_usage(&model, resp.usage.as_ref());
            return Ok(resp);
        }

        // 分块/拆批路径：按 ≤256 物理条分请求，再按原输入下标均值池化回填
        let mut collected: Vec<Vec<Vec<f64>>> = vec![Vec::new(); texts.len()];
        let mut total_tokens = 0u32;
        let mut prompt_tokens = 0u32;
        for batch in items.chunks(MAX_BATCH_ITEMS) {
            let batch_inputs: Vec<String> = batch.iter().map(|(_, t)| t.clone()).collect();
            let resp = self.embed_request(&model, batch_inputs, key).await?;
            if let Some(usage) = &resp.usage {
                total_tokens += usage.total_tokens.unwrap_or(0);
                prompt_tokens += usage.prompt_tokens.unwrap_or(0);
            }
            for (pos, item) in resp.data.into_iter().enumerate() {
                let slot = if item.index < batch.len() {
                    item.index
                } else {
                    pos
                };
                if let Some((idx, _)) = batch.get(slot) {
                    collected[*idx].push(item.embedding);
                }
            }
        }

        let data = collected
            .into_iter()
            .enumerate()
            .map(|(idx, vecs)| EmbeddingItem {
                index: idx,
                embedding: mean_pool(&vecs),
            })
            .collect();

        let aggregate = EmbeddingUsage {
            prompt_tokens: (prompt_tokens > 0).then_some(prompt_tokens),
            total_tokens: (total_tokens > 0).then_some(total_tokens),
        };
        self.record_embed_usage(&model, Some(&aggregate));

        Ok(EmbeddingResponse {
            model: Some(model),
            data,
            usage: Some(aggregate),
        })
    }

    /// 用量台账记录 (best-effort; 向量接口无 completion 概念, 恒记 None)
    fn record_embed_usage(&self, model: &str, usage: Option<&EmbeddingUsage>) {
        self.record_usage(
            "embed",
            model,
            usage.and_then(|u| u.prompt_tokens.map(|v| v as u64)),
            None,
            usage.and_then(|u| u.total_tokens.map(|v| v as u64)),
        );
    }

    async fn embed_request(
        &self,
        model: &str,
        input: Vec<String>,
        key: &str,
    ) -> QyzsResult<EmbeddingResponse> {
        let req = EmbeddingRequest {
            model: model.to_string(),
            input,
        };

        let resp = self
            .http
            .post("api/aiCenterChat/v1/embeddings", Some(key))
            .json(&req)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(QyzsError::Server(format!(
                "向量网关返回 HTTP {}: {}",
                status, err_text
            )));
        }

        let embed_resp: EmbeddingResponse = resp.json().await?;
        Ok(embed_resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_text_respects_char_limit() {
        let text = "a".repeat(25_000);
        let chunks = chunk_text(&text, 12_000);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].chars().count(), 12_000);
        assert_eq!(chunks[2].chars().count(), 1_000);
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn chunk_text_never_splits_multibyte_chars() {
        let text = "中".repeat(2_500);
        let chunks = chunk_text(&text, 1_000);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.chars().all(|ch| ch == '中')));
        assert_eq!(chunks.concat().chars().count(), 2_500);
    }

    #[test]
    fn chunk_text_short_and_empty_pass_through() {
        assert_eq!(chunk_text("hello", 12_000), vec!["hello".to_string()]);
        assert_eq!(chunk_text("", 12_000), vec![String::new()]);
    }

    #[test]
    fn plan_chunks_tracks_origin_indices() {
        let inputs = vec!["short".to_string(), "x".repeat(2_500)];
        let (items, counts) = plan_chunks(&inputs, 1_000);
        assert_eq!(counts, vec![1, 3]);
        assert_eq!(items.len(), 4);
        assert_eq!(items[0], (0, "short".to_string()));
        assert!(items.iter().skip(1).all(|(idx, _)| *idx == 1));
    }

    #[test]
    fn mean_pool_averages_element_wise() {
        let pooled = mean_pool(&[vec![1.0, 3.0], vec![2.0, 5.0]]);
        assert_eq!(pooled, vec![1.5, 4.0]);
        assert_eq!(mean_pool(&[vec![7.0]]), vec![7.0]);
    }

    #[test]
    fn char_limit_matches_measured_boundaries() {
        // 网关限的是 token (bge-m3 8192) 而此处按字符切分: 中文一字约一 token,
        // 故按最坏比 1:1 取 6,000 字符 (12,000 字符实测越界 input_tokens=8193)。
        assert_eq!(char_limit_for("bge-m3"), 6_000);
        assert_eq!(char_limit_for("qwen3-embadding-8b"), 6_000);
        assert_eq!(char_limit_for("bge-large-zh-v1.5"), 500);
    }
}
