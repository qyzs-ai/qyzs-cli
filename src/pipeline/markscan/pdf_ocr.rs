//! 扫描件 PDF 的 OCR 第二通道 (rigor 档交叉验证的证据来源)
//!
//! 背景 (2026-10-20 实测): rigor 档的第二通道是 PDF 文本层, 扫描件文本层为空,
//! 交叉验证从未真正运行, 只能诚实降级为仅润色行为。本模块把第二通道改为
//! 「提取页面内嵌整页图 → 逐页双模型转录」, 与主通道 (MinerU, 内部 OCR) 构成
//! 独立多模型证据:
//! - paddleocrvl: 文字逐行转录 (公文 OCR 专线, 按能力矩阵选择安全尺寸)
//! - qwen3-VL-30B-A3B: 图像理解 —— 逐字转录 + 表格结构 + 印章/图示/版面要素
//!   (带图请求由客户端自动选路, 见 client/chat.rs 模型决策)
//!
//! 提取是纯 Rust 的: 典型扫描件每页恰好一张整页图 XObject, lopdf 直接取出
//! 原始流字节, 无需栅格化渲染、零新增依赖。
//!
//! 按页独立处理, 单页/单通道失败跳过并告警 (与 PDF 切片降级同一契约):
//! - DCTDecode → 原始 JPEG 字节直接落盘 (零重编码)
//! - FlateDecode + 8bit 灰度/RGB → image crate 重编码 PNG
//! - 其他滤镜 (JPX/CCITT/CMYK 等) → 跳过该页

use super::model_fallback;
use crate::client::QyzsClient;
use crate::error::{QyzsError, QyzsResult};
use futures_util::StreamExt;
use lopdf::Document;
use std::path::{Path, PathBuf};

/// 每通道并发上限 (网关多路并发实测稳定; 不加约束易触发限流)
const CHANNEL_CONCURRENCY: usize = 4;

/// VL 逐页转录输出预算 (密集公文页逐字转录 + 表格结构; 准确率优先不设惜)
const VL_PAGE_MAX_TOKENS: u32 = 8192;

/// 单页提取结果
#[derive(Debug, Clone)]
pub struct PageImage {
    /// 1-indexed 页码
    pub page: u32,
    /// 落盘的图片文件 (JPEG/PNG)
    pub path: PathBuf,
}

/// 单页双通道证据 (任一通道失败为 None, 由 merge 跳过)
#[derive(Debug, Clone, Default)]
pub struct PageEvidence {
    pub page: u32,
    /// paddleocrvl 文字转录
    pub ocr: Option<String>,
    /// qwen3-VL 图像理解 (逐字 + 表格 + 版面要素)
    pub vl: Option<String>,
}

impl PageEvidence {
    /// 该页是否至少有一个通道产出
    pub fn is_empty(&self) -> bool {
        self.ocr.is_none() && self.vl.is_none()
    }
}

/// VL 逐页转录提示词: 逐字优先 + 版面结构显式化 (排版验证的证据基础)
const VL_PAGE_PROMPT: &str =
    "你是公文版面转录引擎。逐字转录这张页面图片中的全部内容, 并显式保留版面结构:\n\
1. 标题与章节编号按层级输出;\n\
2. 表格用 Markdown 表格逐格转录, 不许省略行列;\n\
3. 印章、签名、日期、页码、文号等要素单独标注 (格式: 〔印章〕/〔文号〕...);\n\
4. 插图/图形用一句话描述内容与位置;\n\
5. 按页面阅读顺序输出。\n\
只输出转录结果, 不要输出思考过程或解释。";

/// 从扫描件提取每页的内嵌整页图
///
/// 每页取面积最大的位图 XObject (扫描件通常每页只有一张; 少数页面含页眉
/// logo 等小图时, 整页正文图必然面积最大)。产出写入 `out_dir`。
pub fn extract_page_images(pdf_path: &str, out_dir: &Path) -> QyzsResult<Vec<PageImage>> {
    let doc =
        Document::load(pdf_path).map_err(|e| QyzsError::Pdf(format!("加载 PDF 失败: {}", e)))?;
    std::fs::create_dir_all(out_dir)?;

    let mut out = Vec::new();
    for (page_no, page_id) in doc.get_pages() {
        let images = match doc.get_page_images(page_id) {
            Ok(imgs) => imgs,
            Err(_) => continue, // 单页资源异常: 跳过, 不中断整文档
        };
        // 取面积最大的图
        let best = images
            .into_iter()
            .max_by_key(|img| img.width.saturating_mul(img.height));
        let Some(img) = best else { continue };

        let filters = img.filters.clone().unwrap_or_default();
        let stem = out_dir.join(format!("page-{:04}", page_no));
        let result = if filters.iter().any(|f| f == "DCTDecode") {
            // JPEG 原始字节直接落盘
            let path = stem.with_extension("jpg");
            std::fs::write(&path, img.content)
                .map_err(|e| QyzsError::Io(format!("写页图失败: {}", e)))?;
            Ok(path)
        } else if filters.iter().any(|f| f == "FlateDecode") {
            encode_flate_image(img, &stem)
        } else {
            Err(())
        };
        if let Ok(path) = result {
            out.push(PageImage {
                page: page_no,
                path,
            });
        }
    }
    Ok(out)
}

/// FlateDecode 原始采样重编码 (仅支持 8bit 灰度/RGB; 其他色域返回 Err 跳过)
fn encode_flate_image(img: lopdf::xobject::PdfImage<'_>, stem: &Path) -> Result<PathBuf, ()> {
    let channels: usize = match img.color_space.as_deref() {
        Some("DeviceGray") => 1,
        Some("DeviceRGB") => 3,
        _ => return Err(()),
    };
    if img.bits_per_component != Some(8) {
        return Err(());
    }
    let (w, h) = (img.width as u32, img.height as u32);
    let expected = w as usize * h as usize * channels;
    if img.content.len() < expected {
        return Err(());
    }
    let buf: Vec<u8> = img.content[..expected].to_vec();
    let image = match channels {
        1 => image::GrayImage::from_raw(w, h, buf).map(image::DynamicImage::ImageLuma8),
        _ => image::RgbImage::from_raw(w, h, buf).map(image::DynamicImage::ImageRgb8),
    };
    let Some(image) = image else { return Err(()) };
    let path = stem.with_extension("png");
    image
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(|_| ())?;
    Ok(path)
}

/// 有界并发跑单页异步任务, 返回与输入等长的结果序列 (失败位为 None)
async fn run_bounded<T, O, F, Fut>(
    items: Vec<(usize, T)>,
    concurrency: usize,
    mut f: F,
    on_progress: Option<&dyn Fn(usize, usize)>,
) -> Vec<Option<O>>
where
    T: Clone,
    F: FnMut(T) -> Fut,
    Fut: std::future::Future<Output = QyzsResult<O>>,
{
    let total = items.len();
    let mut out: Vec<Option<O>> = (0..total).map(|_| None).collect();
    let mut stream = futures_util::stream::iter(items)
        .map(|(idx, item)| {
            let fut = f(item);
            async move { (idx, fut.await) }
        })
        .buffered(concurrency);
    let mut done = 0usize;
    while let Some((idx, resp)) = stream.next().await {
        if let Ok(v) = resp {
            out[idx] = Some(v);
        }
        done += 1;
        if let Some(progress) = on_progress {
            progress(done, total);
        }
    }
    out
}

/// 逐页文字转录 (角色兜底链: paddleocrvl → qwen36 → qwen3-VL, 有界并发 4 路)
///
/// 专用 OCR 不可用时按链切换多模态 chat 模型, 而非丢弃该证据通道;
/// 链头熔断时直接跳到下一候选 (下线模型不再被反复调用)。
pub async fn ocr_pages(
    client: &QyzsClient,
    pages: &[PageImage],
    max_dim: Option<u32>,
    on_progress: Option<&dyn Fn(usize, usize)>,
) -> (Vec<Option<String>>, Vec<String>) {
    const CHAT_OCR_PROMPT: &str =
        "逐字转录这张图片中的全部文字内容, 保留段落与标题层级。只输出转录文本, 不要解释。";
    let items: Vec<_> = pages
        .iter()
        .enumerate()
        .map(|(i, p)| (i, p.clone()))
        .collect();
    let results = run_bounded(
        items,
        CHANNEL_CONCURRENCY,
        |page: PageImage| {
            let path = page.path.to_string_lossy().to_string();
            async move {
                let (_, text) = model_fallback::with_fallback(model_fallback::ROLE_OCR, |m| {
                    let m = m.to_string();
                    let path = path.clone();
                    async move {
                        match m.as_str() {
                            "paddleocrvl" => client
                                .ocr_with_model(&path, None, max_dim, &m)
                                .await
                                .map(|r| r.text.trim().to_string())
                                .and_then(|t| {
                                    if t.is_empty() {
                                        Err(QyzsError::Server("OCR 返回空内容".to_string()))
                                    } else {
                                        Ok(t)
                                    }
                                }),
                            _ => {
                                model_fallback::transcribe_via_chat(
                                    client,
                                    &m,
                                    &path,
                                    CHAT_OCR_PROMPT,
                                    VL_PAGE_MAX_TOKENS,
                                    max_dim,
                                )
                                .await
                            }
                        }
                    }
                })
                .await?;
                Ok(text)
            }
        },
        on_progress,
    )
    .await;

    let warnings = collect_page_warnings("OCR", pages, &results);
    (results, warnings)
}

/// 逐页版面视觉理解 (角色兜底链: qwen3-VL → qwen36, 有界并发 4 路)
pub async fn vl_pages(
    client: &QyzsClient,
    pages: &[PageImage],
    max_dim: Option<u32>,
    on_progress: Option<&dyn Fn(usize, usize)>,
) -> (Vec<Option<String>>, Vec<String>) {
    let items: Vec<_> = pages
        .iter()
        .enumerate()
        .map(|(i, p)| (i, p.clone()))
        .collect();
    let results = run_bounded(
        items,
        CHANNEL_CONCURRENCY,
        |page: PageImage| {
            let path = page.path.to_string_lossy().to_string();
            async move {
                let (_, text) = model_fallback::with_fallback(model_fallback::ROLE_VL, |m| {
                    let m = m.to_string();
                    let path = path.clone();
                    async move {
                        model_fallback::transcribe_via_chat(
                            client,
                            &m,
                            &path,
                            VL_PAGE_PROMPT,
                            VL_PAGE_MAX_TOKENS,
                            max_dim,
                        )
                        .await
                    }
                })
                .await?;
                Ok(text)
            }
        },
        on_progress,
    )
    .await;

    let warnings = collect_page_warnings("VL 图像理解", pages, &results);
    (results, warnings)
}

/// 汇总逐页失败告警
fn collect_page_warnings(
    channel: &str,
    pages: &[PageImage],
    results: &[Option<String>],
) -> Vec<String> {
    results
        .iter()
        .zip(pages)
        .filter(|(r, _)| r.is_none())
        .map(|(_, p)| format!("第 {} 页 {channel} 失败, 该页该通道证据缺失", p.page))
        .collect()
}

/// 按页合并双通道证据 (供交叉验证的 chunk_secondary 切块)
///
/// 页内先 OCR 后 VL; 单通道缺失只留另一通道; 整页双缺则整页跳过。
pub fn merge_page_evidence(pages: &[PageEvidence]) -> String {
    let mut merged = String::new();
    for ev in pages {
        if ev.is_empty() {
            continue;
        }
        if !merged.is_empty() {
            merged.push_str("\n\n");
        }
        merged.push_str(&format!("[第{}页]\n", ev.page));
        if let Some(ocr) = &ev.ocr {
            merged.push_str("[文字转录]\n");
            merged.push_str(ocr);
            merged.push('\n');
        }
        if let Some(vl) = &ev.vl {
            merged.push_str("[图像理解]\n");
            merged.push_str(vl);
            merged.push('\n');
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实扫描件: 每页一张 DCTDecode JPEG, 全部页都应抽出非空 JPEG
    #[test]
    fn test_extract_from_real_scanned_pdf() {
        let dir = tempfile::tempdir().expect("临时目录");
        let pages = extract_page_images(
            "tests/data/mineru/scanned_pdf/REF-0101_网络安全等级保护指导意见_scan.pdf",
            dir.path(),
        )
        .expect("提取应成功");
        assert_eq!(pages.len(), 10, "10 页扫描件应抽出 10 张页图");
        for (i, p) in pages.iter().enumerate() {
            assert_eq!(p.page, (i + 1) as u32, "页码应连续");
            let data = std::fs::read(&p.path).expect("页图应存在");
            assert!(data.len() > 1000, "页图不应为空: {:?}", p.path);
            // JPEG magic (DCTDecode 流即为 JPEG 文件字节)
            assert_eq!(
                &data[..2],
                b"\xff\xd8",
                "DCTDecode 提取应为合法 JPEG: {:?}",
                p.path
            );
        }
    }

    /// 纯文本层 PDF (非扫描件) 没有内嵌整页图: 返回空列表, 由调用方走降级
    #[test]
    fn test_text_pdf_yields_no_images() {
        let dir = tempfile::tempdir().expect("临时目录");
        let pages = extract_page_images(
            "tests/data/mineru/text_pdf/REF-0056_政府采购评审专家管理办法.pdf",
            dir.path(),
        )
        .expect("提取应成功 (无图不报错)");
        assert!(pages.is_empty(), "文本层 PDF 不应抽出页图");
    }

    /// 证据合并: 双通道按页拼接, 缺失通道跳过, 整页双缺整页跳过
    #[test]
    fn test_merge_page_evidence() {
        let evs = vec![
            PageEvidence {
                page: 1,
                ocr: Some("甲".into()),
                vl: Some("乙".into()),
            },
            PageEvidence {
                page: 2,
                ocr: None,
                vl: Some("丙".into()),
            },
            PageEvidence {
                page: 3,
                ocr: Some("丁".into()),
                vl: None,
            },
            PageEvidence {
                page: 4,
                ..Default::default()
            },
        ];
        let m = merge_page_evidence(&evs);
        assert!(m.contains("[第1页]"));
        assert!(m.contains("[文字转录]\n甲"));
        assert!(m.contains("[图像理解]\n乙"));
        assert!(m.contains("[第2页]"));
        assert!(m.contains("丙"));
        assert!(!m.contains("[第4页]"), "整页双缺应整页跳过");
        // 顺序: 页内 OCR 在 VL 前
        let p1 = m.split("[第2页]").next().unwrap();
        assert!(p1.find("甲").unwrap() < p1.find("乙").unwrap());
    }

    /// 空 pages 输入不触网
    #[tokio::test]
    async fn test_ocr_pages_empty_input() {
        let client = crate::client::QyzsClient::new(test_config()).expect("客户端构造");
        let (results, warnings) = ocr_pages(&client, &[], None, None).await;
        assert!(results.is_empty());
        assert!(warnings.is_empty());
    }

    fn test_config() -> crate::config::ResolvedConfig {
        crate::config::ResolvedConfig::resolve(
            std::env::var("QYZS_APIKEY").ok().as_deref(),
            None,
            Some(300),
            None,
            None,
            Some("never"),
            None,
        )
        .expect("测试配置解析失败")
    }
}
