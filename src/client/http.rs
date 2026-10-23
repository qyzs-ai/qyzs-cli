//! 底层 HTTP 请求封装 (reqwest + rustls-tls-webpki-roots)

use crate::error::{QyzsError, QyzsResult};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use std::time::Duration;

#[derive(Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    base_url: String,
}

impl HttpClient {
    pub fn new(base_url: &str, timeout_secs: u64) -> QyzsResult<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| QyzsError::Network(format!("初始化 HTTP 客户端失败: {}", e)))?;

        let clean_base = base_url.trim_end_matches('/').to_string();

        Ok(Self {
            client,
            base_url: clean_base,
        })
    }

    /// 构造一个「不跟随重定向」的探针客户端
    ///
    /// 网关对部分路径会 301 到不带端口的绝对地址 (如
    /// `http://<host>/ai-gateway/mineru3.2.3/`, 丢掉 `:9200`), 跟随后连到
    /// 不可达端口而挂起, 导致正常工作的专线被误判为不可用。探针场景下
    /// 3xx 本身即证明路由可达, 直接判为在线即可, 无需跟随。
    pub fn new_no_redirect(base_url: &str, timeout_secs: u64) -> QyzsResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| QyzsError::Network(format!("初始化探针 HTTP 客户端失败: {}", e)))?;

        let clean_base = base_url.trim_end_matches('/').to_string();
        Ok(Self {
            client,
            base_url: clean_base,
        })
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// 拼接子路径并附加 Bearer 鉴权
    pub fn post(&self, subpath: &str, api_key: Option<&str>) -> reqwest::RequestBuilder {
        let full_url = if subpath.starts_with("http://") || subpath.starts_with("https://") {
            subpath.to_string()
        } else {
            format!("{}/{}", self.base_url, subpath.trim_start_matches('/'))
        };

        let mut req = self.client.post(&full_url);
        if let Some(key) = api_key.filter(|k| !k.is_empty()) {
            req = req.header(AUTHORIZATION, format!("Bearer {}", key));
        }
        req
    }

    pub fn get(&self, subpath: &str, api_key: Option<&str>) -> reqwest::RequestBuilder {
        let full_url = if subpath.starts_with("http://") || subpath.starts_with("https://") {
            subpath.to_string()
        } else {
            format!("{}/{}", self.base_url, subpath.trim_start_matches('/'))
        };

        let mut req = self.client.get(&full_url);
        if let Some(key) = api_key.filter(|k| !k.is_empty()) {
            req = req.header(AUTHORIZATION, format!("Bearer {}", key));
        }
        req
    }
}
