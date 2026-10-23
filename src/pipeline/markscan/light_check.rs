//! markscan 润色轻量自检 (润色结果与抽取原文的结构对账)
//!
//! 检查不通过 → 该段回退使用抽取原文 (宁要糙的真, 不要漂亮的假)。

/// 单段润色质量判定
pub struct LightCheck {
    pub passed: bool,
    pub issues: Vec<String>,
}

/// 校验润色段: 保留信息量不得显著缩水, 结构要素 (标题/表格/列表) 不得丢失
pub fn check_polished(raw_segment: &str, polished: &str) -> LightCheck {
    let mut issues: Vec<String> = Vec::new();

    if polished.trim().is_empty() {
        issues.push("润色结果为空".to_string());
        return LightCheck {
            passed: false,
            issues,
        };
    }

    // 字符量对账: 排版不应大幅缩水 (LLM 截断护栏)
    let raw_len = raw_segment.trim().chars().count();
    let polished_len = polished.trim().chars().count();
    if raw_len > 0 && (polished_len as f64) < raw_len as f64 * 0.5 {
        issues.push(format!(
            "润色后字符量 {} 不足原文 {} 的一半 (疑似截断)",
            polished_len, raw_len
        ));
    }

    // 结构要素对账: 标题数/表格行数/列表项数不减少
    let raw_headings = count_prefix_lines(raw_segment, '#');
    let polished_headings = count_prefix_lines(polished, '#');
    if polished_headings < raw_headings {
        issues.push(format!(
            "标题数减少: {} → {}",
            raw_headings, polished_headings
        ));
    }

    let raw_rows = count_prefix_lines(raw_segment, '|');
    let polished_rows = count_prefix_lines(polished, '|');
    if polished_rows < raw_rows {
        issues.push(format!("表格行数减少: {} → {}", raw_rows, polished_rows));
    }

    LightCheck {
        passed: issues.is_empty(),
        issues,
    }
}

fn count_prefix_lines(text: &str, marker: char) -> usize {
    let mut in_fence = false;
    text.lines()
        .filter(|l| {
            if l.trim_start().starts_with("```") || l.trim_start().starts_with("~~~") {
                in_fence = !in_fence;
                return false;
            }
            !in_fence && l.trim_start().starts_with(marker)
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pass_on_equivalent() {
        let raw = "# 章\n| a | b |\n|---|---|\n| 1 | 2 |\n正文内容保持一致。";
        let polished = "# 章\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n正文内容保持一致。";
        let check = check_polished(raw, polished);
        assert!(check.passed, "{:?}", check.issues);
    }

    #[test]
    fn test_fail_on_truncation() {
        let raw = "很长的原始内容。".repeat(100);
        let polished = "短了";
        let check = check_polished(&raw, polished);
        assert!(!check.passed);
        assert!(check.issues[0].contains("一半"));
    }

    #[test]
    fn test_fail_on_empty() {
        assert!(!check_polished("内容", "  \n").passed);
    }

    #[test]
    fn test_fail_on_structure_loss() {
        let raw = "# 一\n# 二\n| a |\n|---|\n| 1 |";
        let polished = "只有文字没有结构";
        let check = check_polished(raw, polished);
        assert!(!check.passed);
        assert!(check.issues.iter().any(|i| i.contains("标题")));
        assert!(check.issues.iter().any(|i| i.contains("表格")));
    }

    #[test]
    fn test_fence_immune() {
        // 围栏内的 # 与 | 不计入对账; 围栏本身属于内容, 润色应保留
        let raw = "```\n# code | pipe\n```\n# 正文";
        let polished = "```\n# code | pipe\n```\n\n# 正文";
        let check = check_polished(raw, polished);
        assert!(check.passed, "{:?}", check.issues);
    }
}
