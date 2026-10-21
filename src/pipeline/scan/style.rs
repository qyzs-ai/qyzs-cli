//! 排版与样式信息提取 (合同/公文/标准规范的层级与强调判定)
//!
//! 为什么需要它: 实测证明体裁层级常与字号无关 —— 生态环境法典全篇 9.0pt 单一
//! 字号、国标正文与标题同为 9.7pt, 层级只存在于编号文本; 但另一类文档 (如
//! 「个人信息出境标准合同办法」) 标题 27pt、正文 14.1pt, 字号是唯一可靠信号。
//! 所以样式是**补充信号**, 与编号规则融合使用, 不单独承担定级职责。
//!
//! 两条来源:
//! - **DOCX**: 样式是权威的。`word/styles.xml` 定义 styleId → 名称 (Heading 1 /
//!   标题 1), 段落 `w:pStyle` 直接引用, 无需猜测。
//! - **PDF**: 解析页面内容流, 跟踪 `Tf` (字体/字号) 与 `Tj/TJ` (文本), 从字体
//!   描述符判定粗体。字号与字重即使文字编码损坏 (非标准 CMap) 依然有效 ——
//!   这是几何/样式信息相对文本编码的天然优势。
//!
//! 明确不做: 把任意粗体 run 转成 `**粗体**`。实测文档里粗体 run 动辄上千
//! (需求规格说明书 2652 个), 盲目转义会把正文打满星号, 破坏可读性。仅在
//! 短标题行上使用样式信息定级。

use crate::error::{QyzsError, QyzsResult};
use std::collections::HashMap;

/// 一行的样式摘要
#[derive(Debug, Clone)]
pub struct StyledLine {
    /// 字号 (pt)
    pub size: f32,
    /// 是否粗体 (字体名或描述符 Flags 判定)
    pub bold: bool,
    /// 行文本
    pub text: String,
}

/// 字号倍数阈值: 超过正文此倍数视为「大字号」(标题候选)
const LARGE_FONT_RATIO: f32 = 1.15;

/// 短行阈值 (字符数): 标题一般短, 超过则更像正文
const SHORT_LINE_CHARS: usize = 40;

/// 从一组行样式里估计正文字号 (出现次数最多的字号)
pub fn body_size(lines: &[StyledLine]) -> f32 {
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for l in lines {
        // 以 0.1pt 量化, 避免浮点抖动
        *counts.entry((l.size * 10.0).round() as u32).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(s, _)| s as f32 / 10.0)
        .unwrap_or(0.0)
}

/// 判定一行是否为「大字号标题候选」
pub fn is_large_heading(line: &StyledLine, body: f32) -> bool {
    if body <= 0.0 {
        return false;
    }
    line.size >= body * LARGE_FONT_RATIO
        && line.text.chars().count() <= SHORT_LINE_CHARS
        && !line.text.trim().is_empty()
}

// ── DOCX: 权威样式 → 标题层级 ──────────────────────────────────────────

/// DOCX 的标题段落 (文本 + 层级)
#[derive(Debug, Clone)]
pub struct DocxHeading {
    pub text: String,
    pub level: usize,
}

/// 解析 DOCX 的标题结构 (依据 word/styles.xml + 段落 w:pStyle)
///
/// 中文文档常用数字 styleId (如 "18"/"4"/"5"), 必须经 styles.xml 解析出真实
/// 名称才能判断是否为标题; 直接看 styleId 数字会误判。
pub fn docx_headings(bytes: &[u8]) -> QyzsResult<Vec<DocxHeading>> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(cursor))
        .map_err(|e| QyzsError::Usage(format!("无法解包 DOCX: {}", e)))?;

    let styles_xml = read_entry(&mut archive, "word/styles.xml").unwrap_or_default();
    let document_xml = read_entry(&mut archive, "word/document.xml").unwrap_or_default();
    if document_xml.is_empty() {
        return Ok(Vec::new());
    }

    // styleId → 标题层级 (1..=6)
    let level_by_style = style_levels(&styles_xml);

    let mut out = Vec::new();
    // 按段落切分: 取每个 <w:p>...</w:p> 的 pStyle 与文本
    for para in split_paragraphs(&document_xml) {
        let Some(style_id) = extract_attr(para, "w:pStyle") else {
            continue;
        };
        let Some(&level) = level_by_style.get(&style_id) else {
            continue;
        };
        let text = collect_text_nodes(para).trim().to_string();
        if text.is_empty() {
            continue;
        }
        out.push(DocxHeading { text, level });
    }
    Ok(out)
}

fn read_entry(
    archive: &mut zip::ZipArchive<std::io::BufReader<std::io::Cursor<&[u8]>>>,
    name: &str,
) -> Option<String> {
    let mut f = archive.by_name(name).ok()?;
    let mut s = String::new();
    std::io::Read::read_to_string(&mut f, &mut s).ok()?;
    Some(s)
}

/// 从 styles.xml 建立 styleId → 标题层级 映射
///
/// 识别三种常见命名: `Heading N` / `标题 N` / `heading N`, 并读取 `w:outlineLvl`
/// (0-based) 作为兜底。
fn style_levels(styles_xml: &str) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    let mut rest = styles_xml;
    while let Some(start) = rest.find("<w:style ") {
        let Some(end_rel) = rest[start..].find("</w:style>") else {
            break;
        };
        let block = &rest[start..start + end_rel];
        let style_id = extract_direct_attr(block, "w:styleId").unwrap_or_default();
        let name = extract_attr(block, "w:name").unwrap_or_default();
        if !style_id.is_empty() {
            if let Some(level) = heading_level_from_name(&name) {
                map.insert(style_id.clone(), level);
            } else if let Some(level) = extract_outline_level(block) {
                map.insert(style_id, level);
            }
        }
        rest = &rest[start + end_rel + "</w:style>".len()..];
    }
    map
}

/// 从样式名解析标题层级
fn heading_level_from_name(name: &str) -> Option<usize> {
    let lower = name.to_ascii_lowercase();
    let digits: String = name
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    let is_heading = lower.contains("heading") || name.contains("标题");
    if !is_heading || digits.is_empty() {
        return None;
    }
    let level: usize = digits.parse().ok()?;
    if (1..=6).contains(&level) {
        Some(level)
    } else {
        None
    }
}

/// `w:outlineLvl w:val="0"` → 层级 1
fn extract_outline_level(block: &str) -> Option<usize> {
    let idx = block.find("w:outlineLvl")?;
    let tail = &block[idx..];
    let val_idx = tail.find("w:val=\"")? + "w:val=\"".len();
    let rest = &tail[val_idx..];
    let end = rest.find('"')?;
    let n: usize = rest[..end].parse().ok()?;
    Some(n.min(5) + 1)
}

/// 取形如 `w:styleId="4"` 的直接属性值 (值写在属性名后, 不套 w:val)
fn extract_direct_attr(block: &str, attr: &str) -> Option<String> {
    let idx = block.find(attr)?;
    let tail = &block[idx + attr.len()..];
    let start = tail.find("=\"")? + 2;
    let rest = &tail[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// 取形如 `w:pStyle w:val="Heading1"` 的属性值
fn extract_attr<'a>(block: &'a str, attr: &str) -> Option<String> {
    let idx = block.find(attr)?;
    let tail = &block[idx + attr.len()..];
    let val_idx = tail.find("w:val=\"")? + "w:val=\"".len();
    let rest = &tail[val_idx..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// 按 `<w:p>` 切分段落 (返回含起止标签的片段)
fn split_paragraphs(xml: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<w:p>").or_else(|| rest.find("<w:p ")) {
        let Some(end_rel) = rest[start..].find("</w:p>") else {
            break;
        };
        let end = start + end_rel + "</w:p>".len();
        out.push(&rest[start..end]);
        rest = &rest[end..];
    }
    out
}

/// 收集 `<w:t>` 文本节点
fn collect_text_nodes(block: &str) -> String {
    let mut out = String::new();
    let mut rest = block;
    while let Some(start) = rest.find("<w:t") {
        let Some(open_end_rel) = rest[start..].find('>') else {
            break;
        };
        let content_start = start + open_end_rel + 1;
        let Some(close_rel) = rest[content_start..].find("</w:t>") else {
            break;
        };
        let text = &rest[content_start..content_start + close_rel];
        out.push_str(&unescape_xml(text));
        rest = &rest[content_start + close_rel + "</w:t>".len()..];
    }
    out
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

// ── PDF: 内容流样式提取 ────────────────────────────────────────────────

/// 从 PDF 提取行级样式 (字号/粗体/文本)
///
/// 只用于补充判定: 文字编码损坏时 decode 会失败, 该片段被跳过, 不影响其它页。
pub fn pdf_lines(path: &str) -> QyzsResult<Vec<StyledLine>> {
    use lopdf::Document;

    let doc = Document::load(path)
        .map_err(|e| QyzsError::Pdf(format!("加载 PDF 失败: {}", e)))?;

    let mut out = Vec::new();
    for (_, page_id) in doc.get_pages() {
        // 字体名 → (编码, 是否粗体)
        let mut font_meta: HashMap<Vec<u8>, (Option<lopdf::Encoding>, bool)> = HashMap::new();
        if let Ok(fonts) = doc.get_page_fonts(page_id) {
            for (name, dict) in fonts {
                let enc = dict.get_font_encoding(&doc).ok();
                let bold = font_is_bold(dict, &doc);
                font_meta.insert(name, (enc, bold));
            }
        }

        let content_data = doc.get_page_content(page_id);
        if content_data.is_empty() {
            continue;
        }
        let Ok(content) = lopdf::content::Content::decode(&content_data) else {
            continue;
        };

        let mut cur_size: f32 = 0.0;
        let mut cur_bold = false;
        let mut cur_font: Option<Vec<u8>> = None;
        let mut buf = String::new();

        for op in &content.operations {
            match op.operator.as_ref() {
                "Tf" => {
                    // 换字体前把已累积文本收成一行
                    flush_line(&mut buf, &mut out, cur_size, cur_bold);
                    let name = op
                        .operands
                        .first()
                        .and_then(|o| o.as_name().ok().map(|n| n.to_vec()));
                    let size: Option<f64> = op
                        .operands
                        .get(1)
                        .and_then(|o| o.as_float().ok().map(f64::from))
                        .or_else(|| {
                            op.operands
                                .get(1)
                                .and_then(|o| o.as_i64().ok().map(|i| i as f64))
                        });
                    if let Some(s) = size {
                        cur_size = s as f32;
                    }
                    if let Some(name) = name {
                        if let Some((_, bold)) = font_meta.get(&name) {
                            cur_bold = *bold;
                        }
                        cur_font = Some(name);
                    }
                }
                "Tj" | "TJ" => {
                    if let Some(enc) = cur_font
                        .as_ref()
                        .and_then(|n| font_meta.get(n))
                        .and_then(|(e, _)| e.as_ref())
                    {
                        append_text(&mut buf, enc, &op.operands);
                    }
                }
                // ' 与 " 隐含换行语义
                "'" | "\"" => {
                    flush_line(&mut buf, &mut out, cur_size, cur_bold);
                    if let Some(enc) = cur_font
                        .as_ref()
                        .and_then(|n| font_meta.get(n))
                        .and_then(|(e, _)| e.as_ref())
                    {
                        let operands = if op.operator == "\"" {
                            op.operands.get(2).cloned().into_iter().collect::<Vec<_>>()
                        } else {
                            op.operands.clone()
                        };
                        append_text(&mut buf, enc, &operands);
                    }
                }
                // 文本定位操作: 视为行边界
                "Td" | "TD" | "Tm" | "T*" => {
                    flush_line(&mut buf, &mut out, cur_size, cur_bold);
                }
                _ => {}
            }
        }
        flush_line(&mut buf, &mut out, cur_size, cur_bold);
    }
    Ok(out)
}

fn flush_line(buf: &mut String, out: &mut Vec<StyledLine>, size: f32, bold: bool) {
    let text = buf.trim().to_string();
    buf.clear();
    if text.is_empty() {
        return;
    }
    out.push(StyledLine { size, bold, text });
}

fn append_text(buf: &mut String, enc: &lopdf::Encoding<'_>, operands: &[lopdf::Object]) {
    for operand in operands {
        match operand {
            lopdf::Object::String(bytes, _) => {
                if let Ok(s) = lopdf::Document::decode_text(enc, bytes) {
                    buf.push_str(&s);
                }
            }
            lopdf::Object::Array(items) => append_text(buf, enc, items),
            _ => {}
        }
    }
}

/// 判定字体是否粗体: 名称含 Bold/Black/Hei, 或描述符 Flags 的 ForceBold 位
fn font_is_bold(dict: &lopdf::Dictionary, doc: &lopdf::Document) -> bool {
    if let Ok(base) = dict.get(b"BaseFont").and_then(|o| o.as_name()) {
        let name = String::from_utf8_lossy(base).to_ascii_lowercase();
        if name.contains("bold") || name.contains("black") || name.contains("hei") {
            return true;
        }
    }
    // FontDescriptor /Flags 第 19 位 (ForceBold) = 1 << 18
    if let Ok(desc) = dict
        .get(b"FontDescriptor")
        .and_then(|o| o.as_reference())
        .and_then(|id| doc.get_dictionary(id))
    {
        if let Ok(flags) = desc.get(b"Flags").and_then(|o| o.as_i64()) {
            return flags & (1 << 18) != 0;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_body_size_majority() {
        let lines = vec![
            StyledLine { size: 9.0, bold: false, text: "正文".into() },
            StyledLine { size: 9.0, bold: false, text: "正文".into() },
            StyledLine { size: 26.0, bold: true, text: "标题".into() },
        ];
        assert_eq!(body_size(&lines), 9.0);
    }

    #[test]
    fn test_large_heading_detection() {
        let body = 14.1;
        let title = StyledLine { size: 27.0, bold: false, text: "个人信息出境标准合同办法".into() };
        assert!(is_large_heading(&title, body));
        // 同字号正文不算标题
        let para = StyledLine { size: 14.1, bold: false, text: "第一条 为了规范".into() };
        assert!(!is_large_heading(&para, body));
        // 长行不算标题
        let long = StyledLine { size: 27.0, bold: false, text: "长".repeat(80) };
        assert!(!is_large_heading(&long, body));
    }

    #[test]
    fn test_heading_level_from_name() {
        assert_eq!(heading_level_from_name("Heading 1"), Some(1));
        assert_eq!(heading_level_from_name("heading2"), Some(2));
        assert_eq!(heading_level_from_name("标题 3"), Some(3));
        assert_eq!(heading_level_from_name("Normal"), None);
        assert_eq!(heading_level_from_name("Heading 9"), None);
    }

    #[test]
    fn test_style_levels_with_numeric_ids() {
        // 中文文档常见: styleId 是数字, 名称才是「标题 1」
        let styles = r#"<w:styles>
            <w:style w:type="paragraph" w:styleId="4"><w:name w:val="标题 1"/></w:style>
            <w:style w:type="paragraph" w:styleId="5"><w:name w:val="标题 2"/></w:style>
            <w:style w:type="paragraph" w:styleId="18"><w:name w:val="正文"/></w:style>
        </w:styles>"#;
        let map = style_levels(styles);
        assert_eq!(map.get("4"), Some(&1));
        assert_eq!(map.get("5"), Some(&2));
        assert_eq!(map.get("18"), None, "正文字体不应被视为标题");
    }

    #[test]
    fn test_outline_level_fallback() {
        let styles = r#"<w:style w:styleId="X"><w:name w:val="Mystery"/>
            <w:pPr><w:outlineLvl w:val="2"/></w:pPr></w:style>"#;
        let map = style_levels(styles);
        assert_eq!(map.get("X"), Some(&3), "outlineLvl 0-based → 层级 3");
    }

    #[test]
    fn test_collect_text_nodes_with_entities() {
        let p = r#"<w:p><w:r><w:t>甲方 &amp; 乙方</w:t></w:r></w:p>"#;
        assert_eq!(collect_text_nodes(p), "甲方 & 乙方");
    }

    #[test]
    fn test_docx_headings_real_file() {
        let path = "tests/data/mineru/public_doc/REF-0106_项目验收前符合性审查表.docx";
        let Ok(bytes) = std::fs::read(path) else {
            return; // 样本缺失时跳过 (不误报)
        };
        let heads = docx_headings(&bytes).expect("解析应成功");
        // 该样本样式简单, 不强求有标题; 只要求不崩溃且层级合法
        for h in &heads {
            assert!((1..=6).contains(&h.level));
            assert!(!h.text.is_empty());
        }
    }

    #[test]
    fn test_pdf_lines_real_file() {
        let path = "tests/data/mineru/text_pdf/REF-0056_政府采购评审专家管理办法.pdf";
        let lines = pdf_lines(path).expect("提取应成功");
        assert!(!lines.is_empty(), "文本层 PDF 应提取到行样式");
        assert!(lines.iter().any(|l| l.size > 0.0), "应有有效字号");
    }
}
