//! 图片附件抽取与引用重写 (markscan 的 assets 阶段)
//!
//! 两种来源, 能力不同, 必须分开对待:
//!
//! **OOXML (docx/pptx/xlsx/vsdx)** —— 精确。容器内 `word/media`、`ppt/media`、
//! `xl/media` 就是原图字节, 直接取出即可, 文件名稳定可追溯。
//!
//! **PDF** —— 尽力而为。lopdf 能取出页面内嵌图, 但要区分「整页扫描底图」与
//! 「真实插图」: 面积占比接近整页的属于扫描页本身, 不算附件 (抽出 16 张 A4 扫描
//! 底图给使用方毫无价值)。另外 MinerU 正文里的 `images/<sha256>.jpg` 引用不是
//! 内容哈希 (实测 16 张内嵌图零命中), 无法按名还原, 只能按出现顺序与抽出的
//! 插图对应; 数量不匹配时剩余引用改写为明确缺失标记, 不伪造。
//!
//! 输出布局: `<assets_dir>/<源文件名>/<page|media>-<序号>.<ext>`, Markdown 中使用
//! 相对路径引用, 保证 .md 与 assets 目录可整体搬移。

use super::detect::SourceFormat;
use crate::error::{QyzsError, QyzsResult};
use std::path::{Path, PathBuf};

/// 抽取出的附件
#[derive(Debug, Clone)]
pub struct ExtractedAsset {
    /// 落盘绝对路径
    pub file: PathBuf,
    /// Markdown 中使用的相对路径 (相对 .md 所在目录)
    pub rel: String,
    /// 来源标注 (如 "page-3" / "word/media/image1.png")
    pub origin: String,
}

/// 按格式抽取附件到 `<assets_root>/<stem>/`
///
/// 返回空 Vec 表示该文档没有可抽取的附件 (纯文本/纯扫描件属正常情况)。
pub fn extract(
    source_path: &str,
    format: &SourceFormat,
    assets_root: &Path,
    stem: &str,
    rel_base: &Path,
) -> QyzsResult<Vec<ExtractedAsset>> {
    let dir = assets_root.join(sanitize(stem));
    let assets = match format {
        SourceFormat::Docx | SourceFormat::Pptx | SourceFormat::Xlsx | SourceFormat::Vsdx => {
            extract_ooxml_media(source_path, &dir, rel_base)?
        }
        SourceFormat::Pdf => extract_pdf_figures(source_path, &dir, rel_base)?,
        _ => Vec::new(),
    };
    Ok(assets)
}

/// 文件名净化: 去掉路径分隔符与空白, 避免落到 assets 目录外
fn sanitize(stem: &str) -> String {
    let s: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let trimmed = s.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "document".to_string()
    } else {
        trimmed
    }
}

// ── OOXML: 精确媒体抽取 ────────────────────────────────────────────────

const MEDIA_PREFIXES: &[&str] = &["word/media/", "ppt/media/", "xl/media/", "visio/media/"];

fn extract_ooxml_media(source_path: &str, dir: &Path, rel_base: &Path) -> QyzsResult<Vec<ExtractedAsset>> {
    let bytes = std::fs::read(source_path)
        .map_err(|e| QyzsError::Io(format!("读取源文件失败 {}: {}", source_path, e)))?;
    let cursor = std::io::Cursor::new(&bytes[..]);
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(cursor))
        .map_err(|e| QyzsError::Usage(format!("无法解包 OOXML 容器: {}", e)))?;

    // 先收集条目名 (ZipArchive 借用与读取不能同时持有)
    let mut entries: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        if let Ok(f) = archive.by_index(i) {
            let Ok(name) = f.name().map(|n| n.to_string()) else { continue };
            if MEDIA_PREFIXES.iter().any(|p| name.starts_with(p)) && !name.ends_with('/') {
                entries.push(name);
            }
        }
    }
    entries.sort();

    let mut out = Vec::new();
    std::fs::create_dir_all(dir)?;
    for name in entries {
        let mut f = archive
            .by_name(&name)
            .map_err(|e| QyzsError::Io(format!("读取容器条目 {} 失败: {}", name, e)))?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut buf)?;
        if buf.is_empty() {
            continue;
        }
        let fname = name.rsplit('/').next().unwrap_or("asset");
        let target = dir.join(fname);
        std::fs::write(&target, &buf)?;
        out.push(ExtractedAsset {
            rel: rel_path(rel_base, &target),
            file: target,
            origin: name.clone(),
        });
    }
    Ok(out)
}

/// 计算 Markdown 中应使用的相对路径 (相对 .md 所在目录)
///
/// 目标在基准目录之下时输出 `assets/<stem>/<file>`; 不在其下 (用户用
/// --assets-dir 指到别处) 时退化为可读的完整路径, 不假装相对。
fn rel_path(rel_base: &Path, target: &Path) -> String {
    if let Ok(stripped) = target.strip_prefix(rel_base) {
        return stripped.to_string_lossy().replace('\\', "/");
    }
    target.to_string_lossy().replace('\\', "/")
}

// ── PDF: 区分整页底图与真实插图 ─────────────────────────────────────────

fn extract_pdf_figures(source_path: &str, dir: &Path, rel_base: &Path) -> QyzsResult<Vec<ExtractedAsset>> {
    let doc = lopdf::Document::load(source_path)
        .map_err(|e| QyzsError::Pdf(format!("加载 PDF 失败: {}", e)))?;

    let mut out = Vec::new();
    std::fs::create_dir_all(dir)?;

    for (page_no, page_id) in doc.get_pages() {
        // 页面可视面积 (pt²)
        let page_area = page_media_box_area(&doc, page_id).unwrap_or(0.0);
        let Ok(images) = doc.get_page_images(page_id) else {
            continue;
        };
        for (idx, img) in images.iter().enumerate() {
            // 图像在页面上占的面积按像素比 × 页面面积估算:
            // 无法直接拿到绘制变换矩阵时, 用「图像像素尺寸接近整页」作代理判据
            let full_page = page_area > 0.0 && looks_full_page(img.width, img.height, page_area);
            if full_page {
                continue; // 扫描底图, 不是附件
            }
            let filters = img.filters.clone().unwrap_or_default();
            let (ext, data) = if filters.iter().any(|f| f == "DCTDecode") {
                ("jpg", img.content.to_vec())
            } else {
                continue; // 非 JPEG 的插图暂不落盘 (Flate 需重编码, 见 ocr 通道)
            };
            if data.is_empty() {
                continue;
            }
            let fname = format!("page-{:04}-{}.{}", page_no, idx + 1, ext);
            let target = dir.join(&fname);
            std::fs::write(&target, &data)?;
            out.push(ExtractedAsset {
                rel: rel_path(rel_base, &target),
                file: target,
                origin: format!("page-{}", page_no),
            });
        }
    }
    Ok(out)
}

/// 页面 MediaBox 面积 (pt²)
fn page_media_box_area(doc: &lopdf::Document, page_id: lopdf::ObjectId) -> Option<f64> {
    let dict = doc.get_dictionary(page_id).ok()?;
    let arr = dict.get(b"MediaBox").ok()?.as_array().ok()?;
    if arr.len() != 4 {
        return None;
    }
    let v: Vec<f64> = arr
        .iter()
        .filter_map(|o| {
            o.as_float()
                .ok()
                .map(f64::from)
                .or_else(|| o.as_i64().ok().map(|i| i as f64))
        })
        .collect();
    if v.len() != 4 {
        return None;
    }
    Some((v[2] - v[0]).abs() * (v[3] - v[1]).abs())
}

/// 判断图像是否为整页底图
///
/// 图像像素尺寸无法直接换算成 pt (PDF 允许任意缩放), 这里用经验判据:
/// 像素短边与页面短边 (pt) 的比值过大 → 典型的扫描页 (300dpi 扫描的 A4
/// 约 2450×3500 像素, 页面 595×842pt, 比值约 4.1)。
fn looks_full_page(width: i64, height: i64, page_area: f64) -> bool {
    let short_px = width.min(height) as f64;
    let short_pt = page_area.sqrt().min(842.0);
    if short_pt <= 0.0 {
        return false;
    }
    let ratio = short_px / short_pt;
    // 像素/点比值 > 2.5 表示该图以远高于页面自身的分辨率覆盖整页
    // (150dpi≈2.08, 200dpi≈2.78, 300dpi≈4.17); 取 2.5 覆盖 200/300dpi 扫描件
    ratio > 2.5
}

// ── Markdown 引用重写 ──────────────────────────────────────────────────

/// 重写 Markdown 中的图片引用, 指向实际抽取的附件
///
/// 返回 (新 Markdown, 已替换数, 无法还原数)。
///
/// **仅在引用数与附件数完全相等时按顺序映射**。数量不等说明抽取到的图与正文
/// 引用对不上 (典型: 正文引用在文末, 而抽到的都是封面小图), 此时按顺序硬配
/// 会把 A 图放到 B 的位置 —— 静默的内容错配比缺失更糟。故一律标记为缺失并
/// 保留附件在 assets/ 中供人工核对。
pub fn rewrite_refs(md: &str, assets: &[ExtractedAsset]) -> (String, usize, usize) {
    let local_refs = count_local_refs(md);
    let strict_match = local_refs == assets.len();
    rewrite_refs_inner(md, assets, strict_match)
}

/// 统计需要本地化的图片引用数 (排除远程/内联)
fn count_local_refs(md: &str) -> usize {
    let mut rest = md;
    let mut n = 0;
    while let Some((_, _, target, end)) = next_image_ref(rest) {
        if !(target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with("data:"))
        {
            n += 1;
        }
        rest = &rest[end..];
    }
    n
}

fn rewrite_refs_inner(
    md: &str,
    assets: &[ExtractedAsset],
    map_in_order: bool,
) -> (String, usize, usize) {
    let mut out = String::with_capacity(md.len());
    let mut idx = 0usize;
    let mut replaced = 0usize;
    let mut unresolved = 0usize;
    let mut rest = md;

    while let Some((start, alt, target, end)) = next_image_ref(rest) {
        out.push_str(&rest[..start]);
        // 已有可用的远程/内联引用保持不变
        let keep = target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with("data:");
        if keep {
            out.push_str(&rest[start..end]);
        } else if map_in_order {
            let asset = &assets[idx];
            out.push_str(&format!("![{}]({})", alt, asset.rel));
            replaced += 1;
            idx += 1;
        } else {
            out.push_str(&format!(
                "<!-- qyzs:missing-asset {} -->\n_(图片附件未能还原, 原始引用: {})_",
                target, target
            ));
            unresolved += 1;
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    (out, replaced, unresolved)
}

/// 定位下一个图片引用, 返回 (起始, alt, 目标, 结束)
fn next_image_ref(text: &str) -> Option<(usize, String, String, usize)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] != b'!' || bytes[i + 1] != b'[' {
            i += 1;
            continue;
        }
        let alt_start = i + 2;
        let alt_end_rel = text[alt_start..].find(']')?;
        let alt_end = alt_start + alt_end_rel;
        if alt_end + 1 >= bytes.len() || bytes[alt_end + 1] != b'(' {
            i = alt_end + 1;
            continue;
        }
        let target_start = alt_end + 2;
        let target_end_rel = text[target_start..].find(')')?;
        let target_end = target_start + target_end_rel;
        let target = text[target_start..target_end].trim().to_string();
        if target.is_empty() || target.contains(char::is_whitespace) {
            i = target_end + 1;
            continue;
        }
        let alt = text[alt_start..alt_end].to_string();
        return Some((i, alt, target, target_end + 1));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(rel: &str) -> ExtractedAsset {
        ExtractedAsset {
            file: PathBuf::from(rel),
            rel: rel.to_string(),
            origin: "test".to_string(),
        }
    }

    #[test]
    fn test_rewrite_maps_in_order() {
        let md = "前文\n\n![](images/aaa.jpg)\n\n中间\n\n![](images/bbb.jpg)\n";
        let assets = vec![asset("assets/doc/p1.jpg"), asset("assets/doc/p2.jpg")];
        let (out, replaced, unresolved) = rewrite_refs(md, &assets);
        assert_eq!(replaced, 2);
        assert_eq!(unresolved, 0);
        assert!(out.contains("assets/doc/p1.jpg"));
        assert!(out.contains("assets/doc/p2.jpg"));
        assert!(!out.contains("images/aaa.jpg"));
    }

    #[test]
    fn test_rewrite_marks_missing_instead_of_dead_link() {
        // 引用 2 处 / 附件 1 个: 数量不等 → 全部标缺失, 不做错配映射
        let md = "![](images/aaa.jpg)\n\n![](images/bbb.jpg)";
        let assets = vec![asset("assets/doc/p1.jpg")];
        let (out, replaced, unresolved) = rewrite_refs(md, &assets);
        assert_eq!(replaced, 0, "数量不等时不应硬配");
        assert_eq!(unresolved, 2);
        assert!(out.contains("qyzs:missing-asset"));
        // 缺失的绝不保留死链
        assert!(!out.contains("![](images/bbb.jpg)"));
    }

    #[test]
    fn test_remote_refs_untouched() {
        let md = "![x](https://example.com/a.png) ![y](data:image/png;base64,AAA)";
        let (out, replaced, unresolved) = rewrite_refs(md, &[]);
        assert_eq!(replaced, 0);
        assert_eq!(unresolved, 0);
        assert!(out.contains("https://example.com/a.png"));
        assert!(out.contains("data:image/png"));
    }

    #[test]
    fn test_no_refs_is_identity() {
        let md = "# 标题\n\n纯文本, 无图片。";
        let (out, r, u) = rewrite_refs(md, &[]);
        assert_eq!(out, md);
        assert_eq!((r, u), (0, 0));
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize("N104_地理信息指南"), "N104_地理信息指南");
        assert_eq!(sanitize("a/b c"), "a_b_c");
        assert_eq!(sanitize("///"), "document");
    }

    #[test]
    fn test_full_page_scan_detected() {
        // 300dpi 扫描 A4: 2455×3508 像素 vs 595×842pt 页面
        assert!(looks_full_page(2455, 3508, 595.0 * 842.0));
        // 页内小插图 (600×400) 不应判定为整页
        assert!(!looks_full_page(600, 400, 595.0 * 842.0));
    }

    #[test]
    fn test_text_pdf_has_no_assets() {
        let dir = tempfile::tempdir().expect("临时目录");
        let assets = extract(
            "tests/data/mineru/text_pdf/REF-0056_政府采购评审专家管理办法.pdf",
            &SourceFormat::Pdf,
            dir.path(),
            "doc",
            dir.path(),
        )
        .expect("抽取应成功");
        assert!(assets.is_empty(), "无插图的文本层 PDF 不应产出附件");
    }
}
