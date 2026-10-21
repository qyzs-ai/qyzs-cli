//! 类型特化的排版优化 (markscan 的 profile 阶段)
//!
//! 依据 classify 的判定结果套不同规则。核心动机来自实测: 抽取通道常把层级压平
//! (MinerU 对 34 页国标输出 126 个标题全部落在 `#`/`##` 两级), 而这类文档的
//! 层级信息**不在字号里**——国标正文与标题同为 9.7pt, 法典全篇 9.0pt 单一字号。
//! 层级只存在于编号文本 (第X编/章/节/条、一、（一）、1.)。
//!
//! 因此这里不猜字号, 只做两件确定的事:
//! 1. 标准规范: 按编号体裁重建标题层级 (编>分编>章>节>条)
//! 2. 公文: 按「一、（一）1.」公文序号体系重建层级
//!
//! 明确不做: 按编号数值重排段落顺序。实测中虽能看到「第一章 后面跟第二节」这类
//! 顺序错误, 但重排需要判定跨页/跨节边界, 误判代价高于收益 —— 宁可保留原序并
//! 让结构审查报告出来, 也不做不可逆的重排。

use super::classify::DocKind;

/// 依据文档类型做排版优化
pub fn apply(kind: DocKind, md: &str) -> String {
    let rebuilt = apply_kind(kind, md);
    // 所有类型统一做标题文本规整: 抽取通道会把原文粗体转成 **...**,
    // 整行落到标题里是冗余噪声, 与体裁无关
    normalize_headings(&rebuilt)
}

fn apply_kind(kind: DocKind, md: &str) -> String {
    match kind {
        DocKind::Standard => {
            let (promoted, _) = promote_numbered_headings(md);
            rebuild_standard_hierarchy(&promoted)
        }
        // 合同条款体系与公文办法体裁同构 (章/节/条 + （一）/1.), 共用自适应定级
        DocKind::Official | DocKind::Contract => {
            let (promoted, _) = promote_numbered_headings(md);
            rebuild_official_hierarchy(&promoted)
        }
        // 表格/演示/图示已是结构化输出; 图像由 OCR/VL 直接产出; 通用文档无特有体裁
        _ => md.to_string(),
    }
}

/// 编号体裁 (按由外到内的规范次序排列)
///
/// 定级不看固定表, 而是先扫全文看**实际出现**了哪些体裁, 再按此次序压紧排名。
/// 原因: 同一个「（一）」在通知体裁里是二级, 在「办法」体裁里位于第三章第X条
/// 之下却是五级。固定映射必然错, 只能按文档自身用到的体系自适应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Unit {
    /// 第X编
    Part,
    /// 第X分编
    SubPart,
    /// 第X章
    Chapter,
    /// 第X节
    Section,
    /// 第X条
    Article,
    /// 一、
    CjkComma,
    /// （一）
    CjkParen,
    /// 1.
    ArabicDot,
    /// （1）
    ArabicParen,
}

/// 规范次序 (外 → 内)
const CANONICAL: &[Unit] = &[
    Unit::Part,
    Unit::SubPart,
    Unit::Chapter,
    Unit::Section,
    Unit::Article,
    Unit::CjkComma,
    Unit::CjkParen,
    Unit::ArabicDot,
    Unit::ArabicParen,
];

/// 判定标题属于哪种编号体裁
fn unit_of(title: &str) -> Option<Unit> {
    if matches_numbered_unit(title, "编") {
        return Some(Unit::Part);
    }
    if matches_numbered_unit(title, "分编") {
        return Some(Unit::SubPart);
    }
    if matches_numbered_unit(title, "章") {
        return Some(Unit::Chapter);
    }
    if matches_numbered_unit(title, "节") {
        return Some(Unit::Section);
    }
    if matches_numbered_unit(title, "条") {
        return Some(Unit::Article);
    }
    if is_cjk_comma_heading(title) {
        return Some(Unit::CjkComma);
    }
    if is_cjk_paren_item(title) {
        return Some(Unit::CjkParen);
    }
    if is_arabic_dot_heading(title) {
        return Some(Unit::ArabicDot);
    }
    if is_arabic_paren_heading(title) {
        return Some(Unit::ArabicParen);
    }
    None
}

/// 自适应定级: 仅按文中出现的体裁压紧排名
fn adaptive_level_map(headings: &[&str]) -> std::collections::HashMap<Unit, usize> {
    let present: std::collections::HashSet<Unit> =
        headings.iter().filter_map(|h| unit_of(h)).collect();
    let mut map = std::collections::HashMap::new();
    // 文档标题占位: 首个标题不是编号单元、且其后确有编号单元 → 结构从二级起。
    // 否则出现「一式两份」里标题与"第一章"同为一级, 读起来是并列关系而非包含关系。
    let mut rank = usize::from(has_document_title(headings));
    for u in CANONICAL {
        if present.contains(u) {
            rank += 1;
            map.insert(*u, rank);
        }
    }
    map
}

/// 标准规范层级 (编 > 分编 > 章 > 节 > 条)
fn standard_level(title: &str, ranks: &std::collections::HashMap<Unit, usize>) -> Option<usize> {
    match unit_of(title)? {
        Unit::Part | Unit::SubPart | Unit::Chapter | Unit::Section | Unit::Article => {
            ranks.get(&unit_of(title)?).copied()
        }
        // 标准里（一）作为条款内的项, 排在条之后
        u => ranks.get(&u).copied(),
    }
}

/// 公文层级 (条文体裁: 编>章>节>条; 段落体裁: 一、>（一）>1.>（1）)
fn official_level(title: &str, ranks: &std::collections::HashMap<Unit, usize>) -> Option<usize> {
    let u = unit_of(title)?;
    ranks.get(&u).copied()
}

/// 是否为「第X<unit>」形式的编号开头
fn matches_numbered_unit(title: &str, unit: &str) -> bool {
    let Some(rest) = title.strip_prefix('第') else {
        return false;
    };
    let mut digits_end = 0usize;
    let mut saw_digit = false;
    for (i, c) in rest.char_indices() {
        if is_cjk_numeral(c) || c.is_ascii_digit() {
            saw_digit = true;
            digits_end = i + c.len_utf8();
        } else {
            break;
        }
    }
    if !saw_digit {
        return false;
    }
    rest[digits_end..].trim_start().starts_with(unit)
}

fn is_cjk_numeral(c: char) -> bool {
    matches!(
        c,
        '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十' | '百' | '千'
            | '零' | '〇' | '两'
    )
}

/// 一、  二、
fn is_cjk_comma_heading(title: &str) -> bool {
    let Some(first) = title.chars().next() else {
        return false;
    };
    if !is_cjk_numeral(first) {
        return false;
    }
    let rest = &title[first.len_utf8()..];
    rest.trim_start_matches(is_cjk_numeral).starts_with('、')
}

/// （一）
fn is_cjk_paren_item(title: &str) -> bool {
    let Some(rest) = title.strip_prefix('（').or_else(|| title.strip_prefix('(')) else {
        return false;
    };
    let trimmed = rest.trim_start();
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    if !is_cjk_numeral(first) {
        return false;
    }
    let after: String = trimmed.chars().skip_while(|c| is_cjk_numeral(*c)).collect();
    after.starts_with('）') || after.starts_with(')')
}

/// 1.
fn is_arabic_dot_heading(title: &str) -> bool {
    let digits: String = title.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() || digits.len() > 2 {
        return false;
    }
    let rest = &title[digits.len()..];
    if let Some(r) = rest.strip_prefix('.').or_else(|| rest.strip_prefix('．')) {
        !r.is_empty() && !r.starts_with(|c: char| c.is_ascii_digit())
    } else {
        false
    }
}

/// （1）
fn is_arabic_paren_heading(title: &str) -> bool {
    let Some(rest) = title.strip_prefix('（').or_else(|| title.strip_prefix('(')) else {
        return false;
    };
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return false;
    }
    let after = &rest[digits.len()..];
    after.starts_with('）') || after.starts_with(')')
}

/// 按给定判级函数重建标题层级
///
/// 只调整 `#` 数量, 不增删标题、不改标题文字、不改段落顺序。
fn rebuild_with(md: &str, level_of: impl Fn(&str, &std::collections::HashMap<Unit, usize>) -> Option<usize>) -> String {
    // 第一遍: 收集围栏外的标题, 用于自适应定级
    let headings: Vec<&str> = {
        let mut v = Vec::new();
        let mut in_fence = false;
        for line in md.lines() {
            let t = line.trim_start();
            if t.starts_with("```") || t.starts_with("~~~") {
                in_fence = !in_fence;
                continue;
            }
            if !in_fence {
                if let Some(title) = heading_title(line) {
                    v.push(title);
                }
            }
        }
        v
    };
    let ranks = adaptive_level_map(&headings);

    let mut out = String::with_capacity(md.len());
    let mut in_fence = false;
    let mut changed = 0usize;

    for line in md.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_fence {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if let Some(title) = heading_title(line) {
            if let Some(level) = level_of(title, &ranks) {
                out.push_str(&format!(
                    "{} {}\n",
                    "#".repeat(level.min(6)),
                    normalize_heading_text(title)
                ));
                changed += 1;
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    // 保护: 若一条都没重定级, 原样返回 (避免无意义地重建换行)
    if changed == 0 {
        return md.to_string();
    }
    out.trim_end_matches('\n').to_string() + "\n"
}

/// 规整所有标题行的文本 (去冗余强调包裹)
pub fn normalize_headings(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut in_fence = false;
    let mut changed = 0usize;
    for line in md.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if !in_fence {
            if let Some(title) = heading_title(line) {
                let norm = normalize_heading_text(title);
                if norm != title {
                    out.push_str(&format!("{} {}\n", "#".repeat(t.chars().take_while(|c| *c == '#').count()), norm));
                    changed += 1;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if changed == 0 {
        return md.to_string();
    }
    out.trim_end_matches('\n').to_string() + "\n"
}

/// 规整标题文本: 去掉整体包裹的强调标记
///
/// 抽取通道会把原文粗体转成 `**...**`, 整行包裹时落到标题里就是
/// `# 2 **适用范围**` —— 标题本身已是强调, 再套一层是冗余噪声。
/// 只处理整行包裹, 标题中段的强调保留 (可能承载语义)。
fn normalize_heading_text(title: &str) -> String {
    // 标题内含强调标记一律移除: 标题本身已是强调, 再套一层是噪声。
    // 只处理成对的 ** / __ (单 * 可能是正文符号, 不碰)。
    let mut out = title.trim().replace("**", "").replace("__", "");
    out = out.trim().to_string();
    out
}


/// 取 ATX 标题的标题文本
fn heading_title(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let hashes = t.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &t[hashes..];
    if !rest.starts_with(' ') && !rest.starts_with('\t') {
        return None;
    }
    Some(rest.trim())
}

/// 结构缺失判定: 编号行数超过现有标题数的此倍数时, 视为结构大部分未产出
///
/// 用比例而非绝对数: 条例类文档常有 7~10 个章标题, 但下辖数百条条文,
/// 绝对阈值会把它们误判为"已有结构"而放弃补建。
const PROMOTE_RATIO: usize = 2;

/// 结构缺失时, 从编号文本补建标题
///
/// 动机: 合同、部分公文在抽取结果里章/条与正文同字号且无内置样式, 抽取通道
/// 不产出任何标题 —— 此时「重定级」无从下手 (没有标题可改)。而「第一章」「第一条」
/// 这类行在中文法律文本中是确定的结构单元, 可按编号补建。
///
/// **只在结构几乎缺失时启用** (现有标题 < 5): 已产出结构的文档 (如法典 82 个
/// 标题) 不做补建, 避免把上千条文行全部升级成标题、把文档打散。
///
/// 补建只处理结构单元 (第X编/章/节/条); `（一）`/`1.` 这类条款内的项不升级,
/// 它们通常是正文列举而非独立章节。
pub fn promote_numbered_headings(md: &str) -> (String, usize) {
    let existing = md
        .lines()
        .filter(|l| heading_title(l).is_some())
        .count();
    let unit_lines = md
        .lines()
        .filter(|l| heading_title(l).is_none() && is_structural_unit(l.trim()))
        .count();
    // 结构已基本完整 (编号行没有显著多于标题) → 不补建, 避免把正文条文全部升级
    if unit_lines <= existing * PROMOTE_RATIO {
        return (md.to_string(), 0);
    }

    let mut out = String::with_capacity(md.len() + 256);
    let mut in_fence = false;
    let mut promoted = 0usize;
    for line in md.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if !in_fence && heading_title(line).is_none() {
            let text = t.trim();
            if is_structural_unit(text) {
                // 层级先给 1, 随后由 adaptive 重建压紧到真实层级
                out.push_str(&format!("# {}\n", text));
                promoted += 1;
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if promoted == 0 {
        return (md.to_string(), 0);
    }
    (out, promoted)
}

/// 是否为结构单元行 (第X编/分编/章/节/条)
fn is_structural_unit(text: &str) -> bool {
    if text.chars().count() > 120 {
        return false; // 过长更像整段正文
    }
    if !text.starts_with('第') {
        return false;
    }
    ["编", "章", "节", "条"]
        .iter()
        .any(|u| matches_numbered_unit(text, u))
}

/// 用「文本 → 层级」映射修正标题层级 (来自 DOCX 样式或 PDF 字号)
///
/// 只改已存在的标题行的 `#` 数量, 不新增标题、不改文字。文本按去空白归一后
/// 精确匹配 —— 宁可漏改, 不可错改: 把正文段落误升为标题比层级不准更糟。
pub fn apply_style_levels(md: &str, levels: &[(String, usize)]) -> (String, usize) {
    if levels.is_empty() {
        return (md.to_string(), 0);
    }
    let map: std::collections::HashMap<String, usize> = levels
        .iter()
        .map(|(t, l)| (normalize_text(t), *l))
        .collect();

    let mut out = String::with_capacity(md.len());
    let mut changed = 0usize;
    let mut in_fence = false;
    for line in md.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if !in_fence {
            if let Some(title) = heading_title(line) {
                if let Some(level) = map.get(&normalize_text(title)) {
                    out.push_str(&format!(
                        "{} {}\n",
                        "#".repeat((*level).clamp(1, 6)),
                        normalize_heading_text(title)
                    ));
                    changed += 1;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if changed == 0 {
        return (md.to_string(), 0);
    }
    (out.trim_end_matches('\n').to_string() + "\n", changed)
}

/// 文本归一: 去空白与常见标点差异, 用于跨来源匹配
fn normalize_text(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '（' => '(',
            '）' => ')',
            '：' => ':',
            '，' => ',',
            '　' => ' ',
            // 强调标记不参与匹配: 一边可能带 **, 另一边不带
            '*' | '_' => ' ',
            other => other,
        })
        .filter(|c| *c != ' ')
        .collect()
}

/// 首个标题是否为「文档标题」(非编号单元), 且其后存在编号单元
fn has_document_title(headings: &[&str]) -> bool {
    let mut it = headings.iter().filter(|h| !h.trim().is_empty());
    let Some(first) = it.next() else {
        return false;
    };
    unit_of(first).is_none() && it.any(|h| unit_of(h).is_some())
}

/// 标准规范层级重建
pub fn rebuild_standard_hierarchy(md: &str) -> String {
    rebuild_with(md, standard_level)
}

/// 公文层级重建
pub fn rebuild_official_hierarchy(md: &str) -> String {
    rebuild_with(md, official_level)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_hierarchy_rebuilt() {
        let md = "# 生态环境法典\n\n# 第一编 总则\n\n# 第一章 基本规定\n\n# 第一节 一般规定\n\n# 第一条 为保护和改善环境\n";
        let out = rebuild_standard_hierarchy(md);
        assert!(out.contains("# 第一编 总则"));
        assert!(out.contains("## 第一章 基本规定"));
        assert!(out.contains("### 第一节 一般规定"));
        assert!(out.contains("#### 第一条 为保护和改善环境"));
    }

    #[test]
    fn test_standard_sub_part_level() {
        let md = "# 第二编 污染防治\n# 第一分编 通则\n# 第一章 基本规定\n";
        let out = rebuild_standard_hierarchy(md);
        assert!(out.contains("## 第一分编 通则"));
        assert!(out.contains("## 第一章 基本规定"));
    }

    #[test]
    fn test_official_hierarchy_rebuilt() {
        let md = "# 一、总体要求\n# （一）基本原则\n# 1. 坚持统筹推进\n# （1）部门协同\n";
        let out = rebuild_official_hierarchy(md);
        assert!(out.contains("# 一、总体要求"));
        assert!(out.contains("## （一）基本原则"));
        assert!(out.contains("### 1. 坚持统筹推进"));
        assert!(out.contains("#### （1）部门协同"));
    }

    #[test]
    fn test_no_numbering_keeps_original() {
        // 无编号体裁: 一条都不重定级, 原样返回
        let md = "# 标题\n\n## 小节\n\n正文。\n";
        assert_eq!(rebuild_standard_hierarchy(md), md);
        assert_eq!(rebuild_official_hierarchy(md), md);
    }

    #[test]
    fn test_content_not_modified() {
        // 编+章: 章自适应为二级
        let md = "# 第一编 总则\n\n# 第一章 基本规定\n\n正文内容 123,456.78 元不能被改动。\n";
        let out = rebuild_standard_hierarchy(md);
        assert!(out.contains("正文内容 123,456.78 元不能被改动。"));
        assert!(out.contains("## 第一章 基本规定"));
    }

    #[test]
    fn test_adaptive_only_chapter_is_top_level() {
        // 只出现「章」: 章即最高级 (自适应压紧, 不硬套固定表)
        let md = "# 第一章 总则\n# 第二章 附则\n";
        let out = rebuild_standard_hierarchy(md);
        assert!(out.contains("# 第一章 总则"));
        assert!(out.contains("# 第二章 附则"));
    }

    #[test]
    fn test_adaptive_chapter_deeper_than_article() {
        // 公文办法体裁: 章 > 条 > （一）, 三者不得同级
        let md = "# 第一章 总则\n# 第一条 为了规范管理\n# （一）适用范围\n";
        let out = rebuild_official_hierarchy(md);
        assert!(out.contains("# 第一章 总则"));
        assert!(out.contains("## 第一条 为了规范管理"));
        assert!(out.contains("### （一）适用范围"));
    }

    #[test]
    fn test_fence_immune() {
        let md = "# 第一编 总则\n\n# 第一章 基本规定\n```\n# 第二编 这是代码注释\n```\n";
        let out = rebuild_standard_hierarchy(md);
        // 围栏内不动, 也不参与体裁统计
        assert!(out.contains("# 第二编 这是代码注释"));
        assert!(out.contains("## 第一章 基本规定"));
    }

    #[test]
    fn test_decimal_number_not_heading_level() {
        // "1.5倍" 不应被当作 1. 级标题
        assert!(!is_arabic_dot_heading("1.5倍标准"));
        assert!(is_arabic_dot_heading("1. 总体要求"));
    }

    #[test]
    fn test_generic_untouched() {
        let md = "# 随便\n内容";
        assert_eq!(apply(DocKind::Generic, md), md);
        assert_eq!(apply(DocKind::Table, md), md);
    }
}
