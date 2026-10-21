//! scan ZIP 容器迭代通道
//!
//! 遍历 .zip 内部条目, 按子格式路由到现有通道 (csv/json/xml/html/ooxml/sheet),
//! 合并为单 .md, 章节标题用 `## <原 ZIP 名>/<内部相对路径>`。
//!
//! 安全防御 (zip-bomb 防护):
//! - 解压后总字节 ≤ [`MAX_TOTAL_UNPACKED_BYTES`]
//! - 单条目解压后 ≤ [`MAX_ENTRY_UNPACKED_BYTES`]
//! - 条目数 ≤ [`MAX_ENTRIES`]
//! - 嵌套 ZIP 不递归 (常见 zip-bomb 形态)
//!
//! PDF 通道特殊: 走 MinerU 网关切片, 内部需要 client + 工作目录,
//! 在 ZIP 容器内调用受限 — 失败回退为警告条目而非中断整体 (避免一个 PDF
//! 拖垮整个 ZIP 处理)。

use super::csv_md;
use super::detect::{FormatDetector, SourceFormat};
use super::html_md;
use super::json_md;
use super::workspace::Workspace;
use super::xml_md;
use crate::error::{QyzsError, QyzsResult};
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;
use zip::ZipArchive;

/// ZIP 解压后总字节上限 (防止 zip-bomb)
pub const MAX_TOTAL_UNPACKED_BYTES: u64 = 500 * 1024 * 1024;

/// 单条目解压后字节上限
pub const MAX_ENTRY_UNPACKED_BYTES: u64 = 100 * 1024 * 1024;

/// 条目数量上限
pub const MAX_ENTRIES: usize = 1000;

/// 跳过嵌套 ZIP (zip-bomb 常见形态, 不递归)
const SKIP_NESTED_ZIP: bool = true;

/// ZIP → Markdown (合并容器内所有支持格式条目)
pub fn zip_to_markdown(path: &str) -> QyzsResult<String> {
    let file = File::open(path)
        .map_err(|e| QyzsError::Io(format!("打开 ZIP 文件失败 {}: {}", path, e)))?;
    let mut archive =
        ZipArchive::new(file).map_err(|e| QyzsError::Server(format!("ZIP 解析失败: {}", e)))?;

    let entry_count = archive.len();
    if entry_count == 0 {
        return Err(QyzsError::Server("ZIP 文件为空".to_string()));
    }
    if entry_count > MAX_ENTRIES {
        return Err(QyzsError::Usage(format!(
            "ZIP 条目数 {} 超过上限 {}",
            entry_count, MAX_ENTRIES
        )));
    }

    let zip_name = Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());

    let mut total_bytes: u64 = 0;
    let mut out = String::new();
    let mut processed: usize = 0;
    let mut skipped: Vec<String> = Vec::new();
    let mut failed: Vec<(String, String)> = Vec::new();

    // 收集条目名以便按需排序 (确定性输出)
    let names: Vec<String> = (0..entry_count)
        .map(|i| {
            archive
                .by_index(i)
                .and_then(|f| f.name().map(|cow| cow.into_owned()))
        })
        .collect::<Result<_, _>>()
        .map_err(|e| QyzsError::Server(format!("ZIP 读取条目失败: {}", e)))?;
    let mut indexed: Vec<usize> = (0..entry_count).collect();
    indexed.sort_by_key(|&i| names[i].clone());

    for idx in indexed {
        let entry_name = names[idx].clone();
        let mut entry = match archive.by_index(idx) {
            Ok(e) => e,
            Err(e) => {
                failed.push((entry_name, format!("条目打开失败: {}", e)));
                continue;
            }
        };

        // 目录条目跳过
        if entry.is_dir() {
            continue;
        }

        // 嵌套 ZIP 跳过
        if SKIP_NESTED_ZIP && entry_name.to_lowercase().ends_with(".zip") {
            skipped.push(format!("{} (嵌套 ZIP, 默认不递归)", entry_name));
            continue;
        }

        // 单条目体积预检 (entry.size() 为声明解压后大小, 0 表示流式未知)
        // 仅当声明值 > 0 且 > 上限时跳过; 0 时让 read_to_end 真实读取
        let entry_size = entry.size();
        if entry_size > MAX_ENTRY_UNPACKED_BYTES {
            skipped.push(format!(
                "{} (解压前 {} 字节 > 单条目上限 {})",
                entry_name,
                entry_size,
                MAX_ENTRY_UNPACKED_BYTES
            ));
            continue;
        }

        // 读条目字节 (entry.size() 可能为 0, 不可信; 不预分配)
        let mut bytes = Vec::new();
        if let Err(e) = entry.read_to_end(&mut bytes) {
            failed.push((entry_name.clone(), format!("读取失败: {}", e)));
            continue;
        }
        if bytes.is_empty() {
            failed.push((entry_name.clone(), "条目读取后为空".to_string()));
            continue;
        }
        if bytes.len() as u64 > MAX_ENTRY_UNPACKED_BYTES {
            skipped.push(format!(
                "{} (解压后 {} 字节 > 单条目上限 {})",
                entry_name,
                bytes.len(),
                MAX_ENTRY_UNPACKED_BYTES
            ));
            continue;
        }

        // 总字节累计 + 上限
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        if total_bytes > MAX_TOTAL_UNPACKED_BYTES {
            return Err(QyzsError::Usage(format!(
                "ZIP 解压后总字节 {} 超过上限 {} (zip-bomb 防护)",
                total_bytes, MAX_TOTAL_UNPACKED_BYTES
            )));
        }

        // 嗅探子格式: 内容优先, Unknown 时回退到条目名扩展名
        let format = {
            let sniffed = FormatDetector::detect_bytes(&bytes);
            if sniffed != SourceFormat::Unknown {
                sniffed
            } else {
                // ZIP 内条目名是最可靠的 hint (例如 CSV/JSON 这种无魔数格式)
                let ext = Path::new(&entry_name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s.to_ascii_lowercase());
                match ext.as_deref() {
                    Some("csv") => SourceFormat::Csv,
                    Some("json") => SourceFormat::Json,
                    Some("xml") => SourceFormat::Xml,
                    Some("html") | Some("htm") => SourceFormat::Html,
                    Some("xlsx") => SourceFormat::Xlsx,
                    Some("xls") => SourceFormat::Xls,
                    Some("pptx") => SourceFormat::Pptx,
                    Some("vsdx") => SourceFormat::Vsdx,
                    Some("pdf") => SourceFormat::Pdf,
                    Some("docx") => SourceFormat::Docx,
                    Some("zip") => SourceFormat::Zip,
                    _ => SourceFormat::Unknown,
                }
            }
        };

        // 章节标题
        out.push_str(&format!("## {}/{}\n\n", zip_name, entry_name));

        // 按子格式路由 (PDF/MinerU 走网关, 容器内调用风险高, 走降级)
        match format {
            SourceFormat::Csv => match csv_md::csv_to_markdown(&bytes) {
                Ok(md) => {
                    out.push_str(&md);
                    processed += 1;
                }
                Err(e) => failed.push((entry_name.clone(), e.to_string())),
            },
            SourceFormat::Json => match json_md::json_to_markdown(&bytes) {
                Ok(md) => {
                    out.push_str(&md);
                    processed += 1;
                }
                Err(e) => failed.push((entry_name.clone(), e.to_string())),
            },
            SourceFormat::Xml => match xml_md::xml_to_markdown(&bytes) {
                Ok(md) => {
                    out.push_str(&md);
                    processed += 1;
                }
                Err(e) => failed.push((entry_name.clone(), e.to_string())),
            },
            SourceFormat::Html => match html_md::html_to_markdown(&bytes) {
                Ok(md) => {
                    out.push_str(&md);
                    processed += 1;
                }
                Err(e) => failed.push((entry_name.clone(), e.to_string())),
            },
            SourceFormat::Xlsx => {
                // XLSX 需读文件, 临时落盘到 work dir (zip_iter 工作流内可用 tempfile)
                match extract_xlsx_from_bytes(&bytes) {
                    Ok(md) => {
                        out.push_str(&md);
                        processed += 1;
                    }
                    Err(e) => failed.push((entry_name.clone(), e.to_string())),
                }
            }
            SourceFormat::Pptx => match super::ooxml::extract_pptx_text(&bytes) {
                Ok(md) => {
                    out.push_str(&md);
                    processed += 1;
                }
                Err(e) => failed.push((entry_name.clone(), e.to_string())),
            },
            SourceFormat::Vsdx => match super::ooxml::extract_vsdx_text(&bytes) {
                Ok(md) => {
                    out.push_str(&md);
                    processed += 1;
                }
                Err(e) => failed.push((entry_name.clone(), e.to_string())),
            },
            SourceFormat::Pdf => {
                // PDF 走 MinerU 网关, 容器内调用需 client + work_dir, 简化: 跳过并记录
                skipped.push(format!(
                    "{} (PDF 需 MinerU 网关, ZIP 容器内暂未支持; 建议解压后单独跑 scan)",
                    entry_name
                ));
            }
            SourceFormat::Docx => {
                skipped.push(format!(
                    "{} (DOCX 需 MinerU 网关, ZIP 容器内暂未支持)",
                    entry_name
                ));
            }
            SourceFormat::Image(_) => {
                skipped.push(format!("{} (图像通道需网关 OCR+VL)", entry_name));
            }
            SourceFormat::Zip => {
                skipped.push(format!("{} (嵌套 ZIP 默认不递归)", entry_name));
            }
            _ => {
                skipped.push(format!("{} (子格式 {:?} 未支持)", entry_name, format));
            }
        }

        out.push_str("\n\n---\n\n");
    }

    // 汇总页脚
    if !skipped.is_empty() {
        out.push_str("## 跳过条目\n\n");
        for s in &skipped {
            out.push_str(&format!("- {}\n", s));
        }
        out.push('\n');
    }
    if !failed.is_empty() {
        out.push_str("## 处理失败条目\n\n");
        for (name, reason) in &failed {
            out.push_str(&format!("- `{}`: {}\n", name, reason));
        }
        out.push('\n');
    }
    out.push_str(&format!(
        "<!-- 汇总: 共 {} 个条目, 成功 {}, 跳过 {}, 失败 {} -->\n",
        entry_count,
        processed,
        skipped.len(),
        failed.len()
    ));

    if processed == 0 {
        return Err(QyzsError::Server(format!(
            "ZIP 容器内无任何条目可处理 (共 {} 个, 全部被跳过或失败; 见内部 '跳过/失败' 列表)",
            entry_count
        )));
    }

    Ok(out)
}

/// XLSX 走 calamine 需文件路径 → 写到临时文件再读
fn extract_xlsx_from_bytes(bytes: &[u8]) -> QyzsResult<String> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("entry.xlsx");
    std::fs::write(&path, bytes)?;
    let cursor = Cursor::new(path.to_string_lossy().to_string());
    let _ = cursor; // 仅用于 RAII 显式生命周期
    super::sheet_tables::extract_xlsx_impl(&path.to_string_lossy())
}

pub fn extract_zip(ws: &mut Workspace, path: &str) -> QyzsResult<String> {
    if ws.stage_done("document", "extract") {
        return ws.read_segment_text("document");
    }
    let output = zip_to_markdown(path)?;
    ws.write_segment_text("document", &output)?;
    ws.complete_stage("document", "extract", &[])?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in entries {
            writer
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn test_empty_zip_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.zip");
        std::fs::write(&path, make_zip(&[])).unwrap();
        let err = zip_to_markdown(&path.to_string_lossy()).unwrap_err();
        assert!(matches!(err, QyzsError::Server(_)));
    }

    #[test]
    fn test_basic_zip_with_csv() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.zip");
        std::fs::write(
            &path,
            make_zip(&[("data.csv", b"name,age\nAlice,30\nBob,25\n")]),
        )
        .unwrap();
        match zip_to_markdown(&path.to_string_lossy()) {
            Ok(md) => {
                assert!(md.contains("## data.zip/data.csv"), "missing section: {}", md);
                assert!(md.contains("Alice"));
                assert!(md.contains("2 行 × 2 列"));
            }
            Err(e) => panic!("zip_to_markdown failed: {} (debug={:?})", e, e),
        }
    }

    #[test]
    fn test_zip_with_multiple_formats() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mixed.zip");
        std::fs::write(
            &path,
            make_zip(&[
                ("a.csv", b"k,v\n1,2\n"),
                ("b.json", br#"{"x":1}"#),
                ("c.xml", b"<root>v</root>"),
            ]),
        )
        .unwrap();
        let md = zip_to_markdown(&path.to_string_lossy()).unwrap();
        assert!(md.contains("## mixed.zip/a.csv"));
        assert!(md.contains("## mixed.zip/b.json"));
        assert!(md.contains("## mixed.zip/c.xml"));
        assert!(md.contains("Alice") == false); // sanity
        assert!(md.contains("k"));
        assert!(md.contains("\"x\": 1"));
        assert!(md.contains("**root**: v"));
    }

    #[test]
    fn test_zip_skips_unsupported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.zip");
        std::fs::write(
            &path,
            make_zip(&[
                ("a.csv", b"k,v\n1,2\n"),
                ("b.pdf", b"%PDF-1.5\nfake"), // PDF 在容器内跳过
                ("c.doc", b"\xD0\xCF\x11\xE0"), // OLE2/旧 doc
            ]),
        )
        .unwrap();
        let md = zip_to_markdown(&path.to_string_lossy()).unwrap();
        assert!(md.contains("## partial.zip/a.csv"));
        assert!(md.contains("## 跳过条目"));
        assert!(md.contains("b.pdf"));
        assert!(md.contains("c.doc"));
    }

    #[test]
    fn test_zip_skips_nested_zip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested.zip");
        let inner = make_zip(&[("x.txt", b"hello")]);
        std::fs::write(
            &path,
            make_zip(&[("data.csv", b"k,v\n1,2\n"), ("inner.zip", &inner)]),
        )
        .unwrap();
        let md = zip_to_markdown(&path.to_string_lossy()).unwrap();
        assert!(md.contains("## nested.zip/data.csv"));
        assert!(md.contains("嵌套 ZIP"));
    }

    #[test]
    fn test_zip_single_entry_size_limit() {
        // 单条目超 MAX_ENTRY_UNPACKED_BYTES → 走单条目拦截 (非总字节拦截),
        // 处理函数 Ok 返回, 但 Skipped 列表里有该条目
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge_entry.zip");
        let big = vec![b'x'; (MAX_ENTRY_UNPACKED_BYTES + 1) as usize];
        // 第二个小条目保证 processed > 0
        std::fs::write(
            &path,
            make_zip(&[("big.bin", &big), ("ok.csv", b"a,b\n1,2\n")]),
        )
        .unwrap();
        let md = zip_to_markdown(&path.to_string_lossy()).unwrap();
        // 单条目被跳过, 但小条目成功, 整体 Ok
        assert!(md.contains("## 跳过条目"));
        assert!(md.contains("big.bin"));
        assert!(md.contains("ok.csv"));
    }
}
