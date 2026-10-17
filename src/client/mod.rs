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
}
