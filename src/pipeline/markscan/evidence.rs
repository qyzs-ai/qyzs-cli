//! 第二通道证据构建 (按 pdf_probe 逐页分类路由证据来源)
//!
//! - Text 页: 文本层直接作证据 (零模型调用, 来自 probe 的单次加载)
//! - Scan 页: 页图 OCR (角色兜底链) + VL 版面理解双通道
//! - Opaque 页: 无本地证据, 告警说明
//!
//! 混合文档逐页取源; 证据按页组织, 支撑级联验证的页锚定。

use super::pdf_ocr;
use super::pdf_probe::PdfProbe;
use super::workspace::Workspace;
use crate::client::QyzsClient;
use crate::error::QyzsResult;

/// 按页组织的第二通道证据
pub struct Evidence {
    /// (页号, 该页证据文本); 页号 0 表示锚定退化 (续跑恢复的整块文本)
    pub pages: Vec<(u32, String)>,
    pub warnings: Vec<String>,
}

impl Evidence {
    pub fn total_chars(&self) -> usize {
        self.pages
            .iter()
            .map(|(_, t)| t.trim().chars().count())
            .sum()
    }

    /// 合并为单块文本 (manifest 持久化与降级诊断用)
    pub fn merged(&self) -> String {
        let mut out = String::new();
        for (pno, t) in &self.pages {
            if t.trim().is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            if *pno > 0 {
                out.push_str(&format!("[第{pno}页]\n"));
            }
            out.push_str(t);
            out.push('\n');
        }
        out
    }
}

/// 构建 PDF 第二通道证据 (manifest 阶段 "secondary", 支持续跑复用)
pub async fn build(
    client: &QyzsClient,
    ws: &mut Workspace,
    probe: &PdfProbe,
    path: &str,
    max_dim: Option<u32>,
    on_progress: Option<&dyn Fn(usize, usize)>,
) -> QyzsResult<Evidence> {
    if ws.stage_done("secondary", "extract") {
        let text = ws.read_segment_text("secondary")?;
        return Ok(Evidence {
            pages: vec![(0, text)],
            warnings: Vec::new(),
        });
    }

    let mut warnings = Vec::new();
    // Text 页: 文本层证据 (probe 已在单次加载中取好)
    let mut pages: Vec<(u32, String)> = probe
        .text_pages()
        .into_iter()
        .map(|(p, t)| (p, format!("[文本层]\n{t}")))
        .collect();

    // Scan 页: 页图 OCR + VL 双通道
    let scan_set = probe.scan_pages();
    if !scan_set.is_empty() {
        let dir = ws.root.join("secondary-images");
        let all_imgs = pdf_ocr::extract_page_images(path, &dir)?;
        let imgs: Vec<_> = all_imgs
            .into_iter()
            .filter(|i| scan_set.contains(&i.page))
            .collect();
        if imgs.is_empty() {
            warnings.push(format!(
                "{} 个扫描页未能提取页图 (矢量/JPX/CCITT 等压缩), 这些页无本地证据",
                scan_set.len()
            ));
        } else {
            let first_image = imgs[0].path.to_string_lossy().to_string();
            let (ocr_preflight, vl_preflight) = tokio::join!(
                super::model_fallback::preflight_vision_chain(
                    client,
                    crate::msg!(MdAiStageScanOcr),
                    super::model_fallback::ROLE_OCR,
                    &first_image,
                    max_dim,
                ),
                super::model_fallback::preflight_vision_chain(
                    client,
                    crate::msg!(MdAiStageScanVl),
                    super::model_fallback::ROLE_VL,
                    &first_image,
                    max_dim,
                ),
            );
            for (role, result) in [("OCR", ocr_preflight), ("VL", vl_preflight)] {
                if let Err(error) = result {
                    warnings.push(format!("{role} 模型预检全链失败: {error}"));
                }
            }
            let (ocr_texts, ocr_warns) =
                pdf_ocr::ocr_pages(client, &imgs, max_dim, on_progress).await;
            let (vl_texts, vl_warns) = pdf_ocr::vl_pages(client, &imgs, max_dim, on_progress).await;
            warnings.extend(ocr_warns);
            warnings.extend(vl_warns);
            for (i, img) in imgs.iter().enumerate() {
                let mut t = String::new();
                if let Some(o) = ocr_texts.get(i).and_then(|x| x.as_ref()) {
                    t.push_str(&format!("[文字转录]\n{o}\n"));
                }
                if let Some(v) = vl_texts.get(i).and_then(|x| x.as_ref()) {
                    t.push_str(&format!("[图像理解]\n{v}"));
                }
                if !t.trim().is_empty() {
                    pages.push((img.page, t));
                }
            }
        }
    }

    pages.sort_by_key(|(p, _)| *p);
    let ev = Evidence { pages, warnings };
    ws.write_segment_text("secondary", &ev.merged())?;
    ws.complete_stage("secondary", "extract", &[])?;
    Ok(ev)
}
