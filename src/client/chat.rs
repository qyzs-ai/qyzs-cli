//! 对话与多模态接口封装 (/api/aiCenterChat/v1/chat/completions)

use super::QyzsClient;
use crate::error::{QyzsError, QyzsResult};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: MessageContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrlDetail },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageUrlDetail {
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f64,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    pub stream: bool,
    /// 流式请求附带 include_usage: 网关在最后一个 chunk (choices 为空) 回传
    /// 完整 usage (2026-10-18 实测确认)。非流式响应本就带 usage, 无需设置。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub id: Option<String>,
    pub choices: Vec<ChatChoice>,
    pub usage: Option<ChatUsage>,
}

impl ChatResponse {
    /// 取首个选择的正文文本: content 优先, 为空时回退 reasoning。
    /// 实测部分模型 (qwen3.6-27b / qwen3-VL 非流式) 会把完整回答放入
    /// reasoning 而 content 为空 (capability_matrix 记录的行为), 调用方
    /// 不应只读 content。
    pub fn primary_text(&self) -> String {
        self.choices
            .first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| {
                let content = m.content.as_deref().unwrap_or("").trim();
                if !content.is_empty() {
                    Some(content.to_string())
                } else {
                    m.reasoning.as_deref().map(str::trim).map(str::to_string)
                }
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChoice {
    pub index: usize,
    pub message: Option<ChatMessageResponse>,
    pub delta: Option<ChatDeltaResponse>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessageResponse {
    pub role: Option<String>,
    pub content: Option<String>,
    pub reasoning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatDeltaResponse {
    pub content: Option<String>,
    pub reasoning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatUsage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct ChatOptions {
    pub model: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f64>,
    pub stream: bool,
    pub show_reasoning: bool,
    pub images: Vec<String>,
    /// 多模态图片长边请求值；模型有实测失败边界时会进一步钳制。
    pub image_max_dim: Option<u32>,
}

impl QyzsClient {
    /// 执行对话与多模态请求 (带排他防御与 model-language 注入)
    pub async fn chat(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        opts: ChatOptions,
    ) -> QyzsResult<ChatResponse> {
        let req = self.build_chat_request(prompt, system_prompt, &opts)?;
        let key = self.next_api_key()?;
        let _permit = self.acquire_request_permit().await?;

        let resp = self
            .http
            .post("api/aiCenterChat/v1/chat/completions", Some(&key))
            .json(&req)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(QyzsError::Server(format!(
                "网关返回 HTTP {}: {}",
                status, err_text
            )));
        }

        let chat_resp: ChatResponse = resp.json().await?;
        self.record_chat_usage("chat", &req.model, &key, chat_resp.usage.as_ref());
        Ok(chat_resp)
    }

    /// 执行流式对话请求 (SSE 打字机输出)
    pub async fn chat_stream(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        opts: ChatOptions,
        mut on_delta: impl FnMut(&str, bool) -> QyzsResult<()>,
    ) -> QyzsResult<String> {
        let mut req = self.build_chat_request(prompt, system_prompt, &opts)?;
        req.stream = true;
        req.stream_options = Some(StreamOptions {
            include_usage: true,
        });
        let key = self.next_api_key()?;
        let _permit = self.acquire_request_permit().await?;

        let resp = self
            .http
            .post("api/aiCenterChat/v1/chat/completions", Some(&key))
            .json(&req)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(QyzsError::Server(format!(
                "网关返回 HTTP {}: {}",
                status, err_text
            )));
        }

        let mut stream = resp.bytes_stream();
        let mut full_content = String::new();
        let mut buffer = String::new();
        let mut last_usage: Option<ChatUsage> = None;

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res.map_err(|e| QyzsError::Network(format!("读取流中断: {}", e)))?;
            let text = String::from_utf8_lossy(&chunk);
            buffer.push_str(&text);

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();

                if line.is_empty() {
                    continue;
                }
                if let Some(data) = line.strip_prefix("data: ") {
                    let data = data.trim();
                    if data == "[DONE]" {
                        break;
                    }
                    if let Ok(choice_val) = serde_json::from_str::<ChatResponse>(data) {
                        // include_usage 的收尾 chunk: choices 为空, 仅携带 usage
                        if choice_val.usage.is_some() {
                            last_usage = choice_val.usage;
                        }
                        if let Some(first_choice) = choice_val.choices.first() {
                            if let Some(delta) = &first_choice.delta {
                                // 检查是否有思维链 reasoning
                                if opts.show_reasoning {
                                    if let Some(reasoning) = &delta.reasoning {
                                        if !reasoning.is_empty() {
                                            on_delta(reasoning, true)?;
                                        }
                                    }
                                }
                                // 常规内容
                                if let Some(content) = &delta.content {
                                    if !content.is_empty() {
                                        full_content.push_str(content);
                                        on_delta(content, false)?;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        self.record_chat_usage("chat", &req.model, &key, last_usage.as_ref());
        Ok(full_content)
    }

    /// 构建带有严格校验、防御规则与模型语言注入的 Chat 请求
    fn build_chat_request(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
        opts: &ChatOptions,
    ) -> QyzsResult<ChatRequest> {
        let has_images = !opts.images.is_empty();

        // 1. 决定使用哪个模型
        let model = if let Some(m) = &opts.model {
            // 防呆校验：严格排除 paddleocrvl
            if m.eq_ignore_ascii_case("paddleocrvl") {
                return Err(QyzsError::Usage(
                    crate::msg!(ChatExcludesPaddleOcr).to_string(),
                ));
            }
            m.clone()
        } else if has_images {
            "qwen3-VL-30B-A3B".to_string()
        } else {
            self.config.get_default_model("chat").to_string()
        };

        // 2. 超参级联装配
        let temperature = opts
            .temperature
            .or_else(|| self.config.get_model_param_f64(&model, "temperature"))
            .unwrap_or(0.2);

        let max_tokens = opts
            .max_tokens
            .or_else(|| {
                self.config
                    .get_model_param_i64(&model, "max_tokens")
                    .map(|v| v as u32)
            })
            .unwrap_or(4096);

        let top_p = opts
            .top_p
            .or_else(|| self.config.get_model_param_f64(&model, "top_p"));

        // 3. 构建消息体与注入 model_language
        let mut messages = Vec::new();

        // System Prompt 与语言约束注入
        let mut sys_text = system_prompt.unwrap_or("").to_string();
        match self.config.model_language.as_str() {
            "cn" | "zh" | "zh-cn" => {
                if !sys_text.is_empty() {
                    sys_text.push('\n');
                }
                sys_text.push_str("[要求：请始终使用规范流畅的简体中文进行严谨回答。]");
            }
            "en" => {
                if !sys_text.is_empty() {
                    sys_text.push('\n');
                }
                sys_text.push_str("[Constraint: Please always respond in clear, formal English.]");
            }
            _ => {}
        }

        if !sys_text.is_empty() {
            messages.push(Message {
                role: "system".to_string(),
                content: MessageContent::Text(sys_text),
                reasoning: None,
            });
        }

        // 用户消息构建 (多模态图片编码)
        if has_images {
            let mut parts = vec![ContentPart::Text {
                text: prompt.to_string(),
            }];

            for img_path in &opts.images {
                let requested =
                    crate::model_capabilities::effective_long_edge(&model, opts.image_max_dim);
                let data_url = match requested {
                    Some(max_dim) => super::ocr::preprocess_and_encode_image(img_path, max_dim)?,
                    None => encode_image_to_data_url(img_path)?,
                };
                parts.push(ContentPart::ImageUrl {
                    image_url: ImageUrlDetail { url: data_url },
                });
            }

            messages.push(Message {
                role: "user".to_string(),
                content: MessageContent::Parts(parts),
                reasoning: None,
            });
        } else {
            messages.push(Message {
                role: "user".to_string(),
                content: MessageContent::Text(prompt.to_string()),
                reasoning: None,
            });
        }

        Ok(ChatRequest {
            model,
            messages,
            temperature,
            max_tokens,
            top_p,
            stream: opts.stream,
            stream_options: None,
        })
    }
}

/// 读取本地图片并转换为 Base64 Data URL (data:image/png;base64,...)
pub fn encode_image_to_data_url(path_str: &str) -> QyzsResult<String> {
    use base64::Engine;
    let path = Path::new(path_str);
    if !path.exists() {
        return Err(QyzsError::Io(format!("图片文件不存在: {}", path_str)));
    }

    let bytes = std::fs::read(path)
        .map_err(|e| QyzsError::Io(format!("读取图片文件失败 {}: {}", path_str, e)))?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_lowercase();

    // MIME 必须与实际编码一致: 网关按 data URL 的类型解析, gif/tiff 若误标为
    // image/png 会被当作 PNG 解码而失败 (2026-10-21 多格式实测发现)
    let mime = match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "gif" => "image/gif",
        "tif" | "tiff" => "image/tiff",
        _ => "image/png",
    };

    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", mime, b64))
}
