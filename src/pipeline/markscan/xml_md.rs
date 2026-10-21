//! markscan XML → Markdown 通道 (quick-xml, 纯 Rust)
//!
//! 任意 XML (不仅 XHTML): SOAP 响应、配置文件 (pom.xml / settings.xml)、
//! 行业数据交换格式 (国土空间类 GB/T 39786 等)。策略:
//! - XML 声明与处理指令 (PI) 跳过
//! - 元素逐层缩进, 文本节点做去空白折叠
//! - 属性合并到元素名后: `<book id="1" lang="en">标题</book>` → `- **book** (id=1, lang=en): 标题`
//! - quick-xml 0.42 将实体引用拆为独立 GeneralRef 事件; 常规 5 实体解码, 字符引用支持
//! - 字节先尝试 UTF-8 (含 BOM), 失败回退 GB18030

use super::workspace::Workspace;
use crate::error::{QyzsError, QyzsResult};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use std::io::Cursor;

/// 字节 → Markdown
pub fn xml_to_markdown(bytes: &[u8]) -> QyzsResult<String> {
    let (text, encoding) = decode_xml_bytes(bytes);

    let mut reader = Reader::from_reader(Cursor::new(text.into_bytes()));
    reader.config_mut().trim_text(true);

    let mut out = String::new();
    let mut has_element_output = false;
    let mut buf = Vec::new();
    let mut state = WalkState::default();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                state.flush_pending(&mut out, &mut has_element_output);
                let name = local_name(e.name().as_ref());
                let attrs = format_attrs(&e);
                state.handle_start(name, attrs);
            }
            Ok(Event::End(_)) => {
                state.flush_pending(&mut out, &mut has_element_output);
                state.handle_end();
            }
            Ok(Event::Text(t)) => {
                state.append_text(&t.xml_content(quick_xml::XmlVersion::Implicit1_0));
            }
            Ok(Event::CData(c)) => {
                state.append_text(c.as_ref());
            }
            Ok(Event::GeneralRef(r)) => {
                state.append_text(&decode_general_ref(r.as_ref()));
            }
            Ok(Event::Empty(e)) => {
                state.flush_pending(&mut out, &mut has_element_output);
                let name = local_name(e.name().as_ref());
                let attrs = format_attrs(&e);
                state.handle_empty(name, attrs, &mut out, &mut has_element_output);
            }
            Ok(Event::Eof) => {
                state.flush_pending(&mut out, &mut has_element_output);
                break;
            }
            Ok(_) => {} // Comment / Decl / PI / DocType → 跳过
            Err(e) => {
                return Err(QyzsError::Server(format!(
                    "XML 解析失败 (offset {}): {}",
                    reader.buffer_position(),
                    e
                )))
            }
        }
        buf.clear();
    }

    if !has_element_output {
        return Err(QyzsError::Server(
            "XML 转换结果为空 (文档可能没有元素内容)".to_string(),
        ));
    }

    // 元素内容已产生, 此时再补编码注释
    if !encoding.is_empty() {
        let prefix = format!("<!-- 源文件编码: {} -->\n\n", encoding);
        out = format!("{}{}", prefix, out);
    }
    Ok(out.trim_end().to_string())
}

#[derive(Default)]
struct WalkState {
    /// 当前未闭合的元素名栈
    ancestors: Vec<String>,
    /// 当前未闭合元素的属性 (从最近的 Start 取)
    current_attrs: Vec<String>,
    /// 当前元素累积的文本内容 (跨 Text/CData/GeneralRef 累加)
    pending: String,
}

impl WalkState {
    fn handle_start(&mut self, name: String, attrs: Vec<String>) {
        self.ancestors.push(name);
        self.current_attrs = attrs;
        self.pending.clear();
    }

    fn handle_end(&mut self) {
        self.ancestors.pop();
        self.current_attrs.clear();
        self.pending.clear();
    }

    fn append_text(&mut self, s: &str) {
        self.pending.push_str(s);
    }

    /// 把累积的文本输出为 `- **elem** (attrs): text` 格式
    fn flush_pending(&mut self, out: &mut String, has_output: &mut bool) {
        let trimmed = self.pending.trim();
        if trimmed.is_empty() {
            self.pending.clear();
            return;
        }
        if let Some(top) = self.ancestors.last() {
            let prefix = if self.current_attrs.is_empty() {
                format!("- **{}**:", top)
            } else {
                format!("- **{}** ({}):", top, self.current_attrs.join(", "))
            };
            out.push_str(&format!("{} {}\n", prefix, trimmed));
        } else {
            out.push_str(trimmed);
            out.push('\n');
        }
        *has_output = true;
        self.pending.clear();
    }

    fn handle_empty(
        &mut self,
        name: String,
        attrs: Vec<String>,
        out: &mut String,
        has_output: &mut bool,
    ) {
        let line = match attrs.len() {
            0 => format!("- **{}** _(empty)_\n", name),
            _ => format!("- **{}** ({})\n", name, attrs.join(", ")),
        };
        out.push_str(&line);
        *has_output = true;
    }
}

/// 解码常规 XML 实体引用; 字符引用 (&#x4E2D; / &#20013;) 与未知实体原样保留
fn decode_general_ref(name: &str) -> String {
    match name {
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" => "'".to_string(),
        // 数字字符引用: &#x4E2D; / &#20013;
        n if n.starts_with('#') => decode_char_ref(&n[1..]).unwrap_or_default(),
        // 未知自定义实体 → 保留原引用 (OOXML 中极少, 配置文件类可能见到)
        other => format!("&{};", other),
    }
}

fn decode_char_ref(body: &str) -> Option<String> {
    if let Some(hex) = body.strip_prefix('x').or_else(|| body.strip_prefix('X')) {
        u32::from_str_radix(hex, 16)
            .ok()
            .and_then(char::from_u32)
            .map(|c| c.to_string())
    } else {
        body.parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(|c| c.to_string())
    }
}

fn local_name(qname: &str) -> String {
    // 去掉命名空间前缀 (ns:local → local)
    match qname.find(':') {
        Some(pos) => qname[pos + 1..].to_string(),
        None => qname.to_string(),
    }
}

fn format_attrs(e: &BytesStart) -> Vec<String> {
    e.attributes()
        .with_checks(false)
        .flatten()
        .map(|a| {
            let key = local_name(a.key.as_ref());
            // 0.42 推荐 normalized_value (旧 unescape_value 已弃用)
            let val = a
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map(|s| s.into_owned())
                .unwrap_or_default();
            format!("{}={}", key, val)
        })
        .collect()
}

/// 字节 → (文本, 编码标签)
fn decode_xml_bytes(bytes: &[u8]) -> (String, &'static str) {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return (
            String::from_utf8_lossy(&bytes[3..]).into_owned(),
            "UTF-8 (BOM)",
        );
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), "UTF-8"),
        Err(_) => {
            let (cow, _, had_errors) = encoding_rs::GB18030.decode(bytes);
            if had_errors {
                (cow.into_owned(), "GB18030 (有损)")
            } else {
                (cow.into_owned(), "GB18030")
            }
        }
    }
}

pub fn extract_xml(ws: &mut Workspace, path: &str) -> QyzsResult<String> {
    if ws.stage_done("document", "extract") {
        return ws.read_segment_text("document");
    }
    let bytes = std::fs::read(path)
        .map_err(|e| QyzsError::Io(format!("读取 XML 文件失败 {}: {}", path, e)))?;
    let output = xml_to_markdown(&bytes)?;
    ws.write_segment_text("document", &output)?;
    ws.complete_stage("document", "extract", &[])?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_element() {
        let xml = r#"<?xml version="1.0"?>
            <root>
                <title>测试标题</title>
                <author>张三</author>
            </root>"#;
        let md = xml_to_markdown(xml.as_bytes()).unwrap();
        assert!(md.contains("**title**: 测试标题"));
        assert!(md.contains("**author**: 张三"));
    }

    #[test]
    fn test_attributes() {
        let xml = r#"<book id="1" lang="en">Hello World</book>"#;
        let md = xml_to_markdown(xml.as_bytes()).unwrap();
        assert!(md.contains("**book**"));
        assert!(md.contains("id=1"));
        assert!(md.contains("lang=en"));
        assert!(md.contains("Hello World"));
    }

    #[test]
    fn test_empty_element() {
        let xml = r#"<config><debug/></config>"#;
        let md = xml_to_markdown(xml.as_bytes()).unwrap();
        assert!(md.contains("**debug**"));
        assert!(md.contains("empty"));
    }

    #[test]
    fn test_namespaced_element() {
        let xml = r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
            <soap:Body>
                <Response>OK</Response>
            </soap:Body>
        </soap:Envelope>"#;
        let md = xml_to_markdown(xml.as_bytes()).unwrap();
        assert!(md.contains("**Response**: OK"));
    }

    #[test]
    fn test_cdata_preserved() {
        let xml = r#"<doc><code><![CDATA[<html>raw</html>]]></code></doc>"#;
        let md = xml_to_markdown(xml.as_bytes()).unwrap();
        assert!(md.contains("<html>raw</html>"));
    }

    #[test]
    fn test_xml_declaration_stripped() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
            <root>value</root>"#;
        let md = xml_to_markdown(xml.as_bytes()).unwrap();
        assert!(!md.contains("<?xml"));
        assert!(md.contains("**root**: value"));
    }

    #[test]
    fn test_processing_instruction_skipped() {
        let xml = r#"<?xml-stylesheet type="text/xsl" href="style.xsl"?>
            <root>text</root>"#;
        let md = xml_to_markdown(xml.as_bytes()).unwrap();
        assert!(!md.contains("xml-stylesheet"));
        assert!(md.contains("**root**: text"));
    }

    #[test]
    fn test_malformed_xml_errors() {
        let xml = r#"<unclosed>"#;
        let err = xml_to_markdown(xml.as_bytes()).unwrap_err();
        assert!(matches!(err, QyzsError::Server(_)));
    }

    #[test]
    fn test_empty_doc_errors() {
        let xml = r#"<?xml version="1.0"?>"#;
        let err = xml_to_markdown(xml.as_bytes()).unwrap_err();
        assert!(matches!(err, QyzsError::Server(_)));
    }

    #[test]
    fn test_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"<root>v</root>");
        let md = xml_to_markdown(&bytes).unwrap();
        assert!(md.contains("UTF-8 (BOM)"));
        assert!(md.contains("**root**: v"));
    }

    #[test]
    fn test_gbk_decoding() {
        // "测试" 的 GBK 编码
        let xml = b"<root>\xB2\xE2\xCA\xD4</root>";
        let md = xml_to_markdown(xml).unwrap();
        assert!(md.contains("GB18030"));
        assert!(md.contains("**root**: 测试"));
    }
}
