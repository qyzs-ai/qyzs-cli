//! markitdown 全能文档转 Markdown 流水线
//!
//! 阶段①: 格式嗅探 + 通道路由 + `--dry-run` 规划预览。
//! 阶段②: speed 档执行 — PDF (本地切片 + MinerU 逐片聚合) 与 doc/docx
//! (MinerU 整文件) 通道，规则清洗只做结构归一，工作区生命周期管理
//! (锁/manifest/流式清理/断点续跑)。
//! 阶段③~⑤ (Office/Visio 本地抽取、balanced 润色、rigor 交叉验证) 分阶段接入。

pub mod clean;
pub mod detect;
pub mod pdf;
pub mod workspace;

use crate::client::QyzsClient;
use crate::error::{QyzsError, QyzsResult};
use crate::pdf::PdfExtractor;
use detect::{DetectMethod, Detection, FormatDetector, SourceFormat};
use futures_util::StreamExt;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;

/// URL 输入下载大小上限
const MAX_DOWNLOAD_BYTES: u64 = 200 * 1024 * 1024;

/// MinerU 同步通道实测吞吐 (秒/页，capability_matrix 校准真值)
const MINERU_SECONDS_PER_PAGE: f64 = 0.26;

/// 处理档位
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineMode {
    Speed,
    Balanced,
    Rigor,
}

impl PipelineMode {
    pub fn parse(s: &str) -> QyzsResult<Self> {
        match s {
            "speed" => Ok(Self::Speed),
            "balanced" => Ok(Self::Balanced),
            "rigor" => Ok(Self::Rigor),
            other => Err(QyzsError::Usage(format!(
                "未知档位 '{}'，可选: speed|balanced|rigor",
                other
            ))),
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Self::Speed => "speed",
            Self::Balanced => "balanced",
            Self::Rigor => "rigor",
        }
    }
}

/// 图像内容处理策略
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageHandling {
    Auto,
    Ocr,
    Describe,
    Both,
    Skip,
}

impl ImageHandling {
    pub fn parse(s: &str) -> QyzsResult<Self> {
        match s {
            "auto" => Ok(Self::Auto),
            "ocr" => Ok(Self::Ocr),
            "describe" => Ok(Self::Describe),
            "both" => Ok(Self::Both),
            "skip" => Ok(Self::Skip),
            other => Err(QyzsError::Usage(format!(
                "未知图像处理策略 '{}'，可选: auto|ocr|describe|both|skip",
                other
            ))),
        }
    }

    pub fn key(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Ocr => "ocr",
            Self::Describe => "describe",
            Self::Both => "both",
            Self::Skip => "skip",
        }
    }
}

/// 子命令参数聚合 (dispatch 层完成字符串解析与校验后构造)
#[derive(Debug, Clone)]
pub struct MarkitdownOptions {
    pub input: String,
    pub output: Option<String>,
    pub mode: PipelineMode,
    pub type_override: Option<SourceFormat>,
    pub image_handling: ImageHandling,
    pub model: Option<String>,
    pub min_confidence: Option<f64>,
    pub report: Option<String>,
    pub work_dir: Option<String>,
    pub segment_chars: usize,
    pub resume: bool,
    pub no_clean: bool,
    pub dry_run: bool,
}

/// dry-run 输出的流水线规划报告
#[derive(Debug, Serialize)]
pub struct ConversionPlan {
    pub input: String,
    /// "file" | "url"
    pub input_source: &'static str,
    /// 格式规范键 (pdf/docx/image/...)
    pub detected_format: String,
    /// 中文格式标签
    pub detected_label: String,
    /// "content-sniff" | "extension-fallback" | "manual-override"
    pub detect_method: &'static str,
    pub size_bytes: u64,
    /// PDF 专属：lopdf 实测页数
    pub page_count: Option<usize>,
    /// PDF 专属：按每批 15 页的切片批数
    pub slice_count: Option<usize>,
    pub mode: &'static str,
    pub polish_model: String,
    pub image_handling: &'static str,
    pub segment_chars: usize,
    pub channel: Option<&'static str>,
    pub supported: bool,
    pub unsupported_reason: Option<String>,
    pub stages: Vec<String>,
    pub warnings: Vec<String>,
    pub output_target: String,
}

pub struct MarkitdownPipeline;

impl MarkitdownPipeline {
    /// 解析输入并产出流水线规划 (dry-run)。
    /// URL 输入先下载到临时文件再统一嗅探；临时文件随本次规划结束删除。
    pub async fn plan(opts: &MarkitdownOptions, timeout_secs: u64) -> QyzsResult<ConversionPlan> {
        let is_url = opts.input.starts_with("http://") || opts.input.starts_with("https://");
        let (_temp_guard, local_path) = if is_url {
            let (dir, path) = Self::download_to_temp(&opts.input, timeout_secs).await?;
            (Some(dir), path)
        } else {
            let path = PathBuf::from(&opts.input);
            if !path.exists() {
                return Err(QyzsError::Usage(format!("输入文件不存在: {}", opts.input)));
            }
            if !path.is_file() {
                return Err(QyzsError::Usage(format!(
                    "输入路径不是常规文件: {}",
                    opts.input
                )));
            }
            (None, path)
        };

        let detection = if let Some(t) = &opts.type_override {
            Detection {
                format: t.clone(),
                method: DetectMethod::ManualOverride,
                warning: None,
            }
        } else {
            FormatDetector::detect_file(&local_path)?
        };

        let size_bytes = std::fs::metadata(&local_path).map(|m| m.len()).unwrap_or(0);

        // PDF 额外产出真实页数 (lopdf)；损坏文件降级为警告而非中断预览
        let mut warnings: Vec<String> = Vec::new();
        let page_count = if detection.format == SourceFormat::Pdf {
            match PdfExtractor::read_metadata(&local_path.to_string_lossy()) {
                Ok(meta) => Some(meta.page_count),
                Err(e) => {
                    warnings.push(format!("PDF 页数读取失败: {}", e));
                    None
                }
            }
        } else {
            None
        };
        let slice_count = page_count.map(pdf::estimate_slices);

        let (channel, unsupported_reason) = Self::route(&detection.format);
        let supported = channel.is_some();

        let polish_model = opts
            .model
            .clone()
            .unwrap_or_else(|| "dsv4".to_string());
        let stages = match (&channel, &unsupported_reason) {
            (Some(ch), _) => Self::stages_for(opts.mode, ch, opts.image_handling, &polish_model),
            _ => Vec::new(),
        };

        // 规模类风险提示
        if let Some(pages) = page_count {
            if pages > 30 {
                warnings.push(format!(
                    "共 {} 页：MinerU 异步轮询通道未实现，将按每批 {} 页同步串行抽取，仅抽取环节预估约 {} 秒",
                    pages,
                    pdf::SLICE_BATCH_PAGES,
                    (pages as f64 * MINERU_SECONDS_PER_PAGE).ceil() as u64
                ));
            }
        }
        if matches!(channel, Some("mineru-slice") | Some("mineru-document"))
            && size_bytes > 10 * 1024 * 1024
        {
            warnings.push(
                "文件体积超过 10 MB：MinerU 大文档异步通道未实现，同步处理耗时可能显著增长"
                    .to_string(),
            );
        }
        if opts.segment_chars > 100_000 {
            warnings.push(format!(
                "--segment-chars {} 超过单次模型调用的安全分段预算，转换实现后将按上限约束",
                opts.segment_chars
            ));
        }
        if let Some(w) = &detection.warning {
            warnings.push(w.clone());
        }

        Ok(ConversionPlan {
            input: opts.input.clone(),
            input_source: if is_url { "url" } else { "file" },
            detected_format: detection.format.key().to_string(),
            detected_label: detection.format.label(),
            detect_method: match detection.method {
                DetectMethod::ContentSniff => "content-sniff",
                DetectMethod::ExtensionFallback => "extension-fallback",
                DetectMethod::ManualOverride => "manual-override",
            },
            size_bytes,
            page_count,
            slice_count,
            mode: opts.mode.key(),
            polish_model,
            image_handling: opts.image_handling.key(),
            segment_chars: opts.segment_chars,
            channel,
            supported,
            unsupported_reason,
            stages,
            warnings,
            output_target: opts.output.clone().unwrap_or_else(|| "stdout".to_string()),
        })
    }

    /// 格式 → 抽取通道路由 (通道实现随阶段②~⑤接入)
    fn route(format: &SourceFormat) -> (Option<&'static str>, Option<String>) {
        match format {
            SourceFormat::Pdf => (Some("mineru-slice"), None),
            SourceFormat::Docx => (Some("mineru-document"), None),
            SourceFormat::Xls | SourceFormat::Xlsx => (Some("sheet-tables"), None),
            SourceFormat::Pptx | SourceFormat::Vsdx => (Some("ooxml-text"), None),
            SourceFormat::Image(_) => (Some("ocr-vl"), None),
            SourceFormat::Html => (Some("html-to-md"), None),
            SourceFormat::Doc => (
                None,
                Some("旧版 Word 二进制格式 (.doc) 不被解析网关接受 (实测 HTTP 400 Unsupported file type)，请先在 Word/WPS 中另存为 .docx".to_string()),
            ),
            SourceFormat::Ppt => (
                None,
                Some("旧版 PowerPoint 二进制格式 (.ppt) 不在支持范围，请先在 PowerPoint/WPS 中另存为 .pptx".to_string()),
            ),
            SourceFormat::Vs => (
                None,
                Some("旧版 Visio 二进制格式 (.vsd) 不在支持范围，请先在 Visio 中另存为 .vsdx".to_string()),
            ),
            SourceFormat::Unknown => (
                None,
                Some("内容嗅探与扩展名均未命中：无法识别输入格式，可用 --type 手动指定".to_string()),
            ),
        }
    }

    /// 按档位展开执行阶段序列
    fn stages_for(
        mode: PipelineMode,
        channel: &str,
        image_handling: ImageHandling,
        polish_model: &str,
    ) -> Vec<String> {
        let extract = Self::extract_stage(channel, image_handling);
        let mut stages = match mode {
            PipelineMode::Speed => {
                vec![extract, "rule-clean".to_string()]
            }
            PipelineMode::Balanced => {
                vec![
                    extract,
                    "light-check".to_string(),
                    format!("polish({})", polish_model),
                ]
            }
            PipelineMode::Rigor => {
                vec![
                    extract,
                    "extract-secondary".to_string(),
                    "cross-verify(bge-m3)".to_string(),
                    "arbitrate(冲突段)".to_string(),
                    format!("polish({})", polish_model),
                    "global-review".to_string(),
                ]
            }
        };
        stages.push("emit".to_string());
        stages
    }

    fn extract_stage(channel: &str, image_handling: ImageHandling) -> String {
        match channel {
            "mineru-slice" => format!("extract(mineru3.2.3, {}页/批切片)", pdf::SLICE_BATCH_PAGES),
            "mineru-document" => "extract(mineru3.2.3)".to_string(),
            "sheet-tables" => "extract(本地表格解析)".to_string(),
            "ooxml-text" => "extract(ooxml 文本抽取)".to_string(),
            "html-to-md" => "extract(html 结构化解析)".to_string(),
            "ocr-vl" => match image_handling {
                ImageHandling::Auto => "extract(paddleocrvl + qwen3-VL 自适应)".to_string(),
                ImageHandling::Ocr => "extract(paddleocrvl)".to_string(),
                ImageHandling::Describe => "extract(qwen3-VL)".to_string(),
                ImageHandling::Both => "extract(paddleocrvl + qwen3-VL)".to_string(),
                ImageHandling::Skip => "extract(跳过图像内容)".to_string(),
            },
            _ => "extract".to_string(),
        }
    }

    /// 下载 URL 输入到临时文件 (强制大小上限；临时目录随返回值生命周期删除)
    async fn download_to_temp(url: &str, timeout_secs: u64) -> QyzsResult<(tempfile::TempDir, PathBuf)> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .user_agent(concat!("qyzs-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| QyzsError::Network(e.to_string()))?;

        let resp = client
            .get(url)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| QyzsError::Network(e.to_string()))?;

        if let Some(total) = resp.content_length() {
            if total > MAX_DOWNLOAD_BYTES {
                return Err(QyzsError::Usage(format!(
                    "下载内容 {} 超过大小上限 {}",
                    human_size(total),
                    human_size(MAX_DOWNLOAD_BYTES)
                )));
            }
        }

        let dir = tempfile::tempdir()?;
        let file_path = dir.path().join("input.bin");
        let mut file = std::fs::File::create(&file_path)?;
        let mut written: u64 = 0;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| QyzsError::Network(e.to_string()))?;
            written += chunk.len() as u64;
            if written > MAX_DOWNLOAD_BYTES {
                return Err(QyzsError::Usage(format!(
                    "下载内容超过大小上限 {}",
                    human_size(MAX_DOWNLOAD_BYTES)
                )));
            }
            file.write_all(&chunk)?;
        }
        file.flush()?;
        Ok((dir, file_path))
    }

    /// speed 档转换执行 (阶段②)。
    ///
    /// 流程: 输入落地 → 格式嗅探 → 工作区开立 (锁/指纹/续跑) → 通道抽取
    /// → 规则清洗 (结构归一契约) → 交付 (返回值, 由调用层写 -o 或 stdout)
    /// → 工作区收尾 (成功清理/失败保留)。
    ///
    /// `on_progress`: (已完成片段序号, 片段总数) 回调，长文档切片串行时提供反馈。
    pub async fn convert(
        client: &QyzsClient,
        opts: &MarkitdownOptions,
        timeout_secs: u64,
        on_progress: Option<&dyn Fn(usize, usize)>,
    ) -> QyzsResult<String> {
        if opts.mode != PipelineMode::Speed {
            return Err(QyzsError::Usage(
                "balanced/rigor 档尚未接入；当前版本转换请使用 --mode speed，或先以 --dry-run 查看规划"
                    .to_string(),
            ));
        }

        let (result, ws) = Self::convert_inner(client, opts, timeout_secs, on_progress).await;
        match result {
            Ok(md) => {
                if let Some(ws) = ws {
                    ws.finish_success(opts.no_clean)?;
                }
                Ok(md)
            }
            Err(err) => {
                let hint = match ws {
                    Some(mut ws) => {
                        ws.finish_failure();
                        format!(
                            "\n工作区已保留: {} (修复后可用 --resume 续跑)",
                            ws.root.display()
                        )
                    }
                    None => String::new(),
                };
                Err(err.with_hint(&hint))
            }
        }
    }

    /// MinerU 解析默认开启 OCR 辅助 (与 parse 命令默认一致: 扫描件为难点, 文本件不受影响)
    const MINERU_IS_OCR: bool = true;

    /// 执行主体; 无论成败都以 Option<Workspace> 归还工作区句柄供收尾
    #[allow(clippy::type_complexity)]
    async fn convert_inner(
        client: &QyzsClient,
        opts: &MarkitdownOptions,
        timeout_secs: u64,
        on_progress: Option<&dyn Fn(usize, usize)>,
    ) -> (
        QyzsResult<String>,
        Option<workspace::Workspace>,
    ) {
        // 统一落地为本地文件 (URL 先下载到临时文件, 内容指纹以字节为准)
        let (_temp_guard, local_path) = if is_url_input(&opts.input) {
            match Self::download_to_temp(&opts.input, timeout_secs).await {
                Ok((dir, file_path)) => (Some(dir), file_path),
                Err(e) => return (Err(e), None),
            }
        } else {
            let path = PathBuf::from(&opts.input);
            if !path.exists() {
                return (
                    Err(QyzsError::Usage(format!("输入文件不存在: {}", opts.input))),
                    None,
                );
            }
            (None, path)
        };

        let detection = if let Some(t) = &opts.type_override {
            Detection {
                format: t.clone(),
                method: DetectMethod::ManualOverride,
                warning: None,
            }
        } else {
            match FormatDetector::detect_file(&local_path) {
                Ok(d) => d,
                Err(e) => return (Err(e), None),
            }
        };

        // 路由前置决策 (一处集中): 不支持/未接入的格式不创建工作区, 避免缓存残留死目录
        let fmt = detection.format.clone();
        let (channel, reason) = Self::route(&fmt);
        if channel.is_none() {
            let msg = reason.unwrap_or_else(|| "该格式暂不支持转换".to_string());
            return (Err(QyzsError::Usage(msg)), None);
        }
        if channel != Some("mineru-slice") && channel != Some("mineru-document") {
            return (
                Err(QyzsError::Usage(format!(
                    "格式 '{}' 的转换通道将在后续阶段接入；当前版本支持 PDF/DOCX (speed 档)",
                    fmt.label()
                ))),
                None,
            );
        }

        // 工作区开立 (输入内容指纹 → 并发锁 → manifest；仅显式 --resume 复用现场)
        let (mut ws, resumed) = match workspace::Workspace::open(
            &local_path,
            detection.format.key(),
            opts.mode.key(),
            opts.resume,
        ) {
            Ok(v) => v,
            Err(e) => return (Err(e), None),
        };
        let _ = resumed; // 续跑语义由 workspace.stage_done 逐段生效

        // 通道抽取 (路由已收窄至 mineru-slice / mineru-document)
        let extracted = match detection.format {
            SourceFormat::Pdf => {
                pdf::extract(
                    client,
                    &mut ws,
                    &local_path.to_string_lossy(),
                    Self::MINERU_IS_OCR,
                    on_progress,
                )
                .await
            }
            SourceFormat::Docx => {
                pdf::extract_document(
                    client,
                    &mut ws,
                    &local_path.to_string_lossy(),
                    Self::MINERU_IS_OCR,
                )
                .await
            }
            _ => Err(QyzsError::Server(format!(
                "内部路由不一致: 格式 '{}' 未匹配到已接入通道",
                fmt.label()
            ))),
        };

        match extracted {
            Ok(raw) => (Ok(clean::rule_clean(&raw)), Some(ws)),
            Err(e) => (Err(e), Some(ws)),
        }
    }
}

fn is_url_input(input: &str) -> bool {
    input.starts_with("http://") || input.starts_with("https://")
}

/// 人类可读的体积表示
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixture(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
    }

    #[test]
    fn test_route_unsupported_legacy_formats() {
        let (channel, reason) = MarkitdownPipeline::route(&SourceFormat::Ppt);
        assert!(channel.is_none());
        assert!(reason.unwrap().contains(".pptx"));

        let (channel, reason) = MarkitdownPipeline::route(&SourceFormat::Vs);
        assert!(channel.is_none());
        assert!(reason.unwrap().contains(".vsdx"));

        // 网关实测拒绝 .doc (HTTP 400 Unsupported file type) → 路由为不支持
        let (channel, reason) = MarkitdownPipeline::route(&SourceFormat::Doc);
        assert!(channel.is_none());
        assert!(reason.unwrap().contains(".docx"));

        let (channel, _) = MarkitdownPipeline::route(&SourceFormat::Pdf);
        assert_eq!(channel, Some("mineru-slice"));
    }

    #[test]
    fn test_stages_per_mode() {
        let speed = MarkitdownPipeline::stages_for(PipelineMode::Speed, "mineru-slice", ImageHandling::Auto, "dsv4");
        assert_eq!(speed.first().unwrap(), "extract(mineru3.2.3, 15页/批切片)");
        assert!(!speed.iter().any(|s| s.contains("polish")));

        let balanced = MarkitdownPipeline::stages_for(PipelineMode::Balanced, "ocr-vl", ImageHandling::Ocr, "dsv4");
        assert_eq!(balanced.first().unwrap(), "extract(paddleocrvl)");
        assert!(balanced.iter().any(|s| s == "polish(dsv4)"));

        let rigor = MarkitdownPipeline::stages_for(PipelineMode::Rigor, "mineru-document", ImageHandling::Auto, "dsv4");
        assert!(rigor.iter().any(|s| s == "cross-verify(bge-m3)"));
        assert!(rigor.iter().any(|s| s == "global-review"));
        assert_eq!(rigor.last().unwrap(), "emit");
    }

    #[test]
    fn test_human_size() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(577), "577 B");
        assert_eq!(human_size(222_283), "217.1 KB");
        assert_eq!(human_size(200 * 1024 * 1024), "200.0 MB");
    }

    #[tokio::test]
    async fn test_plan_on_real_pdf() {
        let pdf = fixture("tests/data/mineru/text_pdf/REF-0056_政府采购评审专家管理办法.pdf");
        let opts = MarkitdownOptions {
            input: pdf.to_string_lossy().to_string(),
            output: None,
            mode: PipelineMode::Balanced,
            type_override: None,
            image_handling: ImageHandling::Auto,
            model: None,
            min_confidence: None,
            report: None,
            work_dir: None,
            segment_chars: 24_000,
            resume: false,
            no_clean: false,
            dry_run: true,
        };
        let plan = MarkitdownPipeline::plan(&opts, 30).await.unwrap();
        assert_eq!(plan.detected_format, "pdf");
        assert_eq!(plan.detect_method, "content-sniff");
        assert_eq!(plan.channel, Some("mineru-slice"));
        assert!(plan.supported);
        assert!(plan.page_count.unwrap_or(0) > 0);
        assert!(plan.slice_count.unwrap_or(0) >= 1);
        assert_eq!(plan.output_target, "stdout");
    }

    #[tokio::test]
    async fn test_plan_missing_input() {
        let opts = MarkitdownOptions {
            input: "/nonexistent/no_such_file.pdf".to_string(),
            output: None,
            mode: PipelineMode::Speed,
            type_override: None,
            image_handling: ImageHandling::Auto,
            model: None,
            min_confidence: None,
            report: None,
            work_dir: None,
            segment_chars: 24_000,
            resume: false,
            no_clean: false,
            dry_run: true,
        };
        let err = MarkitdownPipeline::plan(&opts, 30).await.unwrap_err();
        assert!(matches!(err, QyzsError::Usage(_)));
    }
}
