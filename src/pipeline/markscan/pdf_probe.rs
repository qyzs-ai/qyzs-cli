//! PDF 单次加载逐页探测: 页类型分类与路由决策
//!
//! 一次 `Document::load` 完成全部探测 (替代此前 metadata/文本层/整页图/样式/
//! 资产各自独立加载的多遍解析), 产出:
//! - 每页类型: Text (数字文本层充足) / Scan (文本层缺失且含整页扫描图) /
//!   Opaque (两者皆无 —— 矢量扫描、JPX/CCITT 压缩等, 本地无法取证据)
//! - 文档级路由: MinerU 的 is_ocr (任一非 Text 页即需要 OCR 辅助)
//! - 证据计划: Text 页直接用文本层, Scan 页走页图 OCR/VL, Opaque 页无本地证据
//!
//! 混合文档 (数字正文 + 扫描插页) 由逐页分类自然覆盖。

use super::assets::{looks_full_page, page_media_box_area};
use super::reflow;
use crate::error::{QyzsError, QyzsResult};
use lopdf::Document;
use std::collections::BTreeSet;

/// 单页文本层下限 (判为可用文本页的最低字符数)
///
/// 不用 200: 落款页/附页常只有一百余字符 (2026-10-26 N078 实测第 2 页 111 字符,
/// 正是落款与日期所在), 按 200 判会被误弃为 Opaque 而失去验证依据。50 字符仍能
/// 滤掉扫描页上残留的零星字符。文档级证据充足性另有 200 字符地板 (见 rigor)。
const MIN_TEXT_CHARS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    /// 数字文本层充足, 直接用文本层证据
    Text,
    /// 文本层缺失但含整页扫描图 → 页图 OCR/VL 证据
    Scan,
    /// 无文本层也无整页图 (矢量/JPX/CCITT 等), 本地无独立证据来源
    Opaque,
}

/// 竖向碎片重组: 把逐字换行的文本层还原为正常文本
///
/// 部分 PDF 的内容流逐字定位 (每个字形单独 Tj), lopdf 抽出的文本层便是
/// `云\n南\n省\n发\n...` 这样的碎片 (2026-10-26 N078 实测)。碎片文本做
/// 行级 / 数字串级比对会全假阳性, 故在入证据前重组: 连续短行归并回前一行
/// (前一行无终结标点时), 正常文档原样返回。
fn rejoin_vertical(text: &str) -> String {
    let lines: Vec<String> = text
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    // 碎片判定: 存在 3 行以上连续短行 (≤4 字符) 的游程
    //
    // 不用「半数短行」: 碎片页常是混合型 —— 多数为正常行, 仅落款/日期一段
    // 被逐字拆开 (2026-10-26 N078 第 2 页: 正常行 + `2025/年/12/月/15/日`
    // 六连短行)。连续游程才是碎片的确凿信号, 正常文档罕见三连短行。
    let mut run = 0usize;
    let mut fragmented = false;
    for l in &lines {
        if l.chars().count() <= 4 {
            run += 1;
            if run >= 3 {
                fragmented = true;
                break;
            }
        } else {
            run = 0;
        }
    }
    if !fragmented {
        return text.to_string(); // 非碎片, 原样
    }
    let mut out: Vec<String> = Vec::new();
    for l in lines {
        let is_short = l.chars().count() <= 4;
        if let Some(prev) = out.last_mut() {
            let joinable =
                is_short && !reflow::ends_terminal(prev) && !reflow::starts_structural(&l);
            if joinable {
                prev.push_str(&l);
                continue;
            }
        }
        out.push(l);
    }
    out.join("\n")
}

#[derive(Debug, Clone)]
pub struct PageInfo {
    pub page: u32,
    pub kind: PageKind,
    /// 该页文本层内容 (Scan/Opaque 页可能为空)
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct PdfProbe {
    pub pages: Vec<PageInfo>,
}

impl PdfProbe {
    /// 单次加载完成逐页探测
    pub fn probe(path: &str) -> QyzsResult<Self> {
        let doc =
            Document::load(path).map_err(|e| QyzsError::Pdf(format!("加载 PDF 失败: {e}")))?;
        let mut pages = Vec::new();
        for (no, id) in doc.get_pages() {
            let raw_text = doc.extract_text(&[no]).unwrap_or_default();
            let text = rejoin_vertical(&raw_text);
            let chars = text.trim().chars().count();
            let kind = if chars >= MIN_TEXT_CHARS {
                PageKind::Text
            } else {
                let area = page_media_box_area(&doc, id).unwrap_or(0.0);
                let has_fullpage = doc
                    .get_page_images(id)
                    .map(|imgs| {
                        imgs.iter()
                            .any(|i| area > 0.0 && looks_full_page(i.width, i.height, area))
                    })
                    .unwrap_or(false);
                if has_fullpage {
                    PageKind::Scan
                } else {
                    PageKind::Opaque
                }
            };
            pages.push(PageInfo {
                page: no,
                kind,
                text,
            });
        }
        Ok(Self { pages })
    }

    /// MinerU 是否需要 OCR 辅助: 任一页不是充足文本层即需要
    /// (数字件走 is_ocr=false 的 txt 路径更快且避免再识别误差, 2026-10-26 实测)
    pub fn mineru_is_ocr(&self) -> bool {
        self.pages.iter().any(|p| p.kind != PageKind::Text)
    }

    /// Text 页的文本层证据 (页号, 文本)
    pub fn text_pages(&self) -> Vec<(u32, String)> {
        self.pages
            .iter()
            .filter(|p| p.kind == PageKind::Text && !p.text.trim().is_empty())
            .map(|p| (p.page, p.text.trim().to_string()))
            .collect()
    }

    /// Scan 页页号集合 (页图 OCR/VL 证据的提取范围)
    pub fn scan_pages(&self) -> BTreeSet<u32> {
        self.pages
            .iter()
            .filter(|p| p.kind == PageKind::Scan)
            .map(|p| p.page)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(rel: &str) -> String {
        format!("{}/{}", env!("CARGO_MANIFEST_DIR"), rel)
    }

    /// REF-0056: 10 页数字文本件 → 全部 Text, is_ocr=false
    #[test]
    fn test_probe_text_pdf() {
        let p = PdfProbe::probe(&fixture(
            "tests/data/mineru/text_pdf/REF-0056_政府采购评审专家管理办法.pdf",
        ))
        .unwrap();
        assert_eq!(p.pages.len(), 10);
        assert!(p.pages.iter().all(|x| x.kind == PageKind::Text));
        assert!(!p.mineru_is_ocr(), "纯文本件不需要 MinerU OCR");
        assert_eq!(p.text_pages().len(), 10);
        assert!(p.scan_pages().is_empty());
    }

    /// REF-0101: 10 页扫描件 → 全部 Scan
    #[test]
    fn test_probe_scanned_pdf() {
        let p = PdfProbe::probe(&fixture(
            "tests/data/mineru/scanned_pdf/REF-0101_网络安全等级保护指导意见_scan.pdf",
        ))
        .unwrap();
        assert_eq!(p.pages.len(), 10);
        assert!(p.pages.iter().all(|x| x.kind == PageKind::Scan));
        assert!(p.mineru_is_ocr());
        assert_eq!(p.scan_pages().len(), 10);
    }

    /// N078: 混合页型 —— 第 1 页 488 字符 (Text), 第 2 页 111 字符 (Text, 落款页)
    /// 第 2 页若按 200 字符地板会被误弃为 Opaque, 落款日期便失去验证依据
    #[test]
    fn test_probe_mixed_short_tail_page() {
        let path = fixture(
            "tmp/corpus/N078_云南省发展和改革委员会关于召开《云南省全面实施人工智能行动计划》贯彻落实座谈会的函.pdf",
        );
        if !std::path::Path::new(&path).exists() {
            return; // 语料不在 (CI 环境) 时跳过
        }
        let p = PdfProbe::probe(&path).unwrap();
        assert_eq!(p.pages.len(), 2);
        assert!(
            p.pages.iter().all(|x| x.kind == PageKind::Text),
            "两页均为文本页"
        );
        assert_eq!(p.text_pages().len(), 2);
        // 落款日期必须在证据里
        let ev = p
            .text_pages()
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<String>();
        assert!(ev.contains("2025"), "落款日期应进入证据: {ev}");
    }

    /// 竖向碎片重组: 逐字换行的文本层还原为正常行
    #[test]
    fn test_rejoin_vertical() {
        let fragmented = "云\n南\n省\n发\n展\n和\n改\n革\n委\n员\n会\n2025\n年\n12\n月\n15\n日";
        let out = rejoin_vertical(fragmented);
        assert!(out.contains("云南省发展和改革委员会"), "{out}");
        assert!(out.contains("2025年12月15日"), "{out}");
        assert!(!out.contains("\n云\n"), "碎片应被合并");
    }

    /// 混合型碎片页: 多数正常行 + 一段连续短行 → 仅重组该段
    #[test]
    fn test_rejoin_vertical_mixed_page() {
        let mixed = "此函。\n云南省发展和改革委员会\n2025\n年\n12\n月\n15\n日\n（联系人及电话：郑秋秋\n0871-63113902\n、\n18987420580\n）";
        let out = rejoin_vertical(mixed);
        assert!(out.contains("2025年12月15日"), "落款日期应重组: {out}");
        assert!(out.contains("0871-63113902、"), "分隔符应归并: {out}");
        assert!(out.contains("此函。"), "正常行应保留");
    }

    /// 正常文本不被重组
    #[test]
    fn test_rejoin_vertical_keeps_normal_text() {
        let normal = "第一条 内容。\n\n第二条 内容。\n";
        assert_eq!(rejoin_vertical(normal), normal);
    }

    /// N027: 无文本层也无整页图 (矢量/其他压缩) → Opaque, is_ocr=true
    #[test]
    fn test_probe_opaque_pdf() {
        let p = PdfProbe::probe(&fixture(
            "tmp/corpus/N027_关于同意《云南省自然资源管理和国土空间规划一张图2026年实施方案》的函.pdf",
        ))
        .unwrap_or_else(|e| {
            if std::path::Path::new(&fixture("tmp/corpus")).exists() {
                panic!("语料存在时探测不应失败: {e}")
            } else {
                // 语料不在 (CI 环境) 时跳过
                std::process::exit(0);
            }
        });
        assert!(p.pages.iter().all(|x| x.kind == PageKind::Opaque));
        assert!(p.mineru_is_ocr(), "无文本层页仍需 MinerU OCR");
    }
}
