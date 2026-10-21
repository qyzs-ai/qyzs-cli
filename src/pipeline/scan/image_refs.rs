//! MinerU 图片引用的落盘缺口检测与标注
//!
//! MinerU 的 `md_content` 会内联 `![](images/<sha256>.jpg)` 形式的图片引用,
//! 但本项目所接入的网关只返回 Markdown 正文, 没有对应的图片下载端点
//! (实测 `/images` `/zip` `/result` 均 404, 响应体里也没有图片字节)。
//! 结果是: 产物里的图片引用全部指向不存在的文件 —— 对 RAG 入库而言是死链。
//!
//! 这里不擅自改写正文 (图片引用是 MinerU 的事实, 抹掉会丢失"此处有图"的信息),
//! 而是:
//! 1. 统计无法落盘的图片引用数量;
//! 2. 在报告里如实给出计数与告警, 让调用方知道产物含 N 处未落盘图片;
//! 3. 提供可选的显式标注, 便于下游识别这些引用。

use std::collections::HashSet;

/// 提取 Markdown 图片引用的目标地址
///
/// 手写扫描而非引入正则依赖: 语法足够简单 (`![alt](target)`), 且本项目
/// 刻意保持依赖面收敛。返回 (目标, 结束位置) 便于继续扫描。
fn next_image_target(text: &str) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        // 定位 `![`
        if bytes[i] != b'!' || bytes[i + 1] != b'[' {
            i += 1;
            continue;
        }
        // 跳过 alt 文本 (不支持嵌套 `]`; Markdown 规范本身也不允许)
        let alt_start = i + 2;
        let Some(alt_end_rel) = text[alt_start..].find(']') else {
            return None;
        };
        let alt_end = alt_start + alt_end_rel;
        // 必须紧跟 `(`
        if alt_end + 1 >= bytes.len() || bytes[alt_end + 1] != b'(' {
            i = alt_end + 1;
            continue;
        }
        // 目标地址: 到未转义的 `)` 为止, 不含空白
        let target_start = alt_end + 2;
        let Some(target_end_rel) = text[target_start..].find(')') else {
            return None;
        };
        let target_end = target_start + target_end_rel;
        let target = text[target_start..target_end].trim();
        // 目标内含空白 → 不是合法的简单引用 (可能是带标题的引用, 跳过)
        if target.is_empty() || target.contains(char::is_whitespace) {
            i = target_end + 1;
            continue;
        }
        return Some((target.to_string(), target_end + 1));
    }
    None
}

/// 统计 Markdown 中的图片引用
pub fn count_image_refs(markdown: &str) -> usize {
    let mut rest = markdown;
    let mut count = 0;
    while let Some((_, next)) = next_image_target(rest) {
        count += 1;
        rest = &rest[next..];
    }
    count
}

/// 收集 Markdown 中的图片引用目标 (按出现顺序, 去重)
pub fn collect_image_targets(markdown: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut rest = markdown;
    while let Some((target, next)) = next_image_target(rest) {
        if seen.insert(target.clone()) {
            out.push(target);
        }
        rest = &rest[next..];
    }
    out
}

/// 判定图片引用是否指向本地可落盘的路径
///
/// 只有相对路径 (非 http/data:) 才可能由本地文件补齐; 网关给出的 `images/xxx.jpg`
/// 属于此类, 但因网关未提供字节来源, 本项目无法落盘 → 记为未落盘。
pub fn is_resolvable(target: &str) -> bool {
    !target.starts_with("http://")
        && !target.starts_with("https://")
        && !target.starts_with("data:")
        && !target.is_empty()
}

/// 图片引用统计结果
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImageRefStats {
    /// 图片引用总数
    pub total: usize,
    /// 目标为远程/内联 (无需本地落盘) 的引用数
    pub remote: usize,
    /// 指向本地相对路径但当前无法落盘的引用数
    pub unresolved: usize,
    /// 已实际存在于基准目录的本地引用数 (仅 analyze_with_base 会填充)
    pub resolved: usize,
}

impl ImageRefStats {
    /// 统计 Markdown 的图片引用状况
    pub fn analyze(markdown: &str) -> Self {
        Self::analyze_with_base(markdown, None)
    }

    /// 统计并校验本地引用是否真实存在
    ///
    /// `base` 为 .md 所在目录: 给出时会检查相对路径文件是否存在, 已落盘的
    /// 图片不再计入 unresolved (否则 scan 刚写完 assets 就报"未落盘"是假告警)。
    pub fn analyze_with_base(markdown: &str, base: Option<&std::path::Path>) -> Self {
        let mut stats = ImageRefStats::default();
        for target in collect_image_targets(markdown) {
            stats.total += 1;
            if !is_resolvable(&target) {
                stats.remote += 1;
            } else if let Some(dir) = base {
                if dir.join(&target).exists() {
                    stats.resolved += 1;
                } else {
                    stats.unresolved += 1;
                }
            } else {
                stats.unresolved += 1;
            }
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counts_image_refs() {
        let md = "文字\n\n![](images/a.jpg)\n\n更多 ![图](images/b.png) 文字";
        let stats = ImageRefStats::analyze(md);
        assert_eq!(stats.total, 2);
        assert_eq!(stats.unresolved, 2);
        assert_eq!(stats.remote, 0);
    }

    #[test]
    fn test_remote_refs_not_unresolved() {
        let md = "![x](https://example.com/a.png) ![y](data:image/png;base64,AAA)";
        let stats = ImageRefStats::analyze(md);
        assert_eq!(stats.total, 2);
        assert_eq!(stats.remote, 2);
        assert_eq!(stats.unresolved, 0);
    }

    #[test]
    fn test_non_image_markdown_ignored() {
        let md = "# 标题\n\n[链接](https://example.com)\n\n普通文本";
        assert_eq!(ImageRefStats::analyze(md).total, 0);
    }

    #[test]
    fn test_dedup_targets() {
        let md = "![](images/a.jpg)\n![](images/a.jpg)\n![](images/b.jpg)";
        let targets = collect_image_targets(md);
        assert_eq!(targets, vec!["images/a.jpg", "images/b.jpg"]);
    }

    #[test]
    fn test_empty_input() {
        let stats = ImageRefStats::analyze("");
        assert_eq!(stats.total, 0);
        assert_eq!(stats.unresolved, 0);
    }
}
