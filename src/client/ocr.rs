//! 公文 OCR 与版面文字提取 (/api/aiCenterChat/v1/chat/completions with paddleocrvl)

use super::chat::{encode_image_to_data_url, ChatRequest, ChatResponse, ContentPart, ImageUrlDetail, Message, MessageContent};
use super::QyzsClient;
use crate::error::{QyzsError, QyzsResult};
use base64::Engine;
use image::GenericImageView;
use std::io::Cursor;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct OcrResponse {
    pub model: String,
    pub text: String,
    pub raw_response: ChatResponse,
}

impl QyzsClient {
    /// 执行 OCR 识别 (内置 800px 自动等比缩放预处理)
    pub async fn ocr(
        &self,
        image_path: &str,
        prompt_override: Option<&str>,
        max_dim: Option<u32>,
    ) -> QyzsResult<OcrResponse> {
        let key = self.config.ensure_auth()?;
        let model = self.config.get_default_model("ocr");

        // 1. 智能预处理：执行长边等比缩放至 <= 800px (规避 FastDeploy 2048 崩溃)
        let max_dimension = max_dim
            .or_else(|| self.config.get_model_param_i64(model, "max_dim").map(|v| v as u32))
            .unwrap_or(800);

        let data_url = preprocess_and_encode_image(image_path, max_dimension)?;

        // 2. 组装 OCR 提示词
        let prompt_text = prompt_override.unwrap_or("提取图中的所有文字内容，保留原有版面段落结构。");

        let messages = vec![Message {
            role: "user".to_string(),
            content: MessageContent::Parts(vec![
                ContentPart::Text {
                    text: prompt_text.to_string(),
                },
                ContentPart::ImageUrl {
                    image_url: ImageUrlDetail { url: data_url },
                },
            ]),
            reasoning: None,
        }];

        let req = ChatRequest {
            model: model.to_string(),
            messages,
            temperature: 0.0,
            max_tokens: 2048,
            top_p: None,
            stream: false,
        };

        let resp = self
            .http
            .post("api/aiCenterChat/v1/chat/completions", Some(key))
            .json(&req)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(QyzsError::Server(format!("OCR 网关返回 HTTP {}: {}", status, err_text)));
        }

        let chat_resp: ChatResponse = resp.json().await?;
        let text = chat_resp
            .choices
            .first()
            .and_then(|c| c.message.as_ref())
            .and_then(|m| m.content.as_ref())
            .cloned()
            .unwrap_or_default();

        Ok(OcrResponse {
            model: model.to_string(),
            text,
            raw_response: chat_resp,
        })
    }
}

/// 纯内存预处理图像：若长边超过 max_dim，等比下采样缩放并转为 Base64 Data URL
pub fn preprocess_and_encode_image(path_str: &str, max_dim: u32) -> QyzsResult<String> {
    let path = Path::new(path_str);
    if !path.exists() {
        return Err(QyzsError::Io(format!("图片文件不存在: {}", path_str)));
    }

    let bytes = std::fs::read(path)
        .map_err(|e| QyzsError::Io(format!("读取图片文件失败 {}: {}", path_str, e)))?;

    // 尝试解码图片并探测尺寸
    let img = match image::load_from_memory(&bytes) {
        Ok(i) => i,
        Err(_) => {
            // 如果无法用常规格式解析，原样回退 Base64 编码
            return encode_image_to_data_url(path_str);
        }
    };

    let (width, height) = img.dimensions();
    let longest_edge = width.max(height);

    if longest_edge <= max_dim {
        // 无需缩放，直接返回原始 Data URL
        return encode_image_to_data_url(path_str);
    }

    // 执行高质量 Lanczos3 等比缩放
    let scale = max_dim as f32 / longest_edge as f32;
    let new_width = (width as f32 * scale).round() as u32;
    let new_height = (height as f32 * scale).round() as u32;

    let resized = img.resize_exact(new_width, new_height, image::imageops::FilterType::Lanczos3);

    let mut buffer = Cursor::new(Vec::new());
    resized
        .write_to(&mut buffer, image::ImageFormat::Png)
        .map_err(|e| QyzsError::Io(format!("缩放压缩图片失败: {}", e)))?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(buffer.into_inner());
    Ok(format!("data:image/png;base64,{}", b64))
}
