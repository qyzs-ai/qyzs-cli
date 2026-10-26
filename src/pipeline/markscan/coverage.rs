//! 覆盖检查: 文本层 (第二通道证据) 有而产物缺失的内容
//!
//! 背景 (2026-10-26 N078 实测): MinerU 抽取漏掉了第 2 页落款日期
//! (`2025 年 12 月 15 日`)。文本层里有这行字, 但主通道产物没有 —— 上游抽取
//! 缺口, 交叉验证因该行过短 (12 字符 < 20 字符参与下限) 也不会发现。
//! 本模块做端到端覆盖核对, 把缺口如实报进 `--report` 的 warnings 与 stderr。
//!
//! 判定口径:
//! - 数字串: 文本层数字多重集 ⊆ 产物数字多重集? 逐个列出缺失项 (数字是最易
//!   丢失且最不可错的内容);
//! - 短行: 文本层中长度 ≥ MIN_LINE_CHARS 且未在产物出现的行 → 列为疑似缺失。
//!   仅报告不自动补写 (自动补写属于内容改写, 超出"结构归一"契约)。

use std::collections::{BTreeMap, BTreeSet};

/// 参与短行比对的最小字符数
const MIN_LINE_CHARS: usize = 8;

/// 数字**字符**计数 (0-9), 而非数字串
///
/// 用字符级而非数字串级: 部分 PDF 的文字编码损坏, lopdf 抽出的文本层是
/// 逐字换行的碎片 (2026-10-26 N078 实测第 1 页为 `云\n南\n省\n发\n...`),
/// 数字串被拆成单个字符 (12 → "1","2")。按串比较会产生大量假缺失;
/// 按字符计数则与碎片无关, 仍能如实反映数字总量差异。
fn digit_counts(s: &str) -> BTreeMap<char, usize> {
    let mut out: BTreeMap<char, usize> = BTreeMap::new();
    for c in s.chars().filter(|c| c.is_ascii_digit()) {
        *out.entry(c).or_insert(0) += 1;
    }
    out
}

/// 文本层是否碎片化 (多数行为短行) —— 决定行级比对是否可用
fn is_fragmented(layer: &str) -> bool {
    let lines: Vec<&str> = layer.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return false;
    }
    let short = lines
        .iter()
        .filter(|l| l.trim().chars().count() <= 2)
        .count();
    short * 2 > lines.len()
}

/// 覆盖检查结果
#[derive(Debug, Default, Clone)]
pub struct CoverageReport {
    /// 文本层有而产物缺的数字串 (含出现次数差)
    pub missing_numbers: BTreeMap<char, usize>,
    /// 文本层有而产物未出现的行 (疑似丢失内容)
    pub missing_lines: Vec<String>,
    /// 文本层碎片化 (逐字换行) → 行级比对不可用, 只报数字差
    pub fragmented: bool,
}

impl CoverageReport {
    /// 是否存在覆盖缺口
    pub fn has_gap(&self) -> bool {
        !self.missing_numbers.is_empty() || !self.missing_lines.is_empty()
    }

    /// 渲染为可读告警 (无缺口时返回 None)
    pub fn render(&self) -> Option<String> {
        if !self.has_gap() {
            return None;
        }
        let mut parts = Vec::new();
        if !self.missing_numbers.is_empty() {
            let nums: Vec<String> = self
                .missing_numbers
                .iter()
                .map(|(k, v)| format!("{k}×{v}"))
                .collect();
            parts.push(format!("文本层数字字符缺失: {}", nums.join(", ")));
        }
        if !self.missing_lines.is_empty() {
            let lines: Vec<String> = self
                .missing_lines
                .iter()
                .take(5)
                .map(|l| format!("\"{}\"", l.chars().take(40).collect::<String>()))
                .collect();
            parts.push(format!("文本层疑似缺失行: {}", lines.join(" | ")));
        }
        Some(
            format!(
                "覆盖检查: 产物相对文本层有缺失 ({} 项数字, {} 行文本) —— 上游抽取缺口, 未自动补写",
                self.missing_numbers.len(),
                self.missing_lines.len()
            ) + &format!(" [{}]", parts.join("; ")),
        )
    }
}

/// 核对: 文本层 (layer) 相对产物 (output) 的覆盖缺口
pub fn check(layer: &str, output: &str) -> CoverageReport {
    let mut rep = CoverageReport::default();

    // 数字字符差集 (碎片安全)
    let ld = digit_counts(layer);
    let od = digit_counts(output);
    for (k, v) in ld {
        let have = od.get(&k).copied().unwrap_or(0);
        if have < v {
            rep.missing_numbers.insert(k, v - have);
        }
    }

    // 短行差集: 文本层的行是否在产物中作为子串出现
    // 碎片层 (逐字换行) 不做行级比对 —— 碎片行不是语义行, 比对只会全假阳性
    if is_fragmented(layer) {
        rep.fragmented = true;
        return rep;
    }
    let out_norm: String = output.chars().filter(|c| !c.is_whitespace()).collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for line in layer.lines() {
        let t = line.trim();
        let n = t.chars().count();
        if n < MIN_LINE_CHARS {
            continue;
        }
        // 短行多为落款/日期/文号等单行内容; 数字行已由数字差集覆盖, 不重复列
        if t.chars().all(|c| {
            c.is_ascii_digit()
                || c.is_whitespace()
                || c == '年'
                || c == '月'
                || c == '日'
                || c == ':'
                || c == '：'
        }) {
            continue;
        }
        let norm: String = t.chars().filter(|c| !c.is_whitespace()).collect();
        if out_norm.contains(&norm) {
            continue;
        }
        if seen.insert(norm) {
            rep.missing_lines.push(t.to_string());
        }
    }
    rep
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全部覆盖 → 无缺口
    #[test]
    fn test_coverage_full() {
        let layer = "第一条 内容一 2025 年 12 月 15 日\n第二条 内容二";
        let out = "# 标题\n\n第一条 内容一 2025 年 12 月 15 日\n\n第二条 内容二\n";
        let rep = check(layer, out);
        assert!(!rep.has_gap(), "{rep:?}");
    }

    /// 缺落款日期 → 数字差集报出
    #[test]
    fn test_coverage_missing_date() {
        let layer = "正文 2025 年 12 月 15 日\n云南省发展和改革委员会";
        let out = "正文\n\n云南省发展和改革委员会\n";
        let rep = check(layer, out);
        // 字符级: 缺 2,0,5,1 (日期部分), 正文自身的 2,0,2,5 已在产物中
        assert!(rep.missing_numbers.get(&'5').copied().unwrap_or(0) >= 1);
        assert!(rep.has_gap());
        let msg = rep.render().unwrap();
        assert!(msg.contains("覆盖检查"));
    }

    /// 碎片化文本层: 数字按字符统计仍能报出缺失, 且不做行级比对 (避免假阳性)
    #[test]
    fn test_coverage_fragmented_layer() {
        let layer = "1\n2\n 月\n1\n5\n 日\n正\n文";
        let out = "正文\n";
        let rep = check(layer, out);
        assert!(rep.fragmented, "应识别为碎片层");
        assert!(rep.missing_lines.is_empty(), "碎片层不做行级比对");
        assert!(rep.missing_numbers.get(&'1').copied().unwrap_or(0) >= 1);
        assert!(rep.missing_numbers.get(&'5').copied().unwrap_or(0) >= 1);
    }

    /// 缺整行文本 → 行差集报出
    #[test]
    fn test_coverage_missing_line() {
        let layer = "第一段内容\n云南省人民政府办公厅关于改革完善的实施意见\n第二段内容";
        let out = "第一段内容\n\n第二段内容\n";
        let rep = check(layer, out);
        assert!(rep
            .missing_lines
            .iter()
            .any(|l| l.contains("云南省人民政府办公厅")));
    }
}
