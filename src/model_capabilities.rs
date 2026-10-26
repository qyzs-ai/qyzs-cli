//! 编译期嵌入的模型能力事实源。
//!
//! 这里只消费 `capability_matrix.toml` 中机器可读且有测试证据的字段。未知能力保持
//! `None`，调度器不得根据模型名称猜测限制。

use serde::Deserialize;
use std::sync::OnceLock;

const MATRIX_TOML: &str = include_str!("../capability_matrix.toml");

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum CapabilityLevel {
    Flag(bool),
    Mode(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct OcrModelCapability {
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub transport: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub input_mime: Vec<String>,
    pub online_at_test: bool,
    pub tested_at: String,
    pub max_tested_accepted_long_edge: Option<u32>,
    pub min_tested_rejected_long_edge: Option<u32>,
    pub recommended_long_edge: Option<u32>,
    pub max_images: Option<usize>,
    pub max_output_tokens: Option<u32>,
    pub supports_bbox: CapabilityLevel,
    pub supports_native_confidence: CapabilityLevel,
    pub supports_reading_order: CapabilityLevel,
    pub supports_table_structure: CapabilityLevel,
    pub evidence: String,
}

#[derive(Debug, Deserialize)]
struct OcrCapabilities {
    #[allow(dead_code)]
    schema_version: u32,
    #[serde(default)]
    models: Vec<OcrModelCapability>,
}

#[derive(Debug, Deserialize)]
struct Matrix {
    ocr_capabilities: OcrCapabilities,
}

fn capabilities() -> &'static [OcrModelCapability] {
    static CAPABILITIES: OnceLock<Vec<OcrModelCapability>> = OnceLock::new();
    CAPABILITIES.get_or_init(|| {
        toml::from_str::<Matrix>(MATRIX_TOML)
            .map(|matrix| matrix.ocr_capabilities.models)
            .unwrap_or_default()
    })
}

pub fn ocr_model(model: &str) -> Option<&'static OcrModelCapability> {
    capabilities()
        .iter()
        .find(|cap| cap.id == model || cap.aliases.iter().any(|alias| alias == model))
}

/// 根据用户/配置请求和模型实测安全边界计算实际长边。
///
/// - 有跨样本实测安全上沿时不越界；
/// - 未显式请求时采用模型实测推荐值；
/// - 模型尺寸能力未知时返回用户/配置值，完全未知则返回 None（保留原图）。
pub fn effective_long_edge(model: &str, requested: Option<u32>) -> Option<u32> {
    let cap = ocr_model(model);
    let desired = requested.or_else(|| cap.and_then(|c| c.recommended_long_edge));
    // “测到某尺寸成功”不等于上限。只有同一轮还测到了更高尺寸失败，才把
    // max_tested_accepted_long_edge 当作可执行的安全钳制点。
    let enforced_safe_edge = cap
        .filter(|c| c.min_tested_rejected_long_edge.is_some())
        .and_then(|c| c.max_tested_accepted_long_edge);
    match (desired, enforced_safe_edge) {
        (Some(value), Some(safe)) => Some(value.min(safe)),
        (Some(value), None) => Some(value),
        (None, _) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_parses_known_ocr_models() {
        let paddle = ocr_model("paddleocrvl").expect("paddleocrvl capability");
        assert_eq!(paddle.max_tested_accepted_long_edge, Some(1408));
        assert_eq!(paddle.min_tested_rejected_long_edge, Some(1536));
        assert!(paddle.roles.contains(&"text-ocr".to_string()));
        assert_eq!(
            ocr_model("paddleocr-vl").map(|m| m.id.as_str()),
            Some("paddleocrvl")
        );
        assert!(ocr_model("qwen3-VL-30B-A3B").is_some());
    }

    #[test]
    fn tested_safe_limit_clamps_requested_size() {
        assert_eq!(effective_long_edge("paddleocrvl", None), Some(1280));
        assert_eq!(effective_long_edge("paddleocrvl", Some(2048)), Some(1408));
        assert_eq!(effective_long_edge("paddleocrvl", Some(640)), Some(640));
    }

    #[test]
    fn unknown_limit_does_not_invent_resize() {
        assert_eq!(effective_long_edge("qwen3-VL-30B-A3B", None), None);
        assert_eq!(
            effective_long_edge("qwen3-VL-30B-A3B", Some(1600)),
            Some(1600)
        );
        assert_eq!(effective_long_edge("unknown", None), None);
    }
}
