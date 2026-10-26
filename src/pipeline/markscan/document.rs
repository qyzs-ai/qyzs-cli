//! markscan 内部文档结果与确定性输出渲染。
//!
//! 当前上游主要返回 Markdown/纯文本，尚不稳定提供区域坐标。因此本层先保留最终
//! Markdown 作为无损内容载体，并提供 Markdown、HTML、纯文本三种确定性视图。
//! PAGE XML / ALTO XML 必须依赖真实页面尺寸与 bbox，不能从 Markdown 伪造；待
//! 结构化 OCR 接口接入后再在本模块扩展标准导出器。

use crate::error::{QyzsError, QyzsResult};
use pulldown_cmark::{html, Event, Options, Parser, Tag, TagEnd};

/// markscan 文档输出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentOutputFormat {
    Markdown,
    Html,
    Text,
}

/// 页面或区域坐标。值保持在原始页面坐标系中；未知时必须为 `None`，不得伪造。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// 内部区域类型。后续接入 PAGE/ALTO 时映射到对应标准区域类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegionKind {
    Heading(u8),
    Paragraph,
    Table,
    Code,
    List,
    Quote,
    Image,
    Raw,
}

/// 一个识别候选。置信度仅接收引擎原生分数或经基准集校准后的分数；模型自报的
/// “可信度”不得写入该字段。
#[derive(Debug, Clone, PartialEq)]
pub struct TextCandidate {
    pub source: String,
    pub text: String,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationStatus {
    Unchecked,
    Agreed,
    Corrected,
    NeedsReview,
}

/// 页面内的最小结构区域。`markdown` 保留结构表示，`text` 保存纯文字视图。
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub id: String,
    pub kind: RegionKind,
    pub bbox: Option<BoundingBox>,
    pub reading_order: usize,
    pub markdown: String,
    pub text: String,
    pub selected_source: String,
    pub confidence: Option<f64>,
    pub candidates: Vec<TextCandidate>,
    pub verification: VerificationStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub number: usize,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub regions: Vec<Region>,
}

impl DocumentOutputFormat {
    pub fn parse(value: &str) -> QyzsResult<Self> {
        match value {
            "md" => Ok(Self::Markdown),
            "html" => Ok(Self::Html),
            "txt" => Ok(Self::Text),
            other => Err(QyzsError::Usage(format!(
                "未知输出格式 '{other}'，可选: md|html|txt"
            ))),
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Html => "html",
            Self::Text => "txt",
        }
    }

    pub fn key(self) -> &'static str {
        self.extension()
    }
}

/// 流水线内部文档。
///
/// 先把最终 Markdown 封装成独立文档对象，避免输出格式逻辑继续散落在 CLI 和
/// 多文件编排层。未来结构化 OCR 接入后，可在不改变调用方的前提下扩充 pages、
/// regions、reading order、tables 与 geometry。
#[derive(Debug, Clone)]
pub struct Document {
    markdown: String,
    pub pages: Vec<Page>,
}

impl Document {
    pub fn from_markdown(markdown: String) -> Self {
        let regions = parse_regions(&markdown);
        Self {
            markdown,
            pages: vec![Page {
                number: 1,
                width: None,
                height: None,
                regions,
            }],
        }
    }

    pub fn markdown(&self) -> &str {
        &self.markdown
    }

    pub fn render(&self, format: DocumentOutputFormat) -> String {
        match format {
            DocumentOutputFormat::Markdown => self.markdown.clone(),
            DocumentOutputFormat::Html => render_html(&self.markdown),
            DocumentOutputFormat::Text => render_text(&self.markdown),
        }
    }
}

fn heading_level(line: &str) -> Option<u8> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && trimmed.chars().nth(hashes) == Some(' ') {
        Some(hashes as u8)
    } else {
        None
    }
}

fn is_table_separator(line: &str) -> bool {
    let cells: Vec<&str> = line.trim().trim_matches('|').split('|').collect();
    cells.len() >= 2
        && cells.iter().all(|cell| {
            let cell = cell.trim();
            !cell.is_empty()
                && cell.contains('-')
                && cell.chars().all(|c| matches!(c, '-' | ':' | ' '))
        })
}

fn classify_block(lines: &[&str]) -> RegionKind {
    let first = lines.first().copied().unwrap_or("").trim_start();
    if let Some(level) = heading_level(first) {
        return RegionKind::Heading(level);
    }
    if first.starts_with("```") || first.starts_with("~~~") {
        return RegionKind::Code;
    }
    if lines.len() >= 2
        && first.starts_with('|')
        && lines.get(1).is_some_and(|line| is_table_separator(line))
    {
        return RegionKind::Table;
    }
    if first.starts_with('>') {
        return RegionKind::Quote;
    }
    if first.starts_with("![") {
        return RegionKind::Image;
    }
    if first.starts_with("- ")
        || first.starts_with("* ")
        || first.starts_with("+ ")
        || first
            .split_once(['.', ')'])
            .is_some_and(|(prefix, _)| prefix.chars().all(|c| c.is_ascii_digit()))
    {
        return RegionKind::List;
    }
    RegionKind::Paragraph
}

/// 把最终 Markdown 切成有序逻辑区域。这里不推断页面和坐标；未来结构化引擎可直接
/// 构造多页 Document，而不经过这个兼容适配器。
fn parse_regions(markdown: &str) -> Vec<Region> {
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut fence: Option<&str> = None;

    for line in markdown.lines() {
        let trimmed = line.trim_start();
        let marker = if trimmed.starts_with("```") {
            Some("```")
        } else if trimmed.starts_with("~~~") {
            Some("~~~")
        } else {
            None
        };

        if fence.is_some() {
            current.push(line);
            if marker == fence {
                fence = None;
                blocks.push(std::mem::take(&mut current));
            }
            continue;
        }

        if let Some(marker) = marker {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
            current.push(line);
            fence = Some(marker);
            continue;
        }

        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(line);
    }
    if !current.is_empty() {
        blocks.push(current);
    }

    blocks
        .into_iter()
        .enumerate()
        .map(|(index, lines)| {
            let block = lines.join("\n");
            let text = render_text(&block).trim().to_string();
            Region {
                id: format!("p1-r{:04}", index + 1),
                kind: classify_block(&lines),
                bbox: None,
                reading_order: index,
                markdown: block,
                text,
                selected_source: "markscan-final".to_string(),
                confidence: None,
                candidates: Vec::new(),
                verification: VerificationStatus::Unchecked,
            }
        })
        .collect()
}

fn markdown_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
}

fn render_html(markdown: &str) -> String {
    // OCR/LLM 产物是不可信输入；Markdown 中的原始 HTML 必须作为文本转义，避免
    // 生成的 HTML 文件携带 script/onerror 等可执行内容。
    let parser = Parser::new_ext(markdown, markdown_options()).map(|event| match event {
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        other => other,
    });
    let mut body = String::new();
    html::push_html(&mut body, parser);
    format!(
        "<!doctype html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n</head>\n<body>\n<article>\n{body}</article>\n</body>\n</html>\n"
    )
}

fn push_blank_line(out: &mut String) {
    while out.ends_with("\n\n\n") {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with("\n\n") {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
}

fn render_text(markdown: &str) -> String {
    let mut out = String::new();
    for event in Parser::new_ext(markdown, markdown_options()) {
        match event {
            Event::Text(text) | Event::Code(text) => out.push_str(&text),
            Event::SoftBreak | Event::HardBreak => out.push('\n'),
            Event::Rule => {
                push_blank_line(&mut out);
                out.push_str("---");
                push_blank_line(&mut out);
            }
            Event::Start(Tag::Item) => out.push_str("- "),
            Event::Start(Tag::TableCell) if !out.ends_with('\n') && !out.is_empty() => {
                out.push('\t');
            }
            Event::End(TagEnd::Paragraph)
            | Event::End(TagEnd::Heading(_))
            | Event::End(TagEnd::BlockQuote(_))
            | Event::End(TagEnd::CodeBlock)
            | Event::End(TagEnd::List(_)) => push_blank_line(&mut out),
            Event::End(TagEnd::Item) | Event::End(TagEnd::TableRow) => {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Event::End(TagEnd::TableCell) => {}
            _ => {}
        }
    }
    let trimmed = out.trim_end();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# 标题\n\n正文含有 **重点** 和 [链接](https://example.com)。\n\n| 名称 | 数值 |\n|---|---:|\n| 甲 | 10 |\n\n- 条目一\n- 条目二\n";

    #[test]
    fn markdown_is_lossless() {
        let doc = Document::from_markdown(SAMPLE.to_string());
        assert_eq!(doc.render(DocumentOutputFormat::Markdown), SAMPLE);
        assert_eq!(doc.pages.len(), 1);
        assert_eq!(doc.pages[0].regions.len(), 4);
        assert_eq!(doc.pages[0].regions[0].kind, RegionKind::Heading(1));
        assert_eq!(doc.pages[0].regions[2].kind, RegionKind::Table);
        assert_eq!(doc.pages[0].regions[3].kind, RegionKind::List);
        assert!(doc.pages[0].regions.iter().all(|r| r.bbox.is_none()));
    }

    #[test]
    fn html_preserves_document_structures() {
        let out = Document::from_markdown(SAMPLE.to_string()).render(DocumentOutputFormat::Html);
        assert!(out.starts_with("<!doctype html>"));
        assert!(out.contains("<h1>标题</h1>"));
        assert!(out.contains("<strong>重点</strong>"));
        assert!(out.contains("<table>"));
        assert!(out.contains("<li>条目一</li>"));
    }

    #[test]
    fn text_removes_markup_but_keeps_content() {
        let out = Document::from_markdown(SAMPLE.to_string()).render(DocumentOutputFormat::Text);
        assert!(out.contains("标题"));
        assert!(out.contains("正文含有 重点 和 链接。"));
        assert!(out.contains("名称\t数值"));
        assert!(!out.contains("**"));
        assert!(!out.contains("https://example.com"));
    }

    #[test]
    fn html_escapes_untrusted_raw_html() {
        let input = "正文\n\n<script>alert('x')</script>\n";
        let out = Document::from_markdown(input.to_string()).render(DocumentOutputFormat::Html);
        assert!(!out.contains("<script>"));
        assert!(out.contains("&lt;script&gt;"));
    }
}
