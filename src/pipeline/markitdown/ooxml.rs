//! markitdown OOXML 文本抽取通道 (pptx / vsdx → Markdown, 纯 Rust 零 C 依赖)
//!
//! zip + quick-xml 部件抽取:
//! - pptx: 幻灯片按编号序输出, `<a:p>` 段落聚合 `<a:t>` 文本运行;
//! - vsdx: 页面经 pages.xml 取名称, 形状 `<Text>` 逐个输出。
//!
//! 布局/样式信息 (坐标、颜色、连接线拓扑) 不在抽取范围 — Markdown 为线性文本。

use super::workspace::Workspace;
use crate::error::{QyzsError, QyzsResult};
use quick_xml::events::Event;
use quick_xml::XmlVersion;
use quick_xml::Reader;
use std::io::{BufReader, Read as _};

/// 提取元素文本 (含嵌套, 仅取文本节点)
fn element_inner_text(xml: &str, tag_local: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut captures: Vec<String> = Vec::new();
    let mut depth = 0usize;
    let mut buf = String::new();
    let mut active = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name().local_name().as_ref().to_string();
                if name == tag_local {
                    if active {
                        depth += 1;
                    } else {
                        active = true;
                        depth = 1;
                        buf.clear();
                    }
                } else if active {
                    depth += 1;
                }
            }
            Ok(Event::Empty(_)) => {}
            Ok(Event::End(e)) => {
                let name = e.name().local_name().as_ref().to_string();
                if active {
                    if name == tag_local {
                        depth -= 1;
                        if depth == 0 {
                            captures.push(buf.clone());
                            active = false;
                        }
                    } else {
                        depth = depth.saturating_sub(1);
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if active {
                    buf.push_str(&t.xml_content(XmlVersion::Implicit1_0));
                }
            }
            Ok(Event::CData(t)) => {
                if active {
                    buf.push_str(t.as_ref());
                }
            }
            // quick-xml 0.42 将实体引用拆为独立事件 (&amp; 等): 常规实体解码,
            // 字符引用解析, 未知自定义实体忽略 (OOXML 无 DTD)
            Ok(Event::GeneralRef(r)) => {
                if active {
                    let name = r.as_ref();
                    match name {
                        "amp" => buf.push('&'),
                        "lt" => buf.push('<'),
                        "gt" => buf.push('>'),
                        "quot" => buf.push('"'),
                        "apos" => buf.push('\''),
                        _ => {
                            if let Ok(Some(c)) = r.resolve_char_ref() {
                                buf.push(c);
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    captures
}

// ── PPTX ────────────────────────────────────────────────────────────────

/// 幻灯片部件名数值排序 (slide1, slide2, ..., slide10)
fn numeric_sort_key(name: &str) -> u64 {
    let digits: String = name.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse().unwrap_or(0)
}

/// pptx → Markdown: 每页 "## 幻灯片 N" + 段落文本
pub fn extract_pptx_text(bytes: &[u8]) -> QyzsResult<String> {
    let mut archive = open_zip(bytes)?;
    let mut slide_files: Vec<String> = archive
        .file_names()
        .flatten()
        .filter(|n| {
            let lower = n.to_ascii_lowercase();
            lower.starts_with("ppt/slides/slide") && lower.ends_with(".xml")
        })
        .map(|n| n.into_owned())
        .collect();
    slide_files.sort_by_key(|n| numeric_sort_key(n));

    if slide_files.is_empty() {
        return Err(QyzsError::Server(
            "PPTX 中未找到幻灯片部件 (ppt/slides/slide*.xml)".to_string(),
        ));
    }

    let mut out = String::new();
    for (idx, slide_name) in slide_files.iter().enumerate() {
        let xml = read_zip_entry(&mut archive, slide_name)?;
        let xml = String::from_utf8_lossy(&xml);
        let xml = xml.as_ref();

        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&format!("## 幻灯片 {}\n\n", idx + 1));

        // 每个段落 <a:p> 聚合为一个输出行
        let paragraphs = element_inner_text(xml, "p");
        let mut wrote = 0;
        for p in paragraphs {
            let line = p.trim();
            if line.is_empty() {
                continue;
            }
            out.push_str(line);
            out.push('\n');
            wrote += 1;
        }
        if wrote == 0 {
            out.push_str("_(本页无文本)_\n");
        }
    }
    Ok(out)
}

// ── VSDX ────────────────────────────────────────────────────────────────

/// vsdx → Markdown: 页面名称 + 形状文本
pub fn extract_vsdx_text(bytes: &[u8]) -> QyzsResult<String> {
    let mut archive = open_zip(bytes)?;

    // 页面清单: visio/pages/pages.xml 的 <Page Name="..." ID="n">
    let mut pages: Vec<(u64, String)> = Vec::new();
    if let Ok(xml_bytes) = read_zip_entry(&mut archive, "visio/pages/pages.xml") {
        let xml = String::from_utf8_lossy(&xml_bytes).into_owned();
        pages = parse_page_list(&xml);
    }

    if pages.is_empty() {
        // 兜底: 无清单时按 pageN.xml 编号序
        let mut page_files: Vec<String> = archive
            .file_names()
            .flatten()
            .filter(|n| {
                let lower = n.to_ascii_lowercase();
                lower.starts_with("visio/pages/page") && lower.ends_with(".xml") && !lower.ends_with("pages.xml")
            })
            .map(|n| n.into_owned())
            .collect();
        page_files.sort_by_key(|n| numeric_sort_key(n));
        for (i, f) in page_files.iter().enumerate() {
            pages.push((i as u64, f.clone()));
        }
    }

    if pages.is_empty() {
        return Err(QyzsError::Server(
            "VSDX 中未找到页面部件 (visio/pages/*.xml)".to_string(),
        ));
    }

    let mut out = String::new();
    for (idx, (page_id, name)) in pages.iter().enumerate() {
        let part = format!("visio/pages/page{}.xml", page_id + 1);
        let xml_bytes = match read_zip_entry(&mut archive, &part) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let xml = String::from_utf8_lossy(&xml_bytes).into_owned();

        if idx > 0 {
            out.push('\n');
        }
        let display_name = if name.starts_with("visio/pages/") {
            numeric_sort_key(name).to_string()
        } else {
            name.clone()
        };
        out.push_str(&format!("## 页面: {}\n\n", display_name));

        let texts = element_inner_text(&xml, "Text");
        let mut wrote = 0;
        for t in texts {
            let cleaned = t.split_whitespace().collect::<Vec<_>>().join(" ");
            if cleaned.is_empty() {
                continue;
            }
            out.push_str(&format!("- {}\n", cleaned));
            wrote += 1;
        }
        if wrote == 0 {
            out.push_str("_(本页无形状文本)_\n");
        }
    }
    Ok(out)
}

/// 解析 pages.xml: 返回 (页面 ID, 名称) 序列
fn parse_page_list(pages_xml: &str) -> Vec<(u64, String)> {
    let mut reader = Reader::from_str(pages_xml);
    reader.config_mut().trim_text(false);
    let mut pages = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if e.name().local_name().as_ref() == "Page" {
                    let mut id: Option<u64> = None;
                    let mut name: Option<String> = None;
                    for attr in e.attributes().flatten() {
                        match attr.key.local_name().as_ref() {
                            "ID" => {
                                id = attr.value.as_ref().parse().ok();
                            }
                            "Name" => {
                                name = attr.normalized_value(XmlVersion::Implicit1_0).ok().map(|v| v.into_owned());
                            }
                            _ => {}
                        }
                    }
                    if let (Some(i), Some(n)) = (id, name) {
                        pages.push((i, n));
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    let _ = &mut buf;
    pages.sort_by_key(|(id, _)| *id);
    pages
}

// ── 通道封装 ─────────────────────────────────────────────────────────────

pub fn extract_pptx(ws: &mut Workspace, path: &str) -> QyzsResult<String> {
    if ws.stage_done("document", "extract") {
        return ws.read_segment_text("document");
    }
    let bytes = std::fs::read(path)
        .map_err(|e| QyzsError::Io(format!("读取文件失败 {}: {}", path, e)))?;
    let output = extract_pptx_text(&bytes)?;
    ws.write_segment_text("document", &output)?;
    ws.complete_stage("document", "extract", &[])?;
    Ok(output)
}

pub fn extract_vsdx(ws: &mut Workspace, path: &str) -> QyzsResult<String> {
    if ws.stage_done("document", "extract") {
        return ws.read_segment_text("document");
    }
    let bytes = std::fs::read(path)
        .map_err(|e| QyzsError::Io(format!("读取文件失败 {}: {}", path, e)))?;
    let output = extract_vsdx_text(&bytes)?;
    ws.write_segment_text("document", &output)?;
    ws.complete_stage("document", "extract", &[])?;
    Ok(output)
}

/// docx 本地文本抽取 (rigor 档第二通道): word/document.xml 按 <w:p> 聚合 <w:t>
pub fn extract_docx_text(bytes: &[u8]) -> QyzsResult<String> {
    let mut archive = open_zip(bytes)?;
    let xml_bytes = read_zip_entry(&mut archive, "word/document.xml")
        .map_err(|e| QyzsError::Server(format!("docx 缺少主文档部件: {}", e)))?;
    let xml = String::from_utf8_lossy(&xml_bytes).into_owned();

    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut w_depth = 0usize;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                if e.name().local_name().as_ref() == "t" && w_depth > 0 {
                    // w:t 文本由 Text 事件携带
                }
                w_depth += 1;
            }
            Ok(Event::End(e)) => {
                let name = e.name().local_name().as_ref().to_string();
                w_depth = w_depth.saturating_sub(1);
                if name == "p" && w_depth == 0 {
                    let trimmed = current.trim();
                    if !trimmed.is_empty() {
                        paragraphs.push(trimmed.to_string());
                    }
                    current.clear();
                }
            }
            Ok(Event::Text(t)) => {
                // 仅 w:t 内的文本 (简化: w_depth>0 即正文范围, w:t/w:instrText 不区分)
                if w_depth > 1 {
                    current.push_str(&t.xml_content(XmlVersion::Implicit1_0));
                }
            }
            Ok(Event::GeneralRef(r)) => {
                if w_depth > 1 {
                    let name = r.as_ref();
                    match name {
                        "amp" => current.push('&'),
                        "lt" => current.push('<'),
                        "gt" => current.push('>'),
                        "quot" => current.push('"'),
                        "apos" => current.push('\''),
                        _ => {
                            if let Ok(Some(c)) = r.resolve_char_ref() {
                                current.push(c);
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    Ok(paragraphs.join("

"))
}

fn open_zip(bytes: &[u8]) -> QyzsResult<zip::ZipArchive<BufReader<std::io::Cursor<&[u8]>>>> {
    let cursor = std::io::Cursor::new(bytes);
    zip::ZipArchive::new(BufReader::new(cursor))
        .map_err(|e| QyzsError::Usage(format!("无法解包 OOXML 容器: {}", e)))
}

fn read_zip_entry(
    archive: &mut zip::ZipArchive<BufReader<std::io::Cursor<&[u8]>>>,
    name: &str,
) -> QyzsResult<Vec<u8>> {
    let mut entry = archive
        .by_name(name)
        .map_err(|e| QyzsError::Server(format!("OOXML 部件缺失 {}: {}", name, e)))?;
    let mut buf = Vec::new();
    entry
        .read_to_end(&mut buf)
        .map_err(|e| QyzsError::Io(format!("读取部件 {} 失败: {}", name, e)))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_zip(entries: &[(&str, String)]) -> Vec<u8> {
        use std::io::Write as _;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, content) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(content.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn test_pptx_slide_extraction() {
        let slide1 = r#"<?xml version="1.0"?>
<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
<p:sp><p:txBody><a:p><a:r><a:t>国土空间规划</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:txBody><a:p><a:r><a:t>第一部分 总体要求</a:t></a:r></a:p><a:p><a:r><a:t>坚持底线思维</a:t></a:r></a:p></p:txBody></p:sp>
</p:spTree></p:cSld></p:sld>"#;
        let slide2 = r#"<?xml version="1.0"?>
<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree>
<p:sp><p:txBody><a:p><a:r><a:t>第二页</a:t></a:r></a:p></p:txBody></p:sp>
</p:spTree></p:cSld></p:sld>"#;
        let bytes = build_zip(&[
            ("[Content_Types].xml", "<x/>".into()),
            ("ppt/slides/slide1.xml", slide1.into()),
            ("ppt/slides/slide2.xml", slide2.into()),
        ]);

        let md = extract_pptx_text(&bytes).unwrap();
        assert!(md.contains("## 幻灯片 1"));
        assert!(md.contains("国土空间规划"));
        assert!(md.contains("第一部分 总体要求"));
        assert!(md.contains("坚持底线思维"));
        // 段落独立成行
        let lines: Vec<&str> = md.lines().collect();
        assert!(lines.iter().any(|l| *l == "国土空间规划"));
        // slide10 排在 slide2 之后
        assert!(md.contains("## 幻灯片 2"));
        assert!(md.find("国土空间规划").unwrap() < md.find("第二页").unwrap());
    }

    #[test]
    fn test_pptx_numeric_slide_order() {
        // slide10 必须排在 slide2 之后 (字符串序会错)
        let mut entries: Vec<(String, String)> = vec![("[Content_Types].xml".into(), "<x/>".into())];
        for i in 1..=10 {
            entries.push((
                format!("ppt/slides/slide{}.xml", i),
                format!(
                    r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>页{}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#,
                    i
                ),
            ));
        }
        let refs: Vec<(&str, String)> = entries.iter().map(|(a, b)| (a.as_str(), b.clone())).collect();
        let bytes = build_zip(&refs);
        let md = extract_pptx_text(&bytes).unwrap();
        let pos = |tag: &str| md.find(tag).unwrap();
        assert!(pos("页2") < pos("页10"));
    }

    #[test]
    fn test_vsdx_page_and_shape_extraction() {
        let pages_xml = r#"<?xml version="1.0"?>
<Pages xmlns="v"><Page ID="0" Name="流程图"/><Page ID="1" Name="组织架构"/></Pages>"#;
        let page1 = r#"<?xml version="1.0"?>
<PageContents xmlns="v"><Shapes>
<Shape ID="1" Type="Shape"><Text>项目启动</Text></Shape>
<Shape ID="2" Type="Shape"><Text>规划设计</Text></Shape>
</Shapes></PageContents>"#;
        let page2 = r#"<?xml version="1.0"?>
<PageContents xmlns="v"><Shapes><Shape ID="1"><Text>办公室</Text></Shape></Shapes></PageContents>"#;
        let bytes = build_zip(&[
            ("[Content_Types].xml", "<x/>".into()),
            ("visio/pages/pages.xml", pages_xml.into()),
            ("visio/pages/page1.xml", page1.into()),
            ("visio/pages/page2.xml", page2.into()),
        ]);

        let md = extract_vsdx_text(&bytes).unwrap();
        assert!(md.contains("## 页面: 流程图"));
        assert!(md.contains("## 页面: 组织架构"));
        assert!(md.contains("- 项目启动"));
        assert!(md.contains("- 规划设计"));
        assert!(md.find("流程图").unwrap() < md.find("组织架构").unwrap());
    }

    #[test]
    fn test_vsdx_nested_text_elements() {
        // Text 内嵌 <cp> 样式标记: 文本节点仍需完整聚合
        let pages_xml = r#"<Pages xmlns="v"><Page ID="0" Name="P"/></Pages>"#;
        let page1 = r#"<?xml version="1.0"?>
<PageContents xmlns="v"><Shapes><Shape ID="1"><Text><cp IX="0"/>带样式<cp IX="1"/>的文本</Text></Shape></Shapes></PageContents>"#;
        let bytes = build_zip(&[
            ("visio/pages/pages.xml", pages_xml.into()),
            ("visio/pages/page1.xml", page1.into()),
        ]);
        let md = extract_vsdx_text(&bytes).unwrap();
        assert!(md.contains("带样式"));
        assert!(md.contains("的文本"));
    }

    #[test]
    fn test_text_entities_via_general_ref() {
        // quick-xml 0.42 将实体拆为 GeneralRef 事件: 常规实体与数字字符引用需在
        // 文本聚合中显式处理 (曾因忽略该事件丢失 "&")
        let pages_xml = r#"<Pages xmlns="v"><Page ID="0" Name="P"/></Pages>"#;
        let page1 = r#"<?xml version="1.0"?>
<PageContents xmlns="v"><Shapes><Shape ID="1"><Text>A &amp; B &lt;C&gt; &#x4E2D;&#x6587;</Text></Shape></Shapes></PageContents>"#;
        let bytes = build_zip(&[
            ("visio/pages/pages.xml", pages_xml.into()),
            ("visio/pages/page1.xml", page1.into()),
        ]);
        let md = extract_vsdx_text(&bytes).unwrap();
        assert!(md.contains("A & B <C> 中文"), "实际输出: {}", md);
    }

    #[test]
    fn test_xml_entities_decoded() {
        let pages_xml = r#"<Pages xmlns="v"><Page ID="0" Name="A&amp;B"/></Pages>"#;
        let bytes = build_zip(&[("visio/pages/pages.xml", pages_xml.into())]);
        // 无页面内容部件 → 兜底无内容, 但清单解析应解码实体
        let pages = parse_page_list(r#"<Pages><Page ID="3" Name="A&amp;B &lt;测试&gt;"/></Pages>"#);
        assert_eq!(pages[0].1, "A&B <测试>");
        let _ = bytes;
    }

    #[test]
    fn test_empty_pptx_fails_clearly() {
        let bytes = build_zip(&[("[Content_Types].xml", "<x/>".into())]);
        let err = extract_pptx_text(&bytes).unwrap_err();
        assert!(matches!(err, crate::error::QyzsError::Server(_)));
    }
}
