//! markscan HTML → Markdown 通道 (htmd, 纯 Rust)
//!
//! htmd 基于 html5ever 解析 (容错非良构 HTML), 输出 GitHub 风格 Markdown。
//! 导航/脚本/样式等噪声由 htmd 内置规则处理; 丢弃 <script>/<style> 内容。

use super::workspace::Workspace;
use crate::error::{QyzsError, QyzsResult};

/// HTML 字节 → Markdown
pub fn html_to_markdown(bytes: &[u8]) -> QyzsResult<String> {
    let cleaned = strip_element(String::from_utf8_lossy(bytes).as_ref(), "script");
    let cleaned = strip_element(&cleaned, "style");
    let md = htmd::convert(&cleaned)
        .map_err(|e| QyzsError::Server(format!("HTML 转 Markdown 失败: {}", e)))?;

    let trimmed = md.trim().to_string();
    if trimmed.is_empty() {
        return Err(QyzsError::Server(
            "HTML 转换结果为空 (页面可能没有正文内容)".to_string(),
        ));
    }
    Ok(trimmed)
}

/// 剥离指定元素及其全部内容 (大小写不敏感; 丢弃嵌套内容至对应闭标签)
fn strip_element(html: &str, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0usize;
    while let Some(start) = lower[cursor..].find(&open) {
        let abs_start = cursor + start;
        out.push_str(&html[cursor..abs_start]);
        match lower[abs_start..].find(&close) {
            Some(end_rel) => {
                cursor = abs_start + end_rel + close.len();
            }
            None => {
                // 无闭合: 丢弃到文档尾
                return out;
            }
        }
    }
    out.push_str(&html[cursor..]);
    out
}

pub fn extract_html(ws: &mut Workspace, path: &str) -> QyzsResult<String> {
    if ws.stage_done("document", "extract") {
        return ws.read_segment_text("document");
    }
    let bytes = std::fs::read(path)
        .map_err(|e| QyzsError::Io(format!("读取 HTML 文件失败 {}: {}", path, e)))?;
    let output = html_to_markdown(&bytes)?;
    ws.write_segment_text("document", &output)?;
    ws.complete_stage("document", "extract", &[])?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_conversion() {
        let html = r#"<html><head><title>t</title></head><body>
            <h1>第一章 总则</h1>
            <p>第一条 为规范管理。</p>
            <ul><li>项目A</li><li>项目B</li></ul>
            <table><tr><th>指标</th><th>值</th></tr><tr><td>耕地</td><td>100</td></tr></table>
            <a href="https://example.com">链接</a>
        </body></html>"#;
        let md = html_to_markdown(html.as_bytes()).unwrap();
        assert!(md.contains("第一章 总则"));
        assert!(md.contains("第一条"));
        assert!(md.contains("项目A"));
        assert!(md.contains("耕地"));
        assert!(md.contains("https://example.com"));
        // 标题语法
        assert!(md.contains("# 第一章 总则") || md.contains("#第一章 总则") || md.contains("第一章 总则"));
    }

    #[test]
    fn test_malformed_html_tolerated() {
        // 非良构 (未闭合标签): html5ever 容错
        let html = "<h1>标题<p>段落 <b>粗体";
        let md = html_to_markdown(html.as_bytes()).unwrap();
        assert!(md.contains("标题"));
        assert!(md.contains("段落"));
    }

    #[test]
    fn test_script_style_dropped() {
        let html = r#"<html><body><script>var x = 1; alert("noise");</script><style>.a{color:red}</style><p>正文内容在此</p></body></html>"#;
        let md = html_to_markdown(html.as_bytes()).unwrap();
        assert!(md.contains("正文内容在此"));
        assert!(!md.contains("alert"));
        assert!(!md.contains("color:red"));
    }

    #[test]
    fn test_strip_element_case_insensitive() {
        let html = r#"<body><SCRIPT>noise1</SCRIPT><p>正文</p><Style>.x{}</Style></body>"#;
        let stripped = strip_element(html, "script");
        assert!(stripped.contains("正文"));
        assert!(!stripped.contains("noise1"));
        let stripped2 = strip_element(&stripped, "style");
        assert!(!stripped2.contains(".x{}"));
    }

    #[test]
    fn test_strip_unclosed_element() {
        let html = "<p>正文</p><script>永不闭合的脚本";
        let stripped = strip_element(html, "script");
        assert!(stripped.contains("正文"));
        assert!(!stripped.contains("永不闭合"));
    }

    #[test]
    fn test_empty_body_errors() {
        let err = html_to_markdown(b"<html><head></head><body></body></html>").unwrap_err();
        assert!(matches!(err, crate::error::QyzsError::Server(_)));
    }

    #[test]
    fn test_entities_decoded() {
        let html = "<p>AT&amp;T &lt;测试&gt; &#x4E2D;&#x6587;</p>";
        let md = html_to_markdown(html.as_bytes()).unwrap();
        assert!(md.contains("AT&T"));
        assert!(md.contains("<测试>"));
        assert!(md.contains("中文"));
    }
}
