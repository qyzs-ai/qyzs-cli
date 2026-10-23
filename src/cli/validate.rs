//! 命令行参数值校验 (与 `config/resolver` 校验分工:
//! 本模块仅校验 CLI 显式值; 环境变量/配置文件层仍 best-effort 不阻断。
//!
//! 错误码统一为 `QyzsError::Usage`, 退出码 2, 与现有
//! `dispatch.rs` 中 `min_confidence` 校验链一致。

use crate::error::{QyzsError, QyzsResult};

/// 校验 chat --temperature (Option<f64>, 范围 [0.0, 2.0])
/// 拒非有限数 (NaN / ±∞) 与上下界外取值
pub(crate) fn validate_chat_temperature(t: Option<f64>) -> QyzsResult<()> {
    if let Some(v) = t {
        if !v.is_finite() || !(0.0..=2.0).contains(&v) {
            return Err(QyzsError::Usage(format!(
                "--temperature 取值必须在 0.0~2.0 之间，当前: {}",
                v
            )));
        }
    }
    Ok(())
}

/// 校验 chat --top-p (Option<f64>, 范围 [0.0, 1.0])
pub(crate) fn validate_chat_top_p(p: Option<f64>) -> QyzsResult<()> {
    if let Some(v) = p {
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Err(QyzsError::Usage(format!(
                "--top-p 取值必须在 0.0~1.0 之间，当前: {}",
                v
            )));
        }
    }
    Ok(())
}

/// 校验 chat --max-tokens (Option<u32>, 范围 [1, 1_000_000])
/// 上界 1M 取业界主流 LLM 上下文窗口上限 (Magic/Gemini-1.5 量级) 的安全外推,
/// 防止误传 u32::MAX 等退化值 (网关返回难以解读的 HTTP 4xx)
/// 注意: 这是**客户端护栏**上界, 不是网关能力保证——网关上限按模型不同且更低
/// (dsv4 实测接受至 65536; qwen3-VL 文本通道上下文硬上限 131072),
/// 超出模型能力时由网关返回 4xx
pub(crate) fn validate_chat_max_tokens(m: Option<u32>) -> QyzsResult<()> {
    if let Some(v) = m {
        if !(1..=1_000_000).contains(&v) {
            return Err(QyzsError::Usage(format!(
                "--max-tokens 取值必须在 1~1000000 之间，当前: {}",
                v
            )));
        }
    }
    Ok(())
}

/// 校验 pdf-split --chunk-size (Option<usize>, 下界 ≥ 1)
/// 0 会导致 split.rs 生成空 PDF (无实际切片), 1 仍合法 (单页切片)
pub(crate) fn validate_pdf_split_chunk_size(c: Option<usize>) -> QyzsResult<()> {
    if let Some(v) = c {
        if v < 1 {
            return Err(QyzsError::Usage(format!(
                "--chunk-size 取值必须 ≥ 1，当前: {}",
                v
            )));
        }
    }
    Ok(())
}

/// 校验 markscan --max-dim (Option<u32>, 范围 [1, 8192])
/// (原 ocr 子命令已并入 markscan, 此校验器随 dispatch 调用点迁移至 markscan 分支)
/// 8K 已是主流 VLM/OCR 网关的安全上界 (4K/8K 分辨率); 再大不仅耗时,
/// 也容易在网关编码侧触发 4xx
pub(crate) fn validate_ocr_max_dim(d: Option<u32>) -> QyzsResult<()> {
    if let Some(v) = d {
        if !(1..=8192).contains(&v) {
            return Err(QyzsError::Usage(format!(
                "--max-dim 取值必须在 1~8192 之间，当前: {}",
                v
            )));
        }
    }
    Ok(())
}

/// 校验 rerank --top-n (Option<usize>, 范围 [1, 1000])
/// 1000 已覆盖实际召回场景; 更大值对网络/排序无实际意义, 应让网关拒绝
pub(crate) fn validate_rerank_top_n(n: Option<usize>) -> QyzsResult<()> {
    if let Some(v) = n {
        if !(1..=1000).contains(&v) {
            return Err(QyzsError::Usage(format!(
                "--top-n 取值必须在 1~1000 之间，当前: {}",
                v
            )));
        }
    }
    Ok(())
}

/// 校验 usage --days (Option<u32>, 范围 [1, 3650])
/// 3650 = 10 年; 大于此值无意义 (台账自身保留窗口远小于此)
pub(crate) fn validate_usage_days(d: Option<u32>) -> QyzsResult<()> {
    if let Some(v) = d {
        if !(1..=3650).contains(&v) {
            return Err(QyzsError::Usage(format!(
                "--days 取值必须在 1~3650 之间，当前: {}",
                v
            )));
        }
    }
    Ok(())
}

/// 校验 markscan --segment-chars (usize, [1, 100_000])
/// 0 等价"不分段"但仍走分段路径, 行为不一致; 100K 已与原 mod.rs 软警告对齐为硬上限
pub(crate) fn validate_scan_segment_chars(n: usize) -> QyzsResult<()> {
    if !(1..=100_000).contains(&n) {
        return Err(QyzsError::Usage(format!(
            "--segment-chars 取值必须在 1~100000 之间，当前: {}",
            n
        )));
    }
    Ok(())
}

/// 校验 sam --threshold (Option<f64>, 范围 [0.0, 1.0])
pub(crate) fn validate_sam_threshold(t: Option<f64>) -> QyzsResult<()> {
    if let Some(v) = t {
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Err(QyzsError::Usage(format!(
                "--threshold 取值必须在 0.0~1.0 之间，当前: {}",
                v
            )));
        }
    }
    Ok(())
}

/// 校验 usage --kind (Option<&str>, 白名单精确匹配)
/// 取值集与 `client/usage_ledger.rs` 中调用族标签一致 (`chat|ocr|asr|embed|rerank`)
pub(crate) fn validate_usage_kind(k: Option<&str>) -> QyzsResult<()> {
    const VALID: &[&str] = &["chat", "ocr", "asr", "embed", "rerank"];
    if let Some(v) = k {
        if !VALID.contains(&v) {
            return Err(QyzsError::Usage(format!(
                "--kind 取值 '{}' 非法，仅支持: {}",
                v,
                VALID.join(" | ")
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_temperature_validates() {
        assert!(validate_chat_temperature(None).is_ok());
        assert!(validate_chat_temperature(Some(0.0)).is_ok());
        assert!(validate_chat_temperature(Some(2.0)).is_ok());
        assert!(validate_chat_temperature(Some(0.7)).is_ok());
        assert!(validate_chat_temperature(Some(2.5)).is_err());
        assert!(validate_chat_temperature(Some(-0.1)).is_err());
        assert!(validate_chat_temperature(Some(f64::NAN)).is_err());
        assert!(validate_chat_temperature(Some(f64::INFINITY)).is_err());
        assert!(validate_chat_temperature(Some(f64::NEG_INFINITY)).is_err());
    }

    #[test]
    fn chat_top_p_validates() {
        assert!(validate_chat_top_p(None).is_ok());
        assert!(validate_chat_top_p(Some(0.0)).is_ok());
        assert!(validate_chat_top_p(Some(1.0)).is_ok());
        assert!(validate_chat_top_p(Some(0.5)).is_ok());
        assert!(validate_chat_top_p(Some(1.1)).is_err());
        assert!(validate_chat_top_p(Some(-0.1)).is_err());
        assert!(validate_chat_top_p(Some(f64::NAN)).is_err());
        assert!(validate_chat_top_p(Some(f64::INFINITY)).is_err());
    }

    #[test]
    fn chat_max_tokens_validates() {
        assert!(validate_chat_max_tokens(None).is_ok());
        assert!(validate_chat_max_tokens(Some(1)).is_ok());
        assert!(validate_chat_max_tokens(Some(4096)).is_ok());
        assert!(validate_chat_max_tokens(Some(1_000_000)).is_ok());
        // 上界 1M 取业界主流 LLM 上下文窗口的偏安全外推, 拒 u32::MAX
        assert!(validate_chat_max_tokens(Some(u32::MAX)).is_err());
        assert!(validate_chat_max_tokens(Some(1_000_001)).is_err());
        assert!(validate_chat_max_tokens(Some(0)).is_err());
    }

    #[test]
    fn pdf_split_chunk_size_validates() {
        assert!(validate_pdf_split_chunk_size(None).is_ok());
        assert!(validate_pdf_split_chunk_size(Some(1)).is_ok());
        assert!(validate_pdf_split_chunk_size(Some(15)).is_ok());
        assert!(validate_pdf_split_chunk_size(Some(usize::MAX)).is_ok());
        assert!(validate_pdf_split_chunk_size(Some(0)).is_err());
    }

    #[test]
    fn ocr_max_dim_validates() {
        assert!(validate_ocr_max_dim(None).is_ok());
        assert!(validate_ocr_max_dim(Some(1)).is_ok());
        assert!(validate_ocr_max_dim(Some(800)).is_ok());
        assert!(validate_ocr_max_dim(Some(8192)).is_ok());
        assert!(validate_ocr_max_dim(Some(8193)).is_err());
        assert!(validate_ocr_max_dim(Some(0)).is_err());
        assert!(validate_ocr_max_dim(Some(u32::MAX)).is_err());
    }

    #[test]
    fn rerank_top_n_validates() {
        assert!(validate_rerank_top_n(None).is_ok());
        assert!(validate_rerank_top_n(Some(1)).is_ok());
        assert!(validate_rerank_top_n(Some(10)).is_ok());
        assert!(validate_rerank_top_n(Some(1000)).is_ok());
        assert!(validate_rerank_top_n(Some(1001)).is_err());
        assert!(validate_rerank_top_n(Some(0)).is_err());
        assert!(validate_rerank_top_n(Some(usize::MAX)).is_err());
    }

    #[test]
    fn usage_days_validates() {
        assert!(validate_usage_days(None).is_ok());
        assert!(validate_usage_days(Some(1)).is_ok());
        assert!(validate_usage_days(Some(30)).is_ok());
        assert!(validate_usage_days(Some(3650)).is_ok());
        assert!(validate_usage_days(Some(3651)).is_err());
        assert!(validate_usage_days(Some(0)).is_err());
        assert!(validate_usage_days(Some(u32::MAX)).is_err());
    }

    #[test]
    fn scan_segment_chars_validates() {
        assert!(validate_scan_segment_chars(1).is_ok());
        assert!(validate_scan_segment_chars(24_000).is_ok());
        assert!(validate_scan_segment_chars(100_000).is_ok());
        assert!(validate_scan_segment_chars(100_001).is_err());
        assert!(validate_scan_segment_chars(0).is_err());
        assert!(validate_scan_segment_chars(usize::MAX).is_err());
    }

    #[test]
    fn sam_threshold_validates() {
        assert!(validate_sam_threshold(None).is_ok());
        assert!(validate_sam_threshold(Some(0.0)).is_ok());
        assert!(validate_sam_threshold(Some(1.0)).is_ok());
        assert!(validate_sam_threshold(Some(0.5)).is_ok());
        assert!(validate_sam_threshold(Some(1.1)).is_err());
        assert!(validate_sam_threshold(Some(-0.1)).is_err());
        assert!(validate_sam_threshold(Some(f64::NAN)).is_err());
        assert!(validate_sam_threshold(Some(f64::INFINITY)).is_err());
    }

    #[test]
    fn usage_kind_validates() {
        assert!(validate_usage_kind(None).is_ok());
        for k in ["chat", "ocr", "asr", "embed", "rerank"] {
            assert!(
                validate_usage_kind(Some(k)).is_ok(),
                "expected {} to be valid",
                k
            );
        }
        assert!(validate_usage_kind(Some("translation")).is_err());
        assert!(validate_usage_kind(Some("chatty")).is_err());
        assert!(validate_usage_kind(Some("")).is_err());
        // 大小写敏感: 与 `usage_ledger::filter_records` 精确匹配一致
        assert!(validate_usage_kind(Some("CHAT")).is_err());
    }

    #[test]
    fn error_messages_mention_flag_and_valid_range() {
        // 错误文案必须提可操作信息: 参数名 + 合法范围
        let err = validate_chat_temperature(Some(3.0)).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("--temperature"), "应点名参数: {msg}");
        assert!(msg.contains("0.0~2.0"), "应指明合法范围: {msg}");

        let err = validate_usage_kind(Some("foo")).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("--kind"), "应点名参数: {msg}");
        assert!(
            msg.contains("chat") && msg.contains("rerank"),
            "应列出合法值: {msg}"
        );
    }
}
