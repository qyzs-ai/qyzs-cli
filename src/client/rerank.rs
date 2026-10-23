//! 语义重排序接口 (/api/aiCenterChat/v1/rerank)

use super::QyzsClient;
use crate::error::{QyzsError, QyzsResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct RerankRequest {
    pub model: String,
    pub query: String,
    pub documents: Vec<String>,
    pub top_n: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankResponse {
    pub model: Option<String>,
    pub results: Vec<RerankItem>,
    /// 实测网关返回 {"total_tokens": N} (仅总量, 无 prompt/completion 拆分)
    pub usage: Option<RerankUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankUsage {
    pub total_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankItem {
    pub index: usize,
    pub relevance_score: f64,
    pub document: Option<serde_json::Value>,
}

impl QyzsClient {
    /// 执行语义重排序
    pub async fn rerank(
        &self,
        query: &str,
        documents: Vec<String>,
        model_override: Option<&str>,
        top_n: Option<usize>,
    ) -> QyzsResult<RerankResponse> {
        let key = self.config.ensure_auth()?;
        let model = model_override.unwrap_or_else(|| self.config.get_default_model("rerank"));
        let top = top_n
            .or_else(|| {
                self.config
                    .get_model_param_i64(model, "top_n")
                    .map(|v| v as usize)
            })
            .unwrap_or(5);

        let req = RerankRequest {
            model: model.to_string(),
            query: query.to_string(),
            documents,
            top_n: top,
        };

        let resp = self
            .http
            .post("api/aiCenterChat/v1/rerank", Some(key))
            .json(&req)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(QyzsError::Server(format!(
                "重排网关返回 HTTP {}: {}",
                status, err_text
            )));
        }

        let rerank_resp: RerankResponse = resp.json().await?;
        self.record_usage(
            "rerank",
            model,
            None,
            None,
            rerank_resp
                .usage
                .as_ref()
                .and_then(|u| u.total_tokens.map(|v| v as u64)),
        );
        Ok(rerank_resp)
    }
}
