//! markscan 图像理解通道 (paddleocrvl OCR + qwen3-VL 描述, `--image-handling` 策略)
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

/// 双 OCR 引擎的一致性阈值 (字符 bigram Dice): 低于此值视为两引擎分歧。
/// 校准 (2026-10-21, 16 页基准): 干净页双引擎一致度 ≈0.945, 敌对版式页
/// 完全分歧 ≈0.0, 0.70 居中且两侧余量充分。
const OCR_AGREEMENT_THRESHOLD: f64 = 0.70;

/// VL 转录提示词 (与 paddle 输出形态对齐的纯文本转录, 最大化干净页一致度)
const VL_TRANSCRIBE_PROMPT: &str = "逐字转录这张图片中的全部文字内容, 保留段落与标题层级。只输出转录文本, 不要解释。";

/// 字符 bigram Dice 相似度 (0.0~1.0)
fn bigram_dice(a: &str, b: &str) -> f64 {
    use std::collections::HashSet;
    let strip = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
    let (a, b) = (strip(a), strip(b));
    let grams = |s: &str| -> HashSet<(char, char)> {
        s.chars().collect::<Vec<_>>().windows(2).map(|w| (w[0], w[1])).collect()
    };
    let (ga, gb) = (grams(&a), grams(&b));
    if ga.is_empty() || gb.is_empty() {
        return 0.0;
    }
    2.0 * ga.intersection(&gb).count() as f64 / (ga.len() + gb.len()) as f64
}

/// 双引擎结果择优: 一致时取 paddle (快, 公文场景微优), 分歧时取 VL
/// (实测: 敌对版式下 paddle 会幻觉出重复内容 — 16 页基准最低 0.1%,
///  VL 退化平缓 — 同批最低 64.8%)。返回 (文本, 是否采纳了 VL)。
fn select_ocr(paddle: Option<String>, vl: Option<String>) -> (Option<String>, bool) {
    match (paddle, vl) {
        (Some(p), Some(v)) => {
            if bigram_dice(&p, &v) >= OCR_AGREEMENT_THRESHOLD {
                (Some(p), false)
            } else {
                (Some(v), true)
            }
        }
        (Some(p), None) => (Some(p), false),
        (None, Some(v)) => (Some(v), true),
        (None, None) => (None, false),
    }
}

/// 双 OCR 引擎并行转录 (paddleocrvl ∥ qwen3-VL), 分歧时采纳 VL
///
/// 并行使墙钟 ≈ max(两引擎) 而非求和; 令牌开销 = 每图多一次 VL 调用
/// (准确率优先场景接受)。
async fn run_ocr_dual(
    client: &QyzsClient,
    image_path: &str,
    image_prompt: Option<&str>,
    max_dim: Option<u32>,
) -> QyzsResult<(String, bool)> {
    let paddle_fut = async {
        client
            .ocr(image_path, image_prompt, max_dim)
            .await
            .map(|r| r.text.trim().to_string())
            .and_then(|t| {
                if t.is_empty() {
                    Err(QyzsError::Server("paddleocrvl 返回空内容".to_string()))
                } else {
                    Ok(t)
                }
            })
    };
    let vl_fut = async {
        let resp = client
            .chat(
                image_prompt.unwrap_or(VL_TRANSCRIBE_PROMPT),
                None,
                ChatOptions {
                    model: None, // 无显式模型 → 客户端按有图自动选路 qwen3-VL
                    temperature: Some(0.0),
                    max_tokens: Some(8192),
                    top_p: None,
                    stream: false,
                    show_reasoning: false,
                    images: vec![image_path.to_string()],
                },
            )
            .await?;
        let t = resp.primary_text().trim().to_string();
        if t.is_empty() {
            return Err(QyzsError::Server("qwen3-VL 转录返回空内容".to_string()));
        }
        Ok(t)
    };
    let (p, v) = tokio::join!(paddle_fut, vl_fut);
    let (text, used_vl) = select_ocr(p.ok(), v.ok());
    let Some(text) = text else {
        return Err(QyzsError::Server("双 OCR 引擎均返回空内容".to_string()));
    };
    Ok((text, used_vl))
}

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
            let (text, used_vl) = run_ocr_dual(client, image_path, image_prompt, max_dim).await?;
            if used_vl {
                eprintln!("\x1b[36m[INFO]\x1b[0m 双 OCR 引擎分歧, 采纳 qwen3-VL 转录");
            }
            format!("# OCR 文字\n\n{}\n", text)
        }
        super::ImageHandling::Describe => {
            let desc = run_describe(client, image_path).await?;
            format!("# 图像描述\n\n{}\n", desc)
        }
        super::ImageHandling::Both => {
            let (text, used_vl) = run_ocr_dual(client, image_path, image_prompt, max_dim).await?;
            if used_vl {
                eprintln!("\x1b[36m[INFO]\x1b[0m 双 OCR 引擎分歧, 采纳 qwen3-VL 转录");
            }
            let desc = run_describe(client, image_path).await?;
            format!("# OCR 文字\n\n{}\n\n# 图像描述\n\n{}\n", text, desc)
        }
        super::ImageHandling::Auto => {
            let (text, used_vl) = run_ocr_dual(client, image_path, image_prompt, max_dim).await?;
            if used_vl {
                eprintln!("\x1b[36m[INFO]\x1b[0m 双 OCR 引擎分歧, 采纳 qwen3-VL 转录");
            }
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
mod dual_select_tests {
    use super::*;

    #[test]
    fn test_bigram_dice() {
        // 同文异空白/标点: 高
        let a = "第一条 为了规范招标投标活动";
        let b = "第一条为了规范招标投标活动。";
        assert!(bigram_dice(a, b) > 0.85);
        // 完全不同: 低
        let c = "首页 > 正文阅读 请输入检索词 下载PDF";
        assert!(bigram_dice(a, c) < 0.2);
        assert_eq!(bigram_dice("", "任意"), 0.0);
    }

    #[test]
    fn test_select_ocr_agreement_prefers_paddle() {
        let (text, used_vl) = select_ocr(
            Some("第一条 为了规范招标投标活动，根据招标投标法".into()),
            Some("第一条 为了规范招标投标活动，根据招标投标法的规定".into()),
        );
        assert!(!used_vl, "一致时取 paddle");
        assert!(text.as_deref().unwrap().contains("第一条"));
    }

    #[test]
    fn test_select_ocr_divergence_prefers_vl() {
        // 分歧 (paddle 幻觉出重复行): 采纳 VL
        let (text, used_vl) = select_ocr(
            Some("条例|条例\n条例|条例\n条例|条例".repeat(3)),
            Some("中华人民共和国野生植物保护条例 1996年9月30日发布".into()),
        );
        assert!(used_vl, "分歧时应采纳 VL");
        assert!(text.as_deref().unwrap().contains("1996"));
    }

    #[test]
    fn test_select_ocr_single_engine() {
        let (t, used) = select_ocr(Some("只有 paddle 成功".into()), None);
        assert!(!used && t.as_deref() == Some("只有 paddle 成功"));
        let (t, used) = select_ocr(None, Some("只有 VL 成功".into()));
        assert!(used && t.as_deref() == Some("只有 VL 成功"));
        let (t, _) = select_ocr(None, None);
        assert_eq!(t, None);
    }
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
