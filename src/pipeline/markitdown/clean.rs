//! markitdown speed 档规则清洗 (行为契约: 只做结构归一，不改写内容)
//!
//! 两项且仅两项操作 (契约全文见 tests/docs/QYZS-MARKITDOWN.md §3.1):
//! 1. 标题层级相对深度归一: 按各标题级别在文中首次出现顺序映射为连续层级
//!    (如 MinerU 切片聚合后通篇从 ### 起排，归一为 #/##/##)，仅改写 `#` 数量。
//! 2. 空白规整: 3 连以上空行折叠为 1 个空行；行尾空白剥离；文件以单个换行收尾。
//!
//! 围栏代码块 (``` / ~~~) 内的一切内容原样透传——块内 `#` 不是标题，行尾空白
//! 可能承载语义。除上述规则外不做任何字符级改写。

/// 标题归一 + 空白规整
pub fn rule_clean(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();

    // 第一遍: 收集围栏外标题级别的首现顺序
    let mut level_order: Vec<usize> = Vec::new();
    let mut in_fence = false;
    let mut fence_marker = "";
    for line in &lines {
        if in_fence {
            if is_fence_close(line, fence_marker) {
                in_fence = false;
            }
            continue;
        }
        if let Some(marker) = fence_open(line) {
            in_fence = true;
            fence_marker = marker;
            continue;
        }
        if let Some(level) = heading_level(line) {
            if !level_order.contains(&level) {
                level_order.push(level);
            }
        }
    }

    // 第二遍: 应用归一与空白规整
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut blank_run: usize = 0;
    in_fence = false;
    fence_marker = "";
    for line in &lines {
        if in_fence {
            out.push((*line).to_string());
            if is_fence_close(line, fence_marker) {
                in_fence = false;
            }
            continue;
        }
        if let Some(marker) = fence_open(line) {
            in_fence = true;
            fence_marker = marker;
            blank_run = 0;
            out.push((*line).to_string());
            continue;
        }

        if line.trim().is_empty() {
            blank_run += 1;
            // 保留至多 1 个空行
            if blank_run <= 1 {
                out.push(String::new());
            }
            continue;
        }
        blank_run = 0;

        if let Some(level) = heading_level(line) {
            let rank = level_order
                .iter()
                .position(|&l| l == level)
                .map(|i| i + 1)
                .unwrap_or(level);
            let rest = line.trim_start_matches('#').trim();
            out.push(format!("{} {}", "#".repeat(rank.min(6)), rest));
            continue;
        }

        out.push(line.trim_end().to_string());
    }

    let mut result = out.join("\n");
    while result.ends_with("\n\n") {
        result.truncate(result.len() - 1);
    }
    if !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// ATX 标题行级别 (1~6)；`#######` (7+) 非法 Markdown，按普通文本处理
fn heading_level(line: &str) -> Option<usize> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    // 合法标题: # 后紧跟空格或行尾 (# 后直接接文字是普通文本，如 #tag)
    if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
        Some(hashes)
    } else {
        None
    }
}

/// 围栏开行: 返回围栏标记 (``` 或 ~~~)
fn fence_open(line: &str) -> Option<&'static str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") {
        Some("```")
    } else if trimmed.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

/// 围栏闭行: 与开行同标记且不携带信息串
fn is_fence_close(line: &str, marker: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with(marker) && trimmed[marker.len()..].trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heading_relative_depth_normalization() {
        // MinerU 切片聚合典型形态: 通篇从 ### 起排 → 按首现顺序归一为 #/##
        let input = "### 第一章 总则\n### 第二章 条款\n#### 2.1 细则\n#### 2.2 附则\n### 第三章 附则\n";
        let out = rule_clean(input);
        assert!(out.starts_with("# 第一章 总则\n"));
        assert!(out.contains("# 第三章 附则"));
        assert!(out.contains("## 2.1 细则"));
        assert!(out.contains("## 2.2 附则"));
    }

    #[test]
    fn test_heading_preserves_hierarchy_order() {
        // 已从 # 起排的规范文档: 层级映射不变
        let input = "# 标题\n## 节\n### 小节\n";
        assert_eq!(rule_clean(input), input);
    }

    #[test]
    fn test_heading_content_untouched() {
        // 只改 # 数量，标题文字原样 (含内部空格/标点)
        let input = "##### 4.3.2  建设用地指标（修订版）: A-1\n";
        let out = rule_clean(input);
        assert!(out.contains("# 4.3.2  建设用地指标（修订版）: A-1"));
        assert!(!out.contains("#####"));
    }

    #[test]
    fn test_fenced_blocks_immune() {
        // 围栏内 # 不是标题、空白原样
        let input = "# 正文标题\n```python\n# 这是注释\n### 也像标题\n    indented  keep  \n```\n####### 七级非标题\n";
        let out = rule_clean(input);
        assert!(out.contains("# 这是注释"));
        assert!(out.contains("### 也像标题"));
        assert!(out.contains("    indented  keep  "));
        // 七级非法标题不参与归一 (按普通文本处理，且行尾空白被剥离)
        assert!(out.contains("####### 七级非标题"));
    }

    #[test]
    fn test_fence_marker_mismatch_not_closed() {
        // ~~~ 开的围栏不会被 ``` 行关闭
        let input = "~~~\n### not heading\n```\nstill fenced\n~~~\n# real heading\n";
        let out = rule_clean(input);
        assert!(out.contains("### not heading"));
        assert!(out.contains("still fenced"));
        assert!(out.starts_with("# real heading") || out.contains("\n# real heading"));
    }

    #[test]
    fn test_whitespace_collapse_and_trailing_strip() {
        let input = "段落一\n\n\n\n\n段落二   \n下一段\t\n\n\n\n段落三\n";
        let out = rule_clean(input);
        assert_eq!(out, "段落一\n\n段落二\n下一段\n\n段落三\n");
    }

    #[test]
    fn test_inline_hash_not_heading() {
        // #tag 无空格不是标题，不参与归一
        let input = "#tag 内容\n# 真标题\n";
        let out = rule_clean(input);
        assert!(out.contains("#tag 内容"));
        assert!(out.contains("# 真标题"));
    }

    #[test]
    fn test_content_preservation() {
        // 数字与文字逐字保留
        let input = "### 段\n金额 123,456.78 元；比例 10%/90%；编号 GB/T 12345-2020。\n\n\n\n### 段二\n";
        let out = rule_clean(input);
        assert!(out.contains("金额 123,456.78 元；比例 10%/90%；编号 GB/T 12345-2020。"));
    }

    #[test]
    fn test_empty_and_single_line() {
        assert_eq!(rule_clean(""), "");
        assert_eq!(rule_clean("\n\n\n"), "");
        assert_eq!(rule_clean("单行"), "单行\n");
    }
}
