//! MinerU 3.2.3 复杂文档结构化解析专线 (/ai-gateway/mineru3.2.3/file_parse)

use super::QyzsClient;
use crate::error::{QyzsError, QyzsResult};
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MineruResponse {
    pub code: Option<i32>,
    pub msg: Option<String>,
    pub data: Option<MineruData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MineruData {
    pub markdown: Option<String>,
    pub content: Option<String>,
    pub pages: Option<usize>,
}

/// 任务型响应 (网关实测形态): {status, results: {"<文件名>": {md_content}}}
#[derive(Debug, Clone, Deserialize)]
struct MineruTaskResponse {
    status: Option<String>,
    error: Option<serde_json::Value>,
    results: Option<std::collections::BTreeMap<String, MineruTaskResult>>,
}

#[derive(Debug, Clone, Deserialize)]
struct MineruTaskResult {
    md_content: Option<String>,
}

impl QyzsClient {
    /// 上传文档文件到 MinerU 专线进行结构化解析并提取 Markdown
    pub async fn parse_document(
        &self,
        file_path: &str,
        is_ocr: bool,
    ) -> QyzsResult<String> {
        let key = self.config.ensure_auth()?;
        let path = Path::new(file_path);
        if !path.exists() {
            return Err(QyzsError::Io(format!("待解析文件不存在: {}", file_path)));
        }

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document.pdf")
            .to_string();

        let file_bytes = std::fs::read(path)
            .map_err(|e| QyzsError::Io(format!("读取待解析文件失败 {}: {}", file_path, e)))?;

        let part = Part::bytes(file_bytes)
            .file_name(file_name)
            .mime_str("application/octet-stream")
            .map_err(|e| QyzsError::Usage(format!("构建表单文件失败: {}", e)))?;

        let form = Form::new()
            .part("files", part)
            .text("is_ocr", if is_ocr { "true" } else { "false" });

        // 端点路径取配置的解析模型 (model.default_parse, 默认 mineru3.2.3):
        // 该配置项此前无调用方 (parse 子命令已并入 markscan), 现重新生效。
        let model = self.config.get_default_model("markscan");
        let endpoint = format!("ai-gateway/{}/file_parse", model);

        let resp = self
            .http
            .post(&endpoint, Some(key))
            .multipart(form)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(QyzsError::Server(format!("MinerU 网关返回 HTTP {}: {}", status, err_text)));
        }

        // 尝试解析结构化响应
        let raw_text = resp.text().await.map_err(|e| QyzsError::Server(e.to_string()))?;

        // 形态一 (旧包装): {code, msg, data: {markdown | content}}
        if let Ok(res) = serde_json::from_str::<MineruResponse>(&raw_text) {
            if let Some(data) = res.data {
                if let Some(md) = data.markdown {
                    return Ok(md);
                }
                if let Some(c) = data.content {
                    return Ok(c);
                }
            }
        }

        // 形态二 (任务型, 网关实测): {status, results: {"<文件名>": {md_content}}}
        if let Ok(task) = serde_json::from_str::<MineruTaskResponse>(&raw_text) {
            if let Some(results) = task.results {
                // 任务结构命中即走任务路径 (含 md_content 为空的合法情况:
                // 空白页文档 — 不再回落原始 JSON)
                let mut merged = String::new();
                for result in results.values() {
                    if let Some(md) = &result.md_content {
                        if !merged.is_empty() {
                            merged.push_str("\n\n");
                        }
                        merged.push_str(md);
                    }
                }
                return Ok(merged);
            }
            // results 缺失: 任务显式失败或仍在异步执行
            if let Some(status) = &task.status {
                if status != "completed" {
                    if status == "pending" || status == "running" {
                        return Err(QyzsError::Server(format!(
                            "MinerU 返回异步任务状态 (status: {})，当前同步通道不支持轮询；请减小文档体积/页数后重试",
                            status
                        )));
                    }
                    return Err(QyzsError::Server(format!(
                        "MinerU 解析任务未成功 (status: {}): {}",
                        status,
                        task.error
                            .as_ref()
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "无错误详情".to_string())
                    )));
                }
            }
        }

        // 非以上任何结构: 返回原始响应字符串 (保持既有兜底行为)
        Ok(raw_text)
    }
}
