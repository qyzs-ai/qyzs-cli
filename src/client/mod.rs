//! 庆阳智算中心统一网关 HTTP/TLS 客户端门面

pub mod asr;
pub mod chat;
pub mod embeddings;
pub mod http;
pub mod mineru;
pub mod ocr;
pub mod probe;
pub mod rerank;
pub mod sam;
pub mod usage_ledger;

pub use chat::{ChatOptions, ChatResponse, Message, MessageContent};
pub use embeddings::EmbeddingResponse;
pub use http::HttpClient;
pub use mineru::MineruResponse;
pub use ocr::OcrResponse;
pub use probe::GatewayProbes;
pub use rerank::{RerankItem, RerankResponse};

use crate::config::ResolvedConfig;
use crate::error::QyzsResult;

#[derive(Clone)]
pub struct QyzsClient {
    http: HttpClient,
    config: ResolvedConfig,
}

impl QyzsClient {
    pub fn new(config: ResolvedConfig) -> QyzsResult<Self> {
        let http = HttpClient::new(&config.base_url, config.timeout_seconds)?;
        Ok(Self { http, config })
    }

    pub fn config(&self) -> &ResolvedConfig {
        &self.config
    }

    pub fn http(&self) -> &HttpClient {
        &self.http
    }

    /// 记录一次成功调用到本地用量台账 (best-effort, 写失败静默;
    /// 自动附带凭据掩码指纹, 供多 key 分维统计, 不落明文)
    pub fn record_usage(
        &self,
        kind: &str,
        model: &str,
        prompt_tokens: Option<u64>,
        completion_tokens: Option<u64>,
        total_tokens: Option<u64>,
    ) {
        let mut rec = usage_ledger::UsageRecord::now(
            kind,
            model,
            prompt_tokens,
            completion_tokens,
            total_tokens,
        );
        rec.key = Some(crate::config::Credentials::mask_key(&self.config.api_key));
        usage_ledger::append_record(&rec);
    }

    /// chat/completions 系响应 (chat/ocr/asr) 的台账记录变体
    pub fn record_chat_usage(&self, kind: &str, model: &str, usage: Option<&chat::ChatUsage>) {
        let u = usage;
        self.record_usage(
            kind,
            model,
            u.and_then(|u| u.prompt_tokens.map(|v| v as u64)),
            u.and_then(|u| u.completion_tokens.map(|v| v as u64)),
            u.and_then(|u| u.total_tokens.map(|v| v as u64)),
        );
    }
}
