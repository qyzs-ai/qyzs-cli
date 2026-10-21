//! markscan 表格抽取通道 (xlsx/xls → Markdown 表格, 纯 Rust 零 C 依赖)
//!
//! calamine 读取工作簿; 每个工作表输出一张 Markdown 表格:
//! - 首个非空行作为表头;
//! - 合并单元格按左上角值填充整个合并区域;
//! - 单表超过 MAX_TABLE_ROWS 时切分为续表 (表头重复)。

use super::workspace::Workspace;
use crate::error::{QyzsError, QyzsResult};
use calamine::{open_workbook, Data, Range, Reader, Xls, Xlsx};
use std::fs::File;
use std::io::BufReader;

/// 单张 Markdown 表格的最大数据行数 (超过切续表)
pub const MAX_TABLE_ROWS: usize = 5000;

/// 单元格值的文本表示 (整数化浮点; 空单元格为空串)
fn cell_display(v: &Data) -> String {
    match v {
        Data::Empty => String::new(),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                format!("{}", f)
            }
        }
        Data::Int(i) => format!("{}", i),
        Data::String(s) => s.trim().to_string(),
        Data::Bool(b) => format!("{}", b),
        Data::DateTime(dt) => format!("{}", dt),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("[{}]", e),
    }
}

/// 表格行转 Markdown 行 (管道分隔, 单元格内竖线转义)
fn md_row(cells: &[String]) -> String {
    let escaped: Vec<String> = cells
        .iter()
        .map(|c| c.replace('|', "\\|").replace('\n', " "))
        .collect();
    format!("| {} |", escaped.join(" | "))
}

/// 工作表数据 → Markdown 表格 (含续表切分)
pub fn sheet_to_markdown(name: &str, rows: &[Vec<String>]) -> String {
    let mut out = format!("## {}\n\n", name);

    // 去除表格尾部全空行
    let mut rows = rows.to_vec();
    while rows.last().is_some_and(|r| r.iter().all(String::is_empty)) {
        rows.pop();
    }
    if rows.is_empty() {
        out.push_str("_(空工作表)_\n");
        return out;
    }

    // 首个非空行作表头
    let header_idx = rows
        .iter()
        .position(|r| r.iter().any(|c| !c.is_empty()))
        .unwrap_or(0);
    let header = rows[header_idx].clone();
    let body: Vec<&Vec<String>> = rows[header_idx + 1..].iter().collect();

    let col_count = header.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
    let header_cells: Vec<String> = (0..col_count)
        .map(|i| header.get(i).cloned().unwrap_or_default())
        .collect();

    out.push_str(&md_row(&header_cells));
    out.push('\n');
    out.push_str(&format!("|{}|\n", vec!["---"; col_count].join("|")));

    let mut chunk = 0usize;
    for (i, row) in body.iter().enumerate() {
        if i % MAX_TABLE_ROWS == 0 && i > 0 {
            chunk += 1;
            out.push_str(&format!("\n## {} (续表 {})\n\n", name, chunk));
            out.push_str(&md_row(&header_cells));
            out.push('\n');
            out.push_str(&format!("|{}|\n", vec!["---"; col_count].join("|")));
        }
        let cells: Vec<String> = (0..col_count)
            .map(|c| row.get(c).cloned().unwrap_or_default())
            .collect();
        out.push_str(&md_row(&cells));
        out.push('\n');
    }
    out
}

/// 合并单元格还原: 左上角值填充整个合并区域 (坐标含首尾)
fn apply_merged_cells(grid: &mut [Vec<String>], merged: &[(u32, u32, u32, u32)]) {
    for &(start_row, start_col, end_row, end_col) in merged {
        let Some(top_left) = grid
            .get(start_row as usize)
            .and_then(|r| r.get(start_col as usize))
            .cloned()
        else {
            continue;
        };
        if top_left.is_empty() {
            continue;
        }
        for r in start_row..=end_row {
            for c in start_col..=end_col {
                if let Some(row) = grid.get_mut(r as usize) {
                    if let Some(cell) = row.get_mut(c as usize) {
                        *cell = top_left.clone();
                    }
                }
            }
        }
    }
}

/// 网格裁剪到实际使用区域后输出
fn range_to_markdown(name: &str, range: &Range<Data>, merged: &[(u32, u32, u32, u32)]) -> String {
    let (height, width) = range.get_size();
    let mut grid: Vec<Vec<String>> = vec![vec![String::new(); width]; height];
    for (r, row) in range.rows().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            grid[r][c] = cell_display(cell);
        }
    }
    apply_merged_cells(&mut grid, merged);

    let rows: Vec<Vec<String>> = grid
        .iter()
        .map(|row| {
            let last = row
                .iter()
                .rposition(|c| !c.is_empty())
                .map_or(0, |p| p + 1);
            row[..last].to_vec()
        })
        .collect();
    sheet_to_markdown(name, &rows)
}

/// xlsx → Markdown (每工作表一节)
pub fn extract_xlsx(ws: &mut Workspace, path: &str) -> QyzsResult<String> {
    if ws.stage_done("document", "extract") {
        return ws.read_segment_text("document");
    }
    let output = extract_xlsx_impl(path)?;
    ws.write_segment_text("document", &output)?;
    ws.complete_stage("document", "extract", &[])?;
    Ok(output)
}

/// xls (BIFF8) → Markdown
pub fn extract_xls(ws: &mut Workspace, path: &str) -> QyzsResult<String> {
    if ws.stage_done("document", "extract") {
        return ws.read_segment_text("document");
    }
    let output = extract_xls_impl(path)?;
    ws.write_segment_text("document", &output)?;
    ws.complete_stage("document", "extract", &[])?;
    Ok(output)
}

pub fn extract_xlsx_impl(path: &str) -> QyzsResult<String> {
    let mut workbook: Xlsx<BufReader<File>> = open_workbook(path)
        .map_err(|e| QyzsError::Usage(format!("无法打开 xlsx 文件 {}: {}", path, e)))?;

    let sheet_names = workbook.sheet_names().to_vec();
    let mut out = String::new();
    for name in &sheet_names {
        let range = workbook
            .worksheet_range(name)
            .map_err(|e| QyzsError::Server(format!("读取工作表 '{}' 失败: {}", name, e)))?;
        // 合并区域坐标含首尾 (Dimensions.start/end 为 (row, col))
        let merged: Vec<(u32, u32, u32, u32)> = workbook
            .merge_cells_by_sheet_name(name)
            .unwrap_or_default()
            .iter()
            .map(|d| (d.start.0, d.start.1, d.end.0, d.end.1))
            .collect();
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&range_to_markdown(name, &range, &merged));
    }
    Ok(out)
}

fn extract_xls_impl(path: &str) -> QyzsResult<String> {
    let mut workbook: Xls<BufReader<File>> = open_workbook(path)
        .map_err(|e| QyzsError::Usage(format!("无法打开 xls 文件 {}: {}", path, e)))?;

    let sheet_names = workbook.sheet_names().to_vec();
    let mut out = String::new();
    for name in &sheet_names {
        let range = workbook
            .worksheet_range(name)
            .map_err(|e| QyzsError::Server(format!("读取工作表 '{}' 失败: {}", name, e)))?;
        let merged: Vec<(u32, u32, u32, u32)> = workbook
            .merge_cells_by_sheet_name(name)
            .unwrap_or_default()
            .iter()
            .map(|d| (d.start.0, d.start.1, d.end.0, d.end.1))
            .collect();
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&range_to_markdown(name, &range, &merged));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sheet_to_markdown_basic() {
        let rows = vec![
            vec!["指标".into(), "数值".into()],
            vec!["耕地面积".into(), "1234.0".into()],
            vec!["比例".into(), "10%".into()],
        ];
        let md = sheet_to_markdown("清查表", &rows);
        assert!(md.starts_with("## 清查表\n\n"));
        assert!(md.contains("| 指标 | 数值 |"));
        assert!(md.contains("| 耕地面积 | 1234.0 |"));
        assert!(md.contains("|---|---|"));
    }

    #[test]
    fn test_pipe_escaping_and_newlines() {
        let rows = vec![vec!["列A".into()], vec!["含|竖线".into()]];
        let md = sheet_to_markdown("t", &rows);
        assert!(md.contains("含\\|竖线"));
    }

    #[test]
    fn test_continuation_table_over_limit() {
        let mut rows = vec![vec!["表头".into()]];
        for i in 0..(MAX_TABLE_ROWS + 20) {
            rows.push(vec![format!("行{}", i)]);
        }
        let md = sheet_to_markdown("大表", &rows);
        assert!(md.contains("(续表 1)"));
        let parts: Vec<&str> = md.split("(续表 1)").collect();
        assert_eq!(parts.len(), 2);
        assert!(parts[1].contains("| 表头 |"));
    }

    #[test]
    fn test_empty_sheet() {
        let md = sheet_to_markdown("空表", &[]);
        assert!(md.contains("_(空工作表)_"));
        let md2 = sheet_to_markdown("全空行", &[vec![String::new(); 3]]);
        assert!(md2.contains("_(空工作表)_"));
    }

    #[test]
    fn test_merged_cell_fill() {
        let mut grid = vec![
            vec!["类别".into(), String::new(), "B".into()],
            vec![String::new(), String::new(), "b1".into()],
            vec![String::new(), String::new(), "b2".into()],
        ];
        apply_merged_cells(&mut grid, &[(0, 0, 1, 1)]);
        assert_eq!(grid[1][0], "类别");
        assert_eq!(grid[1][1], "类别");
        assert_eq!(grid[2][0], "");
    }

    #[test]
    fn test_cell_display_integer_float() {
        assert_eq!(cell_display(&Data::Float(1234.0)), "1234");
        assert_eq!(cell_display(&Data::Float(10.5)), "10.5");
        assert_eq!(cell_display(&Data::Int(7)), "7");
        assert_eq!(cell_display(&Data::String(" 文本 ".into())), "文本");
        assert_eq!(cell_display(&Data::Empty), "");
    }

    /// 构造最小合法 xlsx (inlineStr 单元格) 供 calamine 读取
    fn minimal_xlsx(sheets: &[(&str, &[&[&str]])]) -> Vec<u8> {
        use std::io::Write as _;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opt =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        w.start_file("[Content_Types].xml", opt).unwrap();
        let mut ct = String::from(
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
        );
        for i in 0..sheets.len() {
            ct.push_str(&format!(
                r#"<Override PartName="/xl/worksheets/sheet{}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#,
                i + 1
            ));
        }
        ct.push_str("</Types>");
        w.write_all(ct.as_bytes()).unwrap();

        w.start_file("_rels/.rels", opt).unwrap();
        w.write_all(
            br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        )
        .unwrap();

        w.start_file("xl/workbook.xml", opt).unwrap();
        let mut wb = String::from(
            r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>"#,
        );
        for (i, (name, _)) in sheets.iter().enumerate() {
            wb.push_str(&format!(
                r#"<sheet name="{}" sheetId="{}" r:id="rId{}"/>"#,
                name,
                i + 1,
                i + 1
            ));
        }
        wb.push_str("</sheets></workbook>");
        w.write_all(wb.as_bytes()).unwrap();

        w.start_file("xl/_rels/workbook.xml.rels", opt).unwrap();
        let mut rels = String::from(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
        );
        for i in 0..sheets.len() {
            rels.push_str(&format!(
                r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{}.xml"/>"#,
                i + 1,
                i + 1
            ));
        }
        rels.push_str("</Relationships>");
        w.write_all(rels.as_bytes()).unwrap();

        for (i, (_, rows)) in sheets.iter().enumerate() {
            w.start_file(format!("xl/worksheets/sheet{}.xml", i + 1), opt)
                .unwrap();
            let mut sheet = String::from(
                r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
            );
            for (r, row) in rows.iter().enumerate() {
                sheet.push_str(&format!(r#"<row r="{}">"#, r + 1));
                for (c, cell) in row.iter().enumerate() {
                    sheet.push_str(&format!(
                        r#"<c r="{}{}" t="inlineStr"><is><t>{}</t></is></c>"#,
                        (b'A' + c as u8) as char,
                        r + 1,
                        xml_escape(cell)
                    ));
                }
                sheet.push_str("</row>");
            }
            sheet.push_str("</sheetData></worksheet>");
            w.write_all(sheet.as_bytes()).unwrap();
        }

        w.finish().unwrap().into_inner()
    }

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    #[test]
    fn test_extract_real_xlsx_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.xlsx");
        let bytes = minimal_xlsx(&[(
            "清查结果",
            &[&["指标", "数值"], &["耕地", "1000"], &["林地", "2000"]],
        )]);
        std::fs::write(&path, &bytes).unwrap();

        let cache = tempfile::tempdir().unwrap();
        let input = cache.path().join("in.xlsx");
        std::fs::write(&input, b"dummy").unwrap();
        let mut ws = crate::pipeline::markscan::workspace::Workspace::open_in(
            cache.path().join("ws"),
            &input,
            "xlsx",
            "speed",
            false,
        )
        .unwrap()
        .0;

        let md = extract_xlsx(&mut ws, path.to_str().unwrap()).unwrap();
        assert!(md.contains("## 清查结果"));
        assert!(md.contains("| 耕地 | 1000 |"));
        assert!(md.contains("| 林地 | 2000 |"));

        // 幂等: 二次抽取走缓存
        let md2 = extract_xlsx(&mut ws, path.to_str().unwrap()).unwrap();
        assert_eq!(md, md2);
        ws.finish_success(false).unwrap();
    }

    #[test]
    fn test_extract_xlsx_with_merged_cells() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m.xlsx");
        // 手工构造含 mergeCell 的最小 xlsx
        let bytes = minimal_xlsx_with_merge(&[(
            "合并表",
            &[&["类别", "子项"], &["耕地", "水田"], &["", "旱地"]],
            "A2:A3",
        )]);
        std::fs::write(&path, &bytes).unwrap();

        let cache = tempfile::tempdir().unwrap();
        let input = cache.path().join("in.xlsx");
        std::fs::write(&input, b"dummy").unwrap();
        let mut ws = crate::pipeline::markscan::workspace::Workspace::open_in(
            cache.path().join("ws"),
            &input,
            "xlsx",
            "speed",
            false,
        )
        .unwrap()
        .0;

        let md = extract_xlsx(&mut ws, path.to_str().unwrap()).unwrap();
        // 合并区域 A2:A3 左上角 "耕地" 填充 A3
        assert!(md.contains("| 耕地 | 水田 |"));
        assert!(md.contains("| 耕地 | 旱地 |"));
        ws.finish_success(false).unwrap();
    }

    /// minimal_xlsx 变体: 追加 mergeCells 声明
    fn minimal_xlsx_with_merge(
        sheets: &[(&str, &[&[&str]], &str)],
    ) -> Vec<u8> {
        use std::io::Write as _;
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opt =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        w.start_file("[Content_Types].xml", opt).unwrap();
        let ct = r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#;
        w.write_all(ct.as_bytes()).unwrap();

        w.start_file("_rels/.rels", opt).unwrap();
        w.write_all(
            br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        )
        .unwrap();

        w.start_file("xl/workbook.xml", opt).unwrap();
        w.write_all(
            r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="合并表" sheetId="1" r:id="rId1"/></sheets></workbook>"#.as_bytes(),
        )
        .unwrap();

        w.start_file("xl/_rels/workbook.xml.rels", opt).unwrap();
        w.write_all(
            br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
        )
        .unwrap();

        let (_, rows, merge_ref) = &sheets[0];
        w.start_file("xl/worksheets/sheet1.xml", opt).unwrap();
        let mut sheet = String::from(
            r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
        );
        for (r, row) in rows.iter().enumerate() {
            sheet.push_str(&format!(r#"<row r="{}">"#, r + 1));
            for (c, cell) in row.iter().enumerate() {
                sheet.push_str(&format!(
                    r#"<c r="{}{}" t="inlineStr"><is><t>{}</t></is></c>"#,
                    (b'A' + c as u8) as char,
                    r + 1,
                    xml_escape(cell)
                ));
            }
            sheet.push_str("</row>");
        }
        sheet.push_str("</sheetData>");
        sheet.push_str(&format!(r#"<mergeCells count="1"><mergeCell ref="{}"/></mergeCells>"#, merge_ref));
        sheet.push_str("</worksheet>");
        w.write_all(sheet.as_bytes()).unwrap();

        w.finish().unwrap().into_inner()
    }
}
