//! 语音转录接口 (Qwen3-ASR-1.7B)

use super::QyzsClient;
use crate::error::{QyzsError, QyzsResult};
use base64::Engine;
use serde_json::json;
use std::path::Path;

impl QyzsClient {
    /// 执行音频文件转文字 (支持 16kHz WAV 或常规音频)
    pub async fn transcribe_audio(
        &self,
        audio_path: &str,
        model_override: Option<&str>,
    ) -> QyzsResult<String> {
        let key = self.config.ensure_auth()?;
        let model = model_override.unwrap_or_else(|| self.config.get_default_model("asr"));

        let path = Path::new(audio_path);
        if !path.exists() {
            return Err(QyzsError::Io(format!("音频文件不存在: {}", audio_path)));
        }

        let audio_bytes = std::fs::read(path)
            .map_err(|e| QyzsError::Io(format!("读取音频文件失败 {}: {}", audio_path, e)))?;

        let b64 = base64::engine::general_purpose::STANDARD.encode(&audio_bytes);
        let audio_data_url = format!("data:audio/wav;base64,{}", b64);

        let req_body = json!({
            "model": model,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "请将这段语音内容逐字转写为文字。" },
                        { "type": "audio_url", "audio_url": { "url": audio_data_url } }
                    ]
                }
            ],
            "temperature": 0.0,
            "stream": false
        });

        let resp = self
            .http
            .post("api/aiCenterChat/v1/chat/completions", Some(key))
            .json(&req_body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(QyzsError::Server(format!("ASR 语音网关返回 HTTP {}: {}", status, err_text)));
        }

        let res_json: serde_json::Value = resp.json().await?;
        let text = res_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        let usage = res_json.get("usage");
        let tok = |k: &str| usage.and_then(|u| u.get(k)).and_then(|v| v.as_u64());
        self.record_usage(
            "asr",
            model,
            tok("prompt_tokens"),
            tok("completion_tokens"),
            tok("total_tokens"),
        );

        Ok(text)
    }
}
