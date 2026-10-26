//! 角色化模型兜底链 (与 engine_health 熔断器联动)
//!
//! 原则: 同一角色按优先级排候选链。调用前跳过熔断中的模型 (下线模型一次
//! 进程内最多被真实探测 3 次即短路, 不反复浪费时延与令牌); 真实失败计入
//! 熔断并落下一位候选; 全链失败才向上抛错, 由调用方按既有语义降级。批处理阶段
//! 先以真实首图或最小文本预检首选模型，失败即熔断并旋到下一候选。
//!
//! 链条依据 (2026-10-26 全量探活): paddleocrvl 网关故障期, qwen36 多模态
//! 实测可用 (图像理解 6.2s), qwen3-VL 稳定 —— OCR 角色可在专用 OCR 不可
//! 用时切换多模态 chat 模型, 而非丢弃该证据通道。

use super::engine_health;
use crate::client::{ChatOptions, QyzsClient};
use crate::error::{QyzsError, QyzsResult};

/// 文字转录/OCR 角色: 专用 OCR 优先, 多模态 chat 模型兜底
pub const ROLE_OCR: &[&str] = &["paddleocrvl", "qwen36", "qwen3-VL-30B-A3B"];
/// 版面视觉理解角色 (有图自动选路的默认模型优先)
pub const ROLE_VL: &[&str] = &["qwen3-VL-30B-A3B", "qwen36"];
/// 对抗仲裁对手链 (与 primary 不同族优先, 依次兜底)
pub const ROLE_ARBITRATION_OPPONENT: &[&str] = &["qwen3.6-27b", "qwen36", "dsv4"];
/// 通用 chat 兜底链 (润色/全局复核/排版验证等)
pub const ROLE_CHAT: &[&str] = &["dsv4", "qwen3.6-27b", "qwen36"];

const PREFLIGHT_PROMPT: &str = "健康检查：只回复 OK。";

fn candidate_available(model: &str) -> bool {
    !crate::model_capabilities::ocr_model(model).is_some_and(|cap| !cap.online_at_test)
        && engine_health::engine_available(model)
}

/// 在正式文本批处理前快速检查候选链，返回首个可调用模型。
pub async fn preflight_chat_chain(client: &QyzsClient, chain: &[&str]) -> QyzsResult<String> {
    let mut last_err = QyzsError::Server("文本模型预检链为空".to_string());
    for model in chain {
        if !candidate_available(model) {
            continue;
        }
        if !engine_health::claim_preflight("chat", model) {
            return Ok((*model).to_string());
        }
        let result = client
            .chat(
                PREFLIGHT_PROMPT,
                None,
                ChatOptions {
                    model: Some((*model).to_string()),
                    temperature: Some(0.0),
                    max_tokens: Some(8),
                    top_p: None,
                    stream: false,
                    show_reasoning: false,
                    images: Vec::new(),
                    image_max_dim: None,
                },
            )
            .await;
        match result {
            Ok(_) => {
                engine_health::record_success(model);
                return Ok((*model).to_string());
            }
            Err(error) => {
                engine_health::trip_now(model);
                last_err = error;
            }
        }
    }
    Err(last_err)
}

/// 用本次任务的真实首图检查视觉候选链，返回首个可调用模型。
pub async fn preflight_vision_chain(
    client: &QyzsClient,
    chain: &[&str],
    image_path: &str,
    max_dim: Option<u32>,
) -> QyzsResult<String> {
    let mut last_err = QyzsError::Server("视觉模型预检链为空".to_string());
    let probe_dim = Some(max_dim.unwrap_or(512).min(512));
    for model in chain {
        if !candidate_available(model) {
            continue;
        }
        if !engine_health::claim_preflight("vision", model) {
            return Ok((*model).to_string());
        }
        let result = if *model == "paddleocrvl" {
            client
                .ocr_with_model(
                    image_path,
                    Some("确认图片可读取，只输出图片中的首行文字。"),
                    probe_dim,
                    model,
                )
                .await
                .map(|response| response.text)
        } else {
            transcribe_via_chat(
                client,
                model,
                image_path,
                "确认图片可读取，只输出图片中的首行文字。",
                128,
                probe_dim,
            )
            .await
        };
        match result {
            Ok(_) => {
                engine_health::record_success(model);
                return Ok((*model).to_string());
            }
            Err(error) => {
                engine_health::trip_now(model);
                last_err = error;
            }
        }
    }
    Err(last_err)
}

/// 在语义验证批处理前用一个最小文本检查嵌入候选链。
pub async fn preflight_embed_chain(client: &QyzsClient, chain: &[&str]) -> QyzsResult<String> {
    let mut last_err = QyzsError::Server("嵌入模型预检链为空".to_string());
    for model in chain {
        if !engine_health::engine_available(model) {
            continue;
        }
        if !engine_health::claim_preflight("embed", model) {
            return Ok((*model).to_string());
        }
        match client
            .embed(vec!["健康检查".to_string()], Some(model))
            .await
        {
            Ok(_) => {
                engine_health::record_success(model);
                return Ok((*model).to_string());
            }
            Err(error) => {
                engine_health::trip_now(model);
                last_err = error;
            }
        }
    }
    Err(last_err)
}

/// 沿候选链调用; 返回 (实际使用的模型, 结果)。
pub async fn with_fallback<T, F, Fut>(chain: &[&str], mut f: F) -> QyzsResult<(String, T)>
where
    F: FnMut(&str) -> Fut,
    Fut: std::future::Future<Output = QyzsResult<T>>,
{
    let mut last_err = QyzsError::Server("兜底链为空".to_string());
    for model in chain {
        if !candidate_available(model) {
            continue; // 熔断中: 不发注定失败的请求
        }
        match f(model).await {
            Ok(v) => {
                engine_health::record_success(model);
                return Ok((model.to_string(), v));
            }
            Err(e) => {
                engine_health::record_failure(model);
                last_err = e;
            }
        }
    }
    Err(last_err)
}

/// 从链中挑选可用的对手模型 (排除 primary 同名; 全不可用时取链首由调用方降级)
pub fn pick_opponent<'a>(primary: &str, chain: &'a [&'a str]) -> &'a str {
    chain
        .iter()
        .find(|m| **m != primary && engine_health::engine_available(m))
        .copied()
        .unwrap_or(chain.first().copied().unwrap_or("dsv4"))
}

/// 经 chat 通道的多模态文字转录 (qwen36 / qwen3-VL 等非 OCR 专线模型用)
pub async fn transcribe_via_chat(
    client: &QyzsClient,
    model: &str,
    image_path: &str,
    prompt: &str,
    max_tokens: u32,
    max_dim: Option<u32>,
) -> QyzsResult<String> {
    let resp = client
        .chat(
            prompt,
            None,
            ChatOptions {
                model: Some(model.to_string()),
                temperature: Some(0.0),
                max_tokens: Some(max_tokens),
                top_p: None,
                stream: false,
                show_reasoning: false,
                images: vec![image_path.to_string()],
                image_max_dim: max_dim,
            },
        )
        .await?;
    let t = resp.primary_text().trim().to_string();
    if t.is_empty() {
        Err(QyzsError::Server(format!("{model} 转录返回空内容")))
    } else {
        Ok(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fallback_to_second_candidate() {
        let (m, v) = with_fallback(&["fb-test-a", "fb-test-b"], |m| {
            let m = m.to_string();
            async move {
                if m == "fb-test-a" {
                    Err::<String, _>(QyzsError::Server("down".to_string()))
                } else {
                    Ok(m.clone())
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(m, "fb-test-b");
        assert_eq!(v, "fb-test-b");
        engine_health::record_success("fb-test-b");
    }

    #[tokio::test]
    async fn test_fallback_skips_matrix_offline_candidate() {
        let (model, _) = with_fallback(&["paddleocrvl", "fallback-online"], |model| {
            let model = model.to_string();
            async move { Ok::<_, QyzsError>(model.clone()) }
        })
        .await
        .unwrap();
        assert_eq!(model, "fallback-online");
    }

    #[tokio::test]
    async fn test_fallback_all_fail() {
        let r = with_fallback(&["fb-test-c"], |_| async {
            Err::<String, _>(QyzsError::Server("down".to_string()))
        })
        .await;
        assert!(r.is_err());
    }

    #[test]
    fn test_pick_opponent_avoids_primary() {
        let opp = pick_opponent("dsv4", ROLE_ARBITRATION_OPPONENT);
        assert_ne!(opp, "dsv4");
        let opp2 = pick_opponent("qwen3.6-27b", ROLE_ARBITRATION_OPPONENT);
        assert_ne!(opp2, "qwen3.6-27b");
    }
}
