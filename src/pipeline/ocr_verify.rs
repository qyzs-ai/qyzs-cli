//! OCR 与 MinerU 结构化解析同源互验 (对齐 QYZS-ANALYZE.md §4)
//!
//! 仅做相似度度量与通过/不通过判定; 不涉及红头、印章或正文的任何改写,
//! 也不调用二次模型复核 —— 提示文案不得暗示这些行为。
//!
//! 相似度口径 (2026-10-20 实测校正):
//! 旧实现用「唯一字符集合」的 Jaccard —— 只看两段文本用到了哪些字, 不看字的
//! 位置与顺序。对中文公文几乎无鉴别力: 任意两份政策文件的常用字集高度重合,
//! 实测「同源文档」49%~87%,「完全无关的两份文档」也达 49%~58%, 全部越过
//! 15% 阈值判为「通过」—— 该闸门根本无法识别错配。
//!
//! 现改为「字符 n-gram 集合」的 Jaccard: n-gram 同时编码字符内容与相邻顺序,
//! 无关文档之间不再共享大量 n-gram, 同源文档 (同一版面不同抽取通道) 则仍然
//! 高度重合。

use std::collections::HashSet;

/// 字符 n-gram 长度
const NGRAM_N: usize = 2;

/// 互验通过阈值 (字符 n-gram Jaccard)
pub const PASS_THRESHOLD: f64 = 0.15;

#[derive(Debug, Clone)]
pub struct VerificationReport {
    /// 字符 n-gram Jaccard 相似度 (0.0~1.0)
    pub jaccard_similarity: f64,
    pub ocr_char_count: usize,
    pub mineru_char_count: usize,
    pub passed: bool,
    pub notes: String,
}

pub struct OcrVerifier;

impl OcrVerifier {
    /// 执行 OCR 识别结果与 MinerU 解析结果的同源互验
    pub fn verify(ocr_text: &str, mineru_markdown: &str) -> VerificationReport {
        let ocr_clean: String = ocr_text.chars().filter(|c| !c.is_whitespace()).collect();
        let md_clean: String = mineru_markdown
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();

        let jaccard = ngram_jaccard(&ocr_clean, &md_clean);

        // 依据实测指标：n-gram 相似度大于阈值判定为互验通过
        // (单页 OCR vs 全篇解析存在长度差异, 阈值取较低以适配)
        let passed = jaccard >= PASS_THRESHOLD;
        // 注意: 本校验器只计算相似度并给结论, 不做任何后处理改写、
        // 印章/红头剥离, 也不调用二次模型复核 —— 提示文案不得暗示这些行为
        let notes = if passed {
            "n-gram 相似度正常, 两通道内容同源".to_string()
        } else {
            format!(
                "n-gram 相似度低于 {:.0}% 阈值, 两通道可能错配或非同源, 建议人工复核",
                PASS_THRESHOLD * 100.0
            )
        };

        VerificationReport {
            jaccard_similarity: jaccard,
            ocr_char_count: ocr_clean.chars().count(),
            mineru_char_count: md_clean.chars().count(),
            passed,
            notes,
        }
    }
}

/// 字符 n-gram 集合的 Jaccard 相似度
///
/// n-gram 同时约束字符内容与相邻顺序, 因此「用字相同但语序不同」的文本
/// 不会被判为高相似度 —— 这正是唯一字符集口径失效的地方。
pub fn ngram_jaccard(a: &str, b: &str) -> f64 {
    let sa = ngram_set(a);
    let sb = ngram_set(b);
    if sa.is_empty() && sb.is_empty() {
        return 1.0;
    }
    let inter = sa.intersection(&sb).count();
    let union = sa.union(&sb).count();
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

/// 提取字符 n-gram 集合
fn ngram_set(s: &str) -> HashSet<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut set = HashSet::new();
    if chars.len() < NGRAM_N {
        // 短于 n 的文本退化为「整串」, 避免空集恒等于 1.0
        if !chars.is_empty() {
            set.insert(chars.iter().collect());
        }
        return set;
    }
    for window in chars.windows(NGRAM_N) {
        set.insert(window.iter().collect());
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_same_document_passes() {
        let ocr = "自然资源部关于印发自然资源领域数据安全管理办法的通知各省自治区直辖市";
        let md = "# 自然资源部关于印发《自然资源领域数据安全管理办法》的通知\n各省自治区直辖市自然资源主管部门";
        let report = OcrVerifier::verify(ocr, md);
        assert!(report.passed, "同源文档应通过: {:?}", report);
    }

    #[test]
    fn test_unrelated_documents_rejected() {
        // 两份完全不同领域的中文政策文件 (用字相近但语序与内容无关)
        let ocr = "生态环境主管部门应当加强生物多样性保护监督管理依法查处违法案件";
        let md = "# 云南省政府采购评审专家管理办法\n省财政厅负责评审专家的抽取和管理工作";
        let report = OcrVerifier::verify(ocr, md);
        assert!(
            !report.passed,
            "无关文档不应通过互验 (实际 {:.2}%)",
            report.jaccard_similarity * 100.0
        );
    }

    #[test]
    fn test_order_matters() {
        // 同样的字, 不同语序 → 唯一字符集口径无法区分, n-gram 可以
        let a = "abcdef";
        let b = "fedcba";
        assert_ne!(ngram_jaccard(a, b), 1.0);
        assert!(ngram_jaccard(a, a) == 1.0);
    }

    #[test]
    fn test_empty_inputs() {
        let r = OcrVerifier::verify("", "");
        assert_eq!(r.jaccard_similarity, 1.0);
        assert!(r.passed);
        let r2 = OcrVerifier::verify("内容", "");
        assert!(!r2.passed);
    }

    #[test]
    fn test_char_counts_ignore_whitespace() {
        let report = OcrVerifier::verify("中 文\n\t国", "中文");
        assert_eq!(report.ocr_char_count, 3);
        assert_eq!(report.mineru_char_count, 2);
    }
}
