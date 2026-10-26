//! 确定性硬换行重组 (reflow)
//!
//! PDF 没有段落概念: 文本按版宽断行, 文本层/OCR 返回的换行是「版式换行」
//! 而非「语义换行」—— `省人力资源社会\n保障厅` 这种词被拦腰截断的行, 本身
//! 就证明是版宽换行。本模块按确定性规则把段内硬换行拼回连续文本:
//!
//! - 拼接条件: 当前行行尾**不是**终结标点, 且下一行行首**不是**结构标记;
//! - 豁免: 围栏代码块、Markdown 表格整块透传;
//! - 边界: CJK|CJK 直接拼接; 两侧均为 ASCII 字母数字时补一个空格。
//!
//! 这是结构归一而非内容改写: 字符逐字保留, 只删除版式换行。
//! 语义歧义 (刻意一行一项的竖排名单) 不在本层猜测, 由后续验证兜底。

use super::cross_verify::is_markdown_table;

/// 行尾终结标点: 句读收束, 其后换行可能是语义边界
pub(crate) fn ends_terminal(line: &str) -> bool {
    let t = line.trim_end();
    let Some(c) = t.chars().last() else {
        return true; // 空行视为边界
    };
    matches!(
        c,
        '。' | '！'
            | '？'
            | '；'
            | '：'
            | '…'
            | '》'
            | '」'
            | '』'
            | '】'
            | '）'
            | '〉'
            | '”'
            | '’'
            | '!'
            | '?'
            | ';'
            | ':'
            | '"'
    )
}

/// 行首结构标记: 标题/列表/表格/引用/围栏等, 不与上一行拼接
pub(crate) fn starts_structural(line: &str) -> bool {
    let t = line.trim_start();
    if t.is_empty() {
        return true;
    }
    let c = t.chars().next().unwrap_or(' ');
    if matches!(c, '#' | '|' | '>' | '*' | '-' | '+' | '`' | '~' | '!' | '<') {
        return true;
    }
    if t.starts_with("```") || t.starts_with("~~~") {
        return true;
    }
    // 中文序号: 一、 二、 ... 十、 及 （一） 1. 1、 第X条/章/节/编
    let cjk_num: &str = "一二三四五六七八九十";
    let mut chars = t.chars();
    if let Some(first) = chars.next() {
        if cjk_num.contains(first) && matches!(chars.next(), Some('、') | Some('．') | Some('.'))
        {
            return true;
        }
    }
    if t.starts_with("（") || t.starts_with("(") {
        return true; // （一） 列举; 保守整体豁免
    }
    if t.starts_with("第") {
        let rest: String = t.chars().skip(1).take(8).collect();
        if ["条", "章", "节", "编", "部分"]
            .iter()
            .any(|u| rest.contains(u))
        {
            return true;
        }
    }
    // ASCII 编号列表: 1. 2. 3) 4、
    if t.chars().take_while(|c| c.is_ascii_digit()).count() >= 1 {
        let after: String = t
            .chars()
            .skip_while(|c| c.is_ascii_digit())
            .take(1)
            .collect();
        if matches!(after.as_str(), "." | "、" | "）" | ")" | "．") {
            return true;
        }
    }
    false
}

fn is_ascii_word(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// 拼接两行: 拉丁词边界补空格, 其余直拼
fn join_lines(a: &str, b: &str) -> String {
    let (ca, cb) = (a.trim_end().chars().last(), b.trim_start().chars().next());
    let mut out = String::from(a.trim_end());
    if let (Some(x), Some(y)) = (ca, cb) {
        if is_ascii_word(x) && is_ascii_word(y) {
            out.push(' ');
        }
    }
    out.push_str(b.trim_start());
    out
}

/// 对整个 Markdown 做段内硬换行重组
pub fn reflow(md: &str) -> String {
    let mut out_blocks: Vec<String> = Vec::new();
    for block in md.split("\n\n") {
        // 围栏块与表格整块透传
        let has_fence = block
            .lines()
            .any(|l| l.trim_start().starts_with("```") || l.trim_start().starts_with("~~~"));
        if has_fence || is_markdown_table(block) {
            out_blocks.push(block.to_string());
            continue;
        }
        // 段内逐行折叠
        let mut folded: Option<String> = None;
        for line in block.lines() {
            match folded {
                None => folded = Some(line.to_string()),
                Some(prev) => {
                    if ends_terminal(&prev) || starts_structural(line) || starts_structural(&prev) {
                        folded = Some(format!("{prev}\n{line}"));
                    } else {
                        folded = Some(join_lines(&prev, line));
                    }
                }
            }
        }
        out_blocks.push(folded.unwrap_or_default());
    }
    let mut result = out_blocks.join("\n\n");
    while result.ends_with("\n\n") {
        result.truncate(result.len() - 1);
    }
    if !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// N078 实测样例: 收文单位列表被版宽截断, 应拼回一行
    #[test]
    fn test_reflow_n078_recipients() {
        let input = "省委社会工作部、省委网信办，省工业和信息化厅、省教育厅、\n省科技厅、省公安厅、省民政厅、省财政厅、省人力资源社会\n保障厅、省自然资源厅、省生态环境厅、省住房城乡建设厅、\n省交通运输厅、省农业农村厅、省水利厅、省商务厅、省文化\n和旅游厅、省卫生健康委、省应急管理厅、省外办，省市场监\n管局、省能源局、省数据局、昆明海关、中国人民银行云南省\n分行：";
        let out = reflow(input);
        assert_eq!(out.lines().count(), 1, "应拼为一行: {out}");
        assert!(out.contains("省人力资源社会保障厅"));
        assert!(out.contains("省文化和旅游厅"));
        assert!(out.contains("省市场监管局"));
        assert!(out.trim_end().ends_with("分行："));
    }

    /// 语义边界保留: 句读收束、结构标记行
    #[test]
    fn test_reflow_preserves_semantic_breaks() {
        let input = "第一条 为了规范招标投标活动。\n第二条 本法适用于招标投标。\n# 标题\n\n|a|b|\n|--|--|\n|1|2|";
        let out = reflow(input);
        assert!(out.contains("第一条 为了规范招标投标活动。\n第二条"));
        assert!(out.contains("# 标题"));
        assert!(out.contains("|a|b|"), "表格透传");
    }

    /// 围栏块透传
    #[test]
    fn test_reflow_fence_passthrough() {
        let input = "```\nabc def\nghi jkl\n```\n";
        assert_eq!(reflow(input), input);
    }

    /// 拉丁词边界补空格
    #[test]
    fn test_reflow_latin_space() {
        let input = "GB/T 12345 与 ISO\n9001 认证体系";
        let out = reflow(input);
        assert!(out.contains("ISO 9001"));
    }
}
