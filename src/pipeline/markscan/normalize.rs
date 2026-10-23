//! 标题文本归一 (排版验证的精确匹配基础)
//!
//! 匹配两侧 (md 行 vs 模型回传) 时忽略空白与全半角标点差异, 但不忽略内容。

/// 归一化标题文本
pub fn normalize_title(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '（' => '(',
            '）' => ')',
            '：' => ':',
            '　' => ' ',
            other => other,
        })
        .filter(|c| *c != ' ')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize() {
        assert_eq!(
            normalize_title("第一章　总　则"),
            normalize_title("第一章 总则")
        );
        assert_eq!(normalize_title("（一）概述"), normalize_title("(一)概述"));
        assert_ne!(normalize_title("第一章"), normalize_title("第二章"));
    }
}
