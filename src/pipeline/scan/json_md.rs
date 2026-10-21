//! scan JSON → Markdown 通道
//!
//! 设计: 输出 pretty JSON 包裹在 ```json fenced code block, 不改变语义。
//! 若顶层是对象数组且所有元素同构 (字段集一致), 额外生成 Markdown 表格
//! (便于 LLM 横向对比), 后跟原始 JSON; 否则只输出 pretty JSON。
//! 字节先尝试 UTF-8, 失败回退 GB18030。

use super::workspace::Workspace;
use crate::error::{QyzsError, QyzsResult};
use serde_json::Value;

/// 元素同构判定的最小数组长度 (低于此长度直接 pretty 输出, 表格化收益小)
const TABLE_MIN_ITEMS: usize = 3;

/// 字节 → Markdown (含可选同构数组表格化)
pub fn json_to_markdown(bytes: &[u8]) -> QyzsResult<String> {
    let (text, encoding) = decode_json_bytes(bytes);
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| QyzsError::Server(format!("JSON 解析失败: {}", e)))?;

    let pretty = serde_json::to_string_pretty(&value)
        .map_err(|e| QyzsError::Server(format!("JSON 序列化失败: {}", e)))?;

    let mut out = String::new();
    if !encoding.is_empty() {
        out.push_str(&format!("<!-- 源文件编码: {} -->\n\n", encoding));
    }
    out.push_str(&format!("<!-- 顶层类型: {} -->\n\n", value_kind_label(&value)));

    // 同构数组 → Markdown 表格 + 原始 JSON
    if let Value::Array(items) = &value {
        if items.len() >= TABLE_MIN_ITEMS && items.iter().all(|v| v.is_object()) {
            let objects: Vec<&serde_json::Map<String, Value>> = items
                .iter()
                .filter_map(|v| v.as_object())
                .collect();
            if let Some(table) = homogeneous_table(&objects) {
                out.push_str(&table);
                out.push_str("\n\n---\n\n## 原始 JSON\n\n");
            }
        }
    }

    out.push_str("```json\n");
    out.push_str(&pretty);
    out.push_str("\n```\n");
    Ok(out)
}

/// 同构判定 + Markdown 表格生成
fn homogeneous_table(objects: &[&serde_json::Map<String, Value>]) -> Option<String> {
    if objects.is_empty() {
        return None;
    }
    let first_keys: Vec<String> = objects[0].keys().cloned().collect();
    if first_keys.is_empty() {
        return None;
    }
    // 所有对象字段集一致
    if !objects
        .iter()
        .all(|o| o.keys().cloned().collect::<Vec<_>>() == first_keys)
    {
        return None;
    }

    let mut out = format!(
        "## 数据表 ({} 行 × {} 列)\n\n",
        objects.len(),
        first_keys.len()
    );
    out.push_str(&md_row(&first_keys));
    out.push('\n');
    out.push_str(&md_separator(first_keys.len()));
    out.push('\n');
    for obj in objects {
        let cells: Vec<String> = first_keys
            .iter()
            .map(|k| {
                obj.get(k)
                    .map(value_compact)
                    .unwrap_or_default()
            })
            .collect();
        out.push_str(&md_row(&cells));
        out.push('\n');
    }
    Some(out)
}

fn value_compact(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(v).unwrap_or_default()
        }
    }
}

fn value_kind_label(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn md_row(cells: &[String]) -> String {
    let escaped: Vec<String> = cells
        .iter()
        .map(|c| c.replace('|', "\\|").replace('\n', " ").replace('\r', ""))
        .collect();
    format!("| {} |", escaped.join(" | "))
}

fn md_separator(n: usize) -> String {
    let parts: Vec<&str> = std::iter::repeat("---").take(n).collect();
    format!("| {} |", parts.join(" | "))
}

/// 字节 → (文本, 编码标签)
fn decode_json_bytes(bytes: &[u8]) -> (String, &'static str) {
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

pub fn extract_json(ws: &mut Workspace, path: &str) -> QyzsResult<String> {
    if ws.stage_done("document", "extract") {
        return ws.read_segment_text("document");
    }
    let bytes = std::fs::read(path)
        .map_err(|e| QyzsError::Io(format!("读取 JSON 文件失败 {}: {}", path, e)))?;
    let output = json_to_markdown(&bytes)?;
    ws.write_segment_text("document", &output)?;
    ws.complete_stage("document", "extract", &[])?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_object_json() {
        let json = r#"{"name":"Alice","age":30,"city":"Beijing"}"#;
        let md = json_to_markdown(json.as_bytes()).unwrap();
        assert!(md.contains("```json"));
        assert!(md.contains("\"name\": \"Alice\""));
        assert!(md.contains("顶层类型: object"));
    }

    #[test]
    fn test_array_json_no_table() {
        // 数组元素非全对象 → 不生成表格
        let json = r#"[1, 2, 3, "hello"]"#;
        let md = json_to_markdown(json.as_bytes()).unwrap();
        assert!(md.contains("```json"));
        assert!(!md.contains("数据表"));
    }

    #[test]
    fn test_homogeneous_array_gets_table() {
        let json = r#"[
            {"name":"Alice","age":30},
            {"name":"Bob","age":25},
            {"name":"Carol","age":28}
        ]"#;
        let md = json_to_markdown(json.as_bytes()).unwrap();
        assert!(md.contains("数据表 (3 行 × 2 列)"));
        assert!(md.contains("Alice"));
        assert!(md.contains("30"));
        assert!(md.contains("原始 JSON"));
    }

    #[test]
    fn test_heterogeneous_array_no_table() {
        let json = r#"[
            {"name":"Alice","age":30},
            {"name":"Bob","role":"admin"}
        ]"#;
        let md = json_to_markdown(json.as_bytes()).unwrap();
        assert!(!md.contains("数据表"));
        assert!(md.contains("```json"));
    }

    #[test]
    fn test_short_array_no_table() {
        // < 3 个元素 → 不生成表格
        let json = r#"[{"a":1}, {"a":2}]"#;
        let md = json_to_markdown(json.as_bytes()).unwrap();
        assert!(!md.contains("数据表"));
    }

    #[test]
    fn test_invalid_json_errors() {
        let err = json_to_markdown(b"not json at all").unwrap_err();
        assert!(matches!(err, QyzsError::Server(_)));
    }

    #[test]
    fn test_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"x":1}"#);
        let md = json_to_markdown(&bytes).unwrap();
        assert!(md.contains("UTF-8 (BOM)"));
        assert!(md.contains("\"x\": 1"));
    }

    #[test]
    fn test_nested_value_compact_in_table() {
        let json = r#"[
            {"name":"A","meta":{"id":1,"tags":["x","y"]}},
            {"name":"B","meta":{"id":2,"tags":["z"]}}
        ]"#;
        let md = json_to_markdown(json.as_bytes()).unwrap();
        // 嵌套对象在表格里序列化为内联 JSON
        assert!(md.contains("\"id\":1") || md.contains("\"id\": 1"));
    }
}
