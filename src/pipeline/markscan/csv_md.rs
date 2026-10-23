//! markscan CSV → Markdown 表格通道 (纯 Rust, 零 C 依赖)
//!
//! 编码自适应: 优先 UTF-8 (含 BOM), 否则按 GB18030 解码 (覆盖 GBK / GB2312 公文附表);
//! 自动嗅探分隔符 (`,` / `;` / `\t` / `|`); 按首个非空行作表头; 单表超
//! [`MAX_ROWS_PER_TABLE`] 时切续表 (表头重复); 空文件与无表头分别显式标注。

use super::workspace::Workspace;
use crate::error::{QyzsError, QyzsResult};

/// 单张 Markdown 表格最大数据行 (含表头); 超限切续表。公文附表实测
/// 几千行常见, 1000 行/段契合 24k 字符润色上限 (中文 ≈ 12 字节/字符)。
pub const MAX_ROWS_PER_TABLE: usize = 1000;

/// 嗅探候选分隔符 (按频度优先匹配; 命中后该分隔符主导整文件)
const CANDIDATE_DELIMS: &[u8] = b",;\t|";

/// 字节流 → Markdown 表格
pub fn csv_to_markdown(bytes: &[u8]) -> QyzsResult<String> {
    let (text, encoding) = decode_csv_bytes(bytes);
    let delim = sniff_delim(text.as_bytes());

    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delim)
        .has_headers(false) // 自行判定首行是否为表头
        .flexible(true)
        .from_reader(text.as_bytes());

    let rows: Vec<Vec<String>> = reader
        .records()
        .map(|r| {
            r.map(|rec| rec.iter().map(sanitize_cell).collect::<Vec<_>>())
                .map_err(|e| QyzsError::Server(format!("CSV 解析失败: {}", e)))
        })
        .collect::<QyzsResult<Vec<_>>>()?;

    let mut out = String::new();
    if !encoding.is_empty() {
        out.push_str(&format!("<!-- 源文件编码: {} -->\n\n", encoding));
    }
    if rows.is_empty() {
        out.push_str("_(空 CSV)_");
        return Ok(out);
    }

    // 表头识别: 若首行所有字段非空且为纯文本 (无数字主导), 视为表头;
    // 否则整文件视为数据, 自动生成列名 (C1, C2, ...)。
    let (header, data_start) = detect_header(&rows);
    let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let effective_header: Vec<String> = if header.is_empty() {
        (1..=col_count).map(|i| format!("C{}", i)).collect()
    } else {
        header
    };

    let total_data_rows = rows.len() - data_start;
    out.push_str(&format!(
        "**{} 行 × {} 列**\n\n",
        total_data_rows, col_count
    ));

    // 切续表: 每个 chunk 输出独立 Markdown 表格 (表头重复)
    let data = &rows[data_start..];
    for (chunk_idx, chunk) in data.chunks(MAX_ROWS_PER_TABLE).enumerate() {
        if chunk_idx > 0 {
            out.push('\n');
        }
        append_table(&mut out, &effective_header, chunk, chunk_idx);
    }

    Ok(out)
}

/// 追加一张表格 (表头 + 数据行; 末列空字符串裁剪)
fn append_table(out: &mut String, header: &[String], rows: &[Vec<String>], chunk_idx: usize) {
    if chunk_idx > 0 {
        out.push_str(&format!(
            "_(续表 {}，第 {}–{} 行)_\n\n",
            chunk_idx,
            chunk_idx * MAX_ROWS_PER_TABLE + 1,
            chunk_idx * MAX_ROWS_PER_TABLE + rows.len()
        ));
    }
    out.push_str(&md_row(header));
    out.push('\n');
    out.push_str(&md_separator(header.len()));
    out.push('\n');
    for row in rows {
        out.push_str(&md_row_padded(row, header.len()));
        out.push('\n');
    }
}

fn md_row(cells: &[String]) -> String {
    let escaped: Vec<String> = cells
        .iter()
        .map(|c| c.replace('|', "\\|").replace('\n', " ").replace('\r', ""))
        .collect();
    format!("| {} |", escaped.join(" | "))
}

fn md_row_padded(row: &[String], width: usize) -> String {
    let mut cells: Vec<String> = row.to_vec();
    while cells.len() < width {
        cells.push(String::new());
    }
    cells.truncate(width);
    md_row(&cells)
}

fn md_separator(n: usize) -> String {
    let parts: Vec<&str> = std::iter::repeat_n("---", n).collect();
    format!("| {} |", parts.join(" | "))
}

/// 单元格清洗: 去掉首尾空白, 折叠行内多空白
fn sanitize_cell(s: &str) -> String {
    let trimmed = s.trim();
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_space = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out
}

/// 表头判定: 首行所有字段非空 + 全行字段类型与数据行不一致时视为表头
fn detect_header(rows: &[Vec<String>]) -> (Vec<String>, usize) {
    if rows.is_empty() {
        return (Vec::new(), 0);
    }
    let first = &rows[0];
    if first.is_empty() || first.iter().any(|c| c.is_empty()) {
        return (Vec::new(), 0);
    }
    // 数据行若存在数字字段, 而首行同列无数字, 判为表头
    if rows.len() >= 2 {
        let data_has_num = rows[1..].iter().any(|r| r.iter().any(|c| looks_numeric(c)));
        let header_no_num = first.iter().all(|c| !looks_numeric(c));
        if data_has_num && header_no_num {
            return (first.clone(), 1);
        }
    }
    (Vec::new(), 0)
}

fn looks_numeric(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let s = s.trim();
    s.parse::<f64>().is_ok()
}

/// 嗅探分隔符: 统计各候选在首 8KB 内的"独占列数"(行内计数)平均值, 取最高者
fn sniff_delim(bytes: &[u8]) -> u8 {
    let sample = &bytes[..bytes.len().min(8 * 1024)];
    let mut best = b',';
    let mut best_score = 0.0f64;
    for &d in CANDIDATE_DELIMS {
        let mut total_cols = 0usize;
        let mut line_count = 0usize;
        for line in sample.split(|&b| b == b'\n').take(50) {
            if line.is_empty() {
                continue;
            }
            // 不计被引号包裹的分隔符
            let mut in_quote = false;
            let mut cols = 1usize;
            for &b in line {
                match b {
                    b'"' => in_quote = !in_quote,
                    x if x == d && !in_quote => cols += 1,
                    _ => {}
                }
            }
            total_cols += cols;
            line_count += 1;
        }
        if line_count == 0 {
            continue;
        }
        let avg = total_cols as f64 / line_count as f64;
        // 至少 2 列才算有效分隔符
        if avg >= 2.0 && avg > best_score {
            best_score = avg;
            best = d;
        }
    }
    best
}

/// 字节 → (文本, 编码标签); 优先 UTF-8 (含 BOM), 否则 GB18030
fn decode_csv_bytes(bytes: &[u8]) -> (String, &'static str) {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let body = &bytes[3..];
        return (String::from_utf8_lossy(body).into_owned(), "UTF-8 (BOM)");
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => (s.to_string(), "UTF-8"),
        Err(_) => {
            let (cow, _, had_errors) = encoding_rs::GB18030.decode(bytes);
            if had_errors {
                // 双重失败: 容错回退 (替换无效字节)
                (cow.into_owned(), "GB18030 (有损)")
            } else {
                (cow.into_owned(), "GB18030")
            }
        }
    }
}

pub fn extract_csv(ws: &mut Workspace, path: &str) -> QyzsResult<String> {
    if ws.stage_done("document", "extract") {
        return ws.read_segment_text("document");
    }
    let bytes = std::fs::read(path)
        .map_err(|e| QyzsError::Io(format!("读取 CSV 文件失败 {}: {}", path, e)))?;
    let output = csv_to_markdown(&bytes)?;
    ws.write_segment_text("document", &output)?;
    ws.complete_stage("document", "extract", &[])?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_csv() {
        let csv = "name,age,city\nAlice,30,Beijing\nBob,25,Shanghai\n";
        let md = csv_to_markdown(csv.as_bytes()).unwrap();
        assert!(md.contains("name"));
        assert!(md.contains("Alice"));
        assert!(md.contains("Shanghai"));
        assert!(md.contains("|"));
        assert!(md.contains("2 行 × 3 列"));
    }

    #[test]
    fn test_tsv_delimiter() {
        let tsv = "a\tb\tc\n1\t2\t3\n4\t5\t6\n";
        let md = csv_to_markdown(tsv.as_bytes()).unwrap();
        assert!(md.contains("a"));
        assert!(md.contains("1"));
        assert!(md.contains("4"));
    }

    #[test]
    fn test_no_header_numeric_data() {
        let csv = "10,20,30\n40,50,60\n";
        let md = csv_to_markdown(csv.as_bytes()).unwrap();
        // 首行为数字 → 视为数据, 自动生成 C1/C2/C3 表头
        assert!(md.contains("C1"));
        assert!(md.contains("C2"));
        assert!(md.contains("C3"));
        assert!(md.contains("10"));
    }

    #[test]
    fn test_pipe_escape() {
        let csv = "name,desc\nfoo,has|pipe\n";
        let md = csv_to_markdown(csv.as_bytes()).unwrap();
        assert!(md.contains("foo"));
        assert!(md.contains("has\\|pipe"));
    }

    #[test]
    fn test_gbk_decoding() {
        // "中文,测试\n你好,世界" 的 GBK 编码
        let gbk_bytes: &[u8] = &[
            0xD6, 0xD0, 0xCE, 0xC4, b',', 0xB2, 0xE2, 0xCA, 0xD4, b'\n', 0xC4, 0xE3, 0xBA, 0xC3,
            b',', 0xCA, 0xC0, 0xBD, 0xE7,
        ];
        let md = csv_to_markdown(gbk_bytes).unwrap();
        assert!(md.contains("GB18030"));
        assert!(md.contains("中文"));
        assert!(md.contains("测试"));
    }

    #[test]
    fn test_utf8_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("h,v\n1,2\n".as_bytes());
        let md = csv_to_markdown(&bytes).unwrap();
        assert!(md.contains("UTF-8 (BOM)"));
        assert!(md.contains("h"));
        assert!(md.contains("1"));
    }

    #[test]
    fn test_quoted_delimiter_not_counted() {
        let csv = "a,b\n\"x,y\",2\n";
        let md = csv_to_markdown(csv.as_bytes()).unwrap();
        // "x,y" 是单字段, 不应被逗号拆开
        assert!(md.contains("x,y"));
    }

    #[test]
    fn test_continuation_table() {
        let mut csv = String::from("h\n");
        for i in 0..(MAX_ROWS_PER_TABLE + 100) {
            csv.push_str(&format!("{}\n", i));
        }
        let md = csv_to_markdown(csv.as_bytes()).unwrap();
        assert!(md.contains("续表 1"));
        assert!(md.contains(&format!("{}", MAX_ROWS_PER_TABLE + 100)));
    }

    #[test]
    fn test_empty_csv() {
        let md = csv_to_markdown(b"").unwrap();
        assert!(md.contains("空 CSV"));
    }

    #[test]
    fn test_sanitize_cell_whitespace() {
        assert_eq!(sanitize_cell("  hello   world  "), "hello world");
        assert_eq!(sanitize_cell(""), "");
        assert_eq!(sanitize_cell("a\nb"), "a b");
    }
}
