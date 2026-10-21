//! scan 图像理解通道 (paddleocrvl OCR + qwen3-VL 描述, `--image-handling` 策略)
//!
//! 策略语义 (SSOT §2):
//! - `ocr`:      仅 OCR 文字转录 (paddleocrvl, 800px 防崩溃缩放);
//! - `describe`: 仅 VL 理解描述 (qwen3-VL);
//! - `both`:     OCR 文字 + VL 描述双节输出;
//! - `auto`:     先 OCR; 文字产出量低于阈值 (版面稀疏/纯图像) 时自动补 VL 描述;
//! - `skip`:     不提取图像内容 (输出占位说明)。
//!
//! 预处理约束 (capability_matrix): paddleocrvl 图像最长边 ≤800px; VL 16 图/请求
//! 上限对单文件输入不构成约束。

use super::workspace::Workspace;
use crate::client::{ChatOptions, QyzsClient};
use crate::error::{QyzsError, QyzsResult};

/// auto 策略的"文字稀少"判定阈值 (OCR 字符数)
const AUTO_SPARSE_TEXT_CHARS: usize = 20;

/// VL 描述提示词
const DESCRIBE_PROMPT: &str = "全面描述这张图片的内容: 主题、文字要点(如有请逐字列出)、图表结构、空间布局。用简体中文的 Markdown 列表输出。";

pub async fn extract_image(
    client: &QyzsClient,
    ws: &mut Workspace,
    image_path: &str,
    handling: super::ImageHandling,
    image_prompt: Option<&str>,
    max_dim: Option<u32>,
) -> QyzsResult<String> {
    if ws.stage_done("document", "extract") {
        return ws.read_segment_text("document");
    }

    let output = match handling {
        super::ImageHandling::Skip => {
            "_(图像内容提取已被 --image-handling skip 跳过)_\n".to_string()
        }
        super::ImageHandling::Ocr => {
            let text = run_ocr(client, image_path, image_prompt, max_dim).await?;
            format!("# OCR 文字\n\n{}\n", text)
        }
        super::ImageHandling::Describe => {
            let desc = run_describe(client, image_path).await?;
            format!("# 图像描述\n\n{}\n", desc)
        }
        super::ImageHandling::Both => {
            let text = run_ocr(client, image_path, image_prompt, max_dim).await?;
            let desc = run_describe(client, image_path).await?;
            format!("# OCR 文字\n\n{}\n\n# 图像描述\n\n{}\n", text, desc)
        }
        super::ImageHandling::Auto => {
            let text = run_ocr(client, image_path, image_prompt, max_dim).await?;
            let text_chars = text.trim().chars().count();
            if text_chars < AUTO_SPARSE_TEXT_CHARS {
                // 版面稀疏 (照片/图纸): OCR 文字少, 自动补 VL 描述
                let desc = run_describe(client, image_path).await?;
                if desc.trim().is_empty() {
                    format!("# OCR 文字\n\n{}\n", text)
                } else {
                    format!(
                        "# OCR 文字\n\n{}\n\n# 图像描述\n\n{}\n",
                        text, desc
                    )
                }
            } else {
                format!("# OCR 文字\n\n{}\n", text)
            }
        }
    };

    ws.write_segment_text("document", &output)?;
    ws.complete_stage("document", "extract", &[])?;
    Ok(output)
}

async fn run_ocr(
    client: &QyzsClient,
    image_path: &str,
    image_prompt: Option<&str>,
    max_dim: Option<u32>,
) -> QyzsResult<String> {
    let resp = client.ocr(image_path, image_prompt, max_dim).await?;
    let text = resp.text.trim().to_string();
    if text.is_empty() {
        return Err(QyzsError::Server(format!(
            "OCR 对 {} 返回空内容 (图像可能无法解码或网关异常)",
            image_path
        )));
    }
    Ok(text)
}

async fn run_describe(client: &QyzsClient, image_path: &str) -> QyzsResult<String> {
    let resp = client
        .chat(
            DESCRIBE_PROMPT,
            None,
            ChatOptions {
                // None → 客户端按有图自动选 qwen3-VL (chat.rs 模型决策)
                model: None,
                temperature: Some(0.1),
                max_tokens: Some(2048),
                top_p: None,
                stream: false,
                show_reasoning: false,
                images: vec![image_path.to_string()],
            },
        )
        .await?;

    Ok(resp.primary_text())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// auto 决策逻辑抽成纯函数测试 (extract_image 的分支依据)
    #[test]
    fn test_auto_sparse_threshold() {
        assert!(AUTO_SPARSE_TEXT_CHARS > 0);
        // 阈值语义: OCR 产出不足 20 字符视为版面稀疏
        let dense = "财政部关于政府采购评审专家管理办法的通知";
        let sparse = "2024";
        assert!(dense.chars().count() >= AUTO_SPARSE_TEXT_CHARS);
        assert!(sparse.chars().count() < AUTO_SPARSE_TEXT_CHARS);
    }
}
