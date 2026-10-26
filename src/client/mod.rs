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
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone)]
pub struct QyzsClient {
    http: HttpClient,
    config: ResolvedConfig,
    key_cursor: Arc<AtomicUsize>,
    request_limit: Arc<Semaphore>,
}

impl QyzsClient {
    pub fn new(config: ResolvedConfig) -> QyzsResult<Self> {
        let http = HttpClient::new(&config.base_url, config.timeout_seconds)?;
        Ok(Self {
            http,
            config,
            key_cursor: Arc::new(AtomicUsize::new(0)),
            request_limit: Arc::new(Semaphore::new(Semaphore::MAX_PERMITS)),
        })
    }

    pub fn config(&self) -> &ResolvedConfig {
        &self.config
    }

    pub fn http(&self) -> &HttpClient {
        &self.http
    }

    pub fn with_max_concurrency(&self, concurrency: usize) -> Self {
        let mut client = self.clone();
        client.request_limit = Arc::new(Semaphore::new(concurrency.max(1)));
        client
    }

    pub fn with_timeout_seconds(&self, timeout_seconds: u64) -> QyzsResult<Self> {
        let mut config = self.config.clone();
        config.timeout_seconds = timeout_seconds;
        Ok(Self {
            http: HttpClient::new(&config.base_url, timeout_seconds)?,
            config,
            key_cursor: Arc::clone(&self.key_cursor),
            request_limit: Arc::clone(&self.request_limit),
        })
    }

    pub fn next_api_key(&self) -> QyzsResult<String> {
        if self.config.api_keys.is_empty() {
            return self.config.ensure_auth().map(str::to_string);
        }
        let index = self.key_cursor.fetch_add(1, Ordering::Relaxed) % self.config.api_keys.len();
        Ok(self.config.api_keys[index].clone())
    }

    pub async fn acquire_request_permit(&self) -> QyzsResult<OwnedSemaphorePermit> {
        self.request_limit
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| crate::error::QyzsError::Server("请求并行度控制器已关闭".to_string()))
    }

    /// 记录一次成功调用到本地用量台账 (best-effort, 写失败静默;
    /// 自动附带凭据掩码指纹, 供多 key 分维统计, 不落明文)
    pub fn record_usage(
        &self,
        kind: &str,
        model: &str,
        api_key: &str,
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
        rec.key = Some(crate::config::Credentials::mask_key(api_key));
        usage_ledger::append_record(&rec);
    }

    /// chat/completions 系响应 (chat/ocr/asr) 的台账记录变体
    pub fn record_chat_usage(
        &self,
        kind: &str,
        model: &str,
        api_key: &str,
        usage: Option<&chat::ChatUsage>,
    ) {
        let u = usage;
        self.record_usage(
            kind,
            model,
            api_key,
            u.and_then(|u| u.prompt_tokens.map(|v| v as u64)),
            u.and_then(|u| u.completion_tokens.map(|v| v as u64)),
            u.and_then(|u| u.total_tokens.map(|v| v as u64)),
        );
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;

    #[tokio::test]
    async fn rotates_keys_and_enforces_shared_limit() {
        let mut config = crate::config::ResolvedConfig::resolve(
            Some("key-a"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        config.api_key = "key-a".to_string();
        config.api_keys = vec!["key-a".to_string(), "key-b".to_string()];
        let client = QyzsClient::new(config).unwrap().with_max_concurrency(2);
        let extended = client.with_timeout_seconds(300).unwrap();
        assert!(Arc::ptr_eq(&client.key_cursor, &extended.key_cursor));
        assert!(Arc::ptr_eq(&client.request_limit, &extended.request_limit));
        assert_eq!(extended.config.timeout_seconds, 300);
        assert_eq!(client.next_api_key().unwrap(), "key-a");
        assert_eq!(client.next_api_key().unwrap(), "key-b");
        assert_eq!(client.next_api_key().unwrap(), "key-a");

        let first = client.acquire_request_permit().await.unwrap();
        let second = client.acquire_request_permit().await.unwrap();
        assert_eq!(client.request_limit.available_permits(), 0);
        drop(first);
        assert_eq!(client.request_limit.available_permits(), 1);
        drop(second);
    }
}
