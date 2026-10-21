//! scan 智能文档转 Markdown 流水线
//!
//! 端到端流程 (各段落的实现见对应子模块):
//! 1. **嗅探与路由** — 内容魔数 > 容器结构 > 扩展名; `--type` 覆盖;
//!    `--dry-run` 离线输出规划。 (`detect`)
//! 2. **工作区** — 输入指纹 + 并发锁 + manifest 段×阶段状态机, 支持
//!    `--resume` 幂等续跑与成功即流式清理。 (`workspace`)
//! 3. **抽取** — PDF 本地切片 + MinerU 逐片聚合; Office/Visio 本地部件抽取;
//!    图像走 paddleocrvl + qwen3-VL; 扫描件在 rigor 档另有 OCR/VL 双通道
//!    证据。 (`pdf` / `ooxml` / `sheet_tables` / `ocr_vl` / `pdf_ocr`)
//! 4. **档位后处理** — speed 规则清洗; balanced 分段润色 + light-check;
//!    rigor 多通道交叉验证 + 逐段仲裁 + 全局审查。 (`clean` / `polish` /
//!    `cross_verify`)
//! 5. **收尾** — 体裁识别与体裁特化排版 (按编号/样式重建层级)、图片附件
//!    抽取与引用重写。 (`classify` / `profile` / `style` / `assets`)
//!
//! 行为契约见 `tests/docs/QYZS-SCAN.md`; 抽取能力边界实测数据见
//! `capability_matrix.toml`。

pub mod assets;
pub mod classify;
pub mod clean;
pub mod cross_verify;
pub mod csv_md;
pub mod detect;
pub mod front_matter;
pub mod html_md;
pub mod image_refs;
pub mod json_md;
pub mod light_check;
pub mod multi;
pub mod ocr_vl;
pub mod ooxml;
pub mod pdf;
pub mod pdf_ocr;
pub mod polish;
pub mod profile;
pub mod report;
pub mod sheet_tables;
pub mod style;
pub mod workspace;
pub mod xml_md;
pub mod zip_iter;

use crate::client::QyzsClient;
use crate::error::{QyzsError, QyzsResult};
use crate::pdf::PdfExtractor;
use detect::{DetectMethod, Detection, FormatDetector, SourceFormat};
use futures_util::StreamExt;
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};

/// URL 输入下载大小上限
const MAX_DOWNLOAD_BYTES: u64 = 200 * 1024 * 1024;

/// MinerU 同步通道实测吞吐 (秒/页，capability_matrix 校准真值)
const MINERU_SECONDS_PER_PAGE: f64 = 0.26;

/// 转换场景超时下限 (秒): 网关同步解析存在长尾, 扫描件实测需 >60s
const EXTRACT_TIMEOUT_FLOOR_SECS: u64 = 300;

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
pub struct ScanOptions {
    pub input: Vec<String>,
    pub output: Option<String>,
    pub recursive: bool,
    pub mode: PipelineMode,
    pub type_override: Option<SourceFormat>,
    pub image_handling: ImageHandling,
    /// 图像输入的自定义 OCR 引导提示词
    pub image_prompt: Option<String>,
    /// 图像长边最大像素尺寸 (防网关越界)
    pub max_dim: Option<u32>,
    pub model: Option<String>,
    pub min_confidence: Option<f64>,
    pub report: Option<String>,
    pub work_dir: Option<String>,
    pub segment_chars: usize,
    pub resume: bool,
    pub no_clean: bool,
    pub no_front_matter: bool,
    pub dry_run: bool,
    /// 图片附件输出目录 (None: 与输出 .md 同级的 assets/)
    pub assets_dir: Option<String>,
    /// 不抽取图片附件
    pub no_assets: bool,
    /// 跳过文档类型识别
    pub no_classify: bool,
    /// 体裁识别模型: None = 规则优先, 判不出时用默认模型兜底;
    /// Some(m) = 模型优先, 规则兜底
    pub classify_model: Option<String>,
}

/// 流水线过程指标 (阶段耗时 / 冲突数), 供 --report 兑现其承诺字段
#[derive(Debug, Default, Clone)]
pub struct PipelineMetrics {
    /// 各阶段耗时 (秒)
    pub stages: Vec<(String, f64)>,
    /// rigor 档判定为冲突的段落数
    pub conflicts: usize,
}

impl PipelineMetrics {
    /// 记录一个阶段的耗时
    fn record(&mut self, name: &str, secs: f64) {
        self.stages.push((name.to_string(), secs));
    }
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

pub struct ScanPipeline;

impl ScanPipeline {
    /// 解析输入并产出流水线规划 (dry-run)。
    /// URL 输入先下载到临时文件再统一嗅探；临时文件随本次规划结束删除。
    /// `single_input` 是单文件路径/URL (顶层多文件编排由 `multi::run` 承担)。
    pub async fn plan(
        opts: &ScanOptions,
        single_input: &str,
        timeout_secs: u64,
    ) -> QyzsResult<ConversionPlan> {
        let is_url = is_url_input(single_input);
        let (_temp_guard, local_path) = if is_url {
            let (dir, path) = Self::download_to_temp(single_input, timeout_secs).await?;
            (Some(dir), path)
        } else {
            let path = PathBuf::from(single_input);
            if !path.exists() {
                return Err(QyzsError::Usage(format!("输入文件不存在: {}", single_input)));
            }
            if !path.is_file() {
                return Err(QyzsError::Usage(format!(
                    "输入路径不是常规文件: {}",
                    single_input
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
                    "共 {} 页：按每批 {} 页同步串行抽取，仅抽取环节预估约 {} 秒 (2026-10-18 实测 40 页/10.7MB 网关同步完成，未触发异步；若网关返回异步任务状态将显式报错)",
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
            input: single_input.to_string(),
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
            SourceFormat::Csv => (Some("csv-tables"), None),
            SourceFormat::Json => (Some("json-pretty"), None),
            SourceFormat::Xml => (Some("xml-to-md"), None),
            SourceFormat::Zip => (Some("zip-iterate"), None),
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
            "csv-tables" => "extract(本地 csv 表格化)".to_string(),
            "json-pretty" => "extract(pretty + 同构数组表格化)".to_string(),
            "xml-to-md" => "extract(xml 元素展开)".to_string(),
            "zip-iterate" => "extract(zip 容器迭代, 子格式路由)".to_string(),
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
    /// `single_input` 是单文件路径/URL；顶层多文件编排由 `multi::run` 承担。
    pub async fn convert(
        client: &QyzsClient,
        opts: &ScanOptions,
        single_input: &str,
        timeout_secs: u64,
        on_progress: Option<&dyn Fn(usize, usize)>,
    ) -> QyzsResult<String> {
        let started = std::time::Instant::now();
        let mut metrics = PipelineMetrics::default();
        let (result, ws) = {
            // 抽取/润色共用的宽延时客户端: 网关同步解析存在长尾 (实测扫描件
            // 默认 60s 超时误伤, 300s 内完成), 转换场景取 max(用户超时, 300s)
            let mut cfg = client.config().clone();
            cfg.timeout_seconds = cfg.timeout_seconds.max(EXTRACT_TIMEOUT_FLOOR_SECS);
            let extract_client = QyzsClient::new(cfg)?;
            Self::convert_inner(
                &extract_client,
                opts,
                single_input,
                timeout_secs,
                on_progress,
                &mut metrics,
            )
            .await
        };
        match result {
            Ok(md) => {
                // 收集警告 (manifest.warnings, 含 PDF 单片降级标记)
                let warnings: Vec<String> = ws
                    .as_ref()
                    .map(|w| w.warnings().to_vec())
                    .unwrap_or_default();
                if let Some(ws) = ws {
                    ws.finish_success(opts.no_clean)?;
                }
                let duration_secs = started.elapsed().as_secs_f64();

                // 类型识别 + 特化排版 + 图片资产落盘 (scan 区别于旧转换命令的核心)
                let md = Self::finalize(client, opts, single_input, md, &mut metrics).await?;

                // front matter prepend (默认开启, --no-front-matter 关闭)
                let md = if opts.no_front_matter {
                    md
                } else {
                    Self::prepend_front_matter(opts, single_input, md)
                };

                // 图片引用落盘缺口: 网关只回 Markdown 正文, 不提供图片字节。
                // 已由 assets 通道落盘的引用会通过存在性校验, 不再误报。
                let md_dir = opts
                    .output
                    .as_ref()
                    .map(|o| {
                        let p = PathBuf::from(o);
                        if p.extension().is_some() {
                            p.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
                        } else {
                            p
                        }
                    });
                let image_stats =
                    image_refs::ImageRefStats::analyze_with_base(&md, md_dir.as_deref());
                if image_stats.unresolved > 0 {
                    eprintln!(
                        "\x1b[33m[WARN]\x1b[0m 产物含 {} 处未落盘图片引用 (网关未提供图片字节, 链接不可达)",
                        image_stats.unresolved
                    );
                }

                // --report: 写结构化 JSON 报告 (best-effort, 失败仅警告不影响主输出)
                if let Some(report_path) = &opts.report {
                    let report = Self::build_report(
                        opts,
                        single_input,
                        &md,
                        duration_secs,
                        warnings,
                        &metrics,
                    );
                    if let Err(e) = report.write_to(report_path) {
                        eprintln!(
                            "\x1b[33m[WARN]\x1b[0m 报告写入失败 ({}): {}",
                            report_path, e
                        );
                    }
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

    /// 构造报告 (字段来自前端 compat detect / opts / 实际输出)
    fn build_report(
        opts: &ScanOptions,
        single_input: &str,
        final_md: &str,
        duration_secs: f64,
        warnings: Vec<String>,
        metrics: &PipelineMetrics,
    ) -> report::ConversionReport {
        let (format, detect_method, size_bytes, page_count, slice_count) =
            Self::collect_report_meta(opts, single_input);
        let model = opts.model.clone().unwrap_or_else(|| "dsv4".to_string());
        let image_stats = image_refs::ImageRefStats::analyze(final_md);
        let stages = metrics
            .stages
            .iter()
            .map(|(name, secs)| report::StageTiming {
                name: name.clone(),
                secs: *secs,
            })
            .collect();
        let mut report = report::ConversionReport::build_with_details(
            single_input,
            size_bytes,
            &format,
            detect_method,
            opts.mode,
            &model,
            page_count,
            slice_count,
            final_md.len(),
            opts.output.as_deref().unwrap_or("stdout"),
            duration_secs,
            warnings,
            stages,
            image_stats.total,
            image_stats.unresolved,
            opts.min_confidence,
        );
        report.conflicts = metrics.conflicts;
        report
    }

    /// 报告所需元信息: 重嗅探输入以取 format/detect_method/size_bytes/page_count
    fn collect_report_meta(
        opts: &ScanOptions,
        single_input: &str,
    ) -> (SourceFormat, DetectMethod, u64, Option<usize>, Option<usize>) {
        let detection = if let Some(t) = &opts.type_override {
            Detection {
                format: t.clone(),
                method: DetectMethod::ManualOverride,
                warning: None,
            }
        } else if is_url_input(single_input) {
            Detection {
                format: SourceFormat::Unknown,
                method: DetectMethod::ExtensionFallback,
                warning: None,
            }
        } else {
            FormatDetector::detect_file(std::path::Path::new(single_input)).unwrap_or(Detection {
                format: SourceFormat::Unknown,
                method: DetectMethod::ExtensionFallback,
                warning: None,
            })
        };
        let size_bytes = if is_url_input(single_input) {
            0
        } else {
            std::fs::metadata(single_input).map(|m| m.len()).unwrap_or(0)
        };
        let page_count = if detection.format == SourceFormat::Pdf && !is_url_input(single_input) {
            crate::pdf::PdfExtractor::read_metadata(single_input)
                .ok()
                .map(|meta| meta.page_count)
        } else {
            None
        };
        let slice_count = page_count.map(pdf::estimate_slices);
        (detection.format, detection.method, size_bytes, page_count, slice_count)
    }

    /// MinerU 解析默认开启 OCR 辅助 (扫描件为难点, 文本件不受影响)
    const MINERU_IS_OCR: bool = true;

    /// 统一落地为本地文件 (URL 先下载到临时文件, 内容指纹以字节为准)
    /// → 格式嗅探 → 通道路由 → 照片跳过 → 工作区开立 → 通道抽取 → 档位后处理。
    /// 无论成败都以 Option<Workspace> 归还工作区句柄供收尾。
    async fn convert_inner(
        client: &QyzsClient,
        opts: &ScanOptions,
        single_input: &str,
        timeout_secs: u64,
        on_progress: Option<&dyn Fn(usize, usize)>,
        metrics: &mut PipelineMetrics,
    ) -> (QyzsResult<String>, Option<workspace::Workspace>) {
        let (_temp_guard, local_path) = if is_url_input(single_input) {
            match Self::download_to_temp(single_input, timeout_secs).await {
                Ok((dir, file_path)) => (Some(dir), file_path),
                Err(e) => return (Err(e), None),
            }
        } else {
            let path = PathBuf::from(single_input);
            if !path.exists() {
                return (
                    Err(QyzsError::Usage(format!("输入文件不存在: {}", single_input))),
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

        // 通道路由: 未接入的格式直接报错 (含 .doc/.ppt/.vsd 的明确改劝)
        let (channel, reason) = Self::route(&detection.format);
        if channel.is_none() {
            return (
                Err(QyzsError::Usage(
                    reason.unwrap_or_else(|| "该格式暂不支持转换".to_string()),
                )),
                None,
            );
        }

        // 图像输入先判「文档 or 照片」: 照片按 scan 策略跳过, 不进入抽取与润色,
        // 避免为一张风景照白跑 OCR/VL 与后续流水线。
        if !opts.no_classify {
            if let SourceFormat::Image(_) = detection.format {
                match classify::classify_image(client, &local_path.to_string_lossy()).await {
                    Ok(classify::ImageContent::Photo) => {
                        return (
                            Err(QyzsError::Skipped(format!(
                                "{} 判定为真实照片/插画, 非文档图像",
                                single_input
                            ))),
                            None,
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!(
                            "\x1b[33m[WARN]\x1b[0m {}",
                            crate::msgf!(MdImageKindFailed, e)
                        );
                    }
                }
            }
        }

        // 工作区开立 (输入内容指纹 → 并发锁 → manifest; 仅显式 --resume 复用现场)
        let (mut ws, resumed) = match workspace::Workspace::open(
            &local_path,
            detection.format.key(),
            opts.mode.key(),
            opts.resume,
            opts.work_dir.as_deref(),
        ) {
            Ok(v) => v,
            Err(e) => return (Err(e), None),
        };
        let _ = resumed; // 续跑语义由 workspace.stage_done 逐段生效

        // 通道抽取 (路由已收窄至已接入通道)
        let local_path_str = local_path.to_string_lossy().to_string();
        let extract_started = std::time::Instant::now();
        let extracted = match channel {
            Some("mineru-slice") => {
                pdf::extract(
                    client,
                    &mut ws,
                    &local_path_str,
                    Self::MINERU_IS_OCR,
                    on_progress,
                )
                .await
            }
            Some("mineru-document") => {
                pdf::extract_document(client, &mut ws, &local_path_str, Self::MINERU_IS_OCR).await
            }
            Some("sheet-tables") => {
                if detection.format == SourceFormat::Xls {
                    sheet_tables::extract_xls(&mut ws, &local_path_str)
                } else {
                    sheet_tables::extract_xlsx(&mut ws, &local_path_str)
                }
            }
            Some("ooxml-text") => match detection.format {
                SourceFormat::Pptx => ooxml::extract_pptx(&mut ws, &local_path_str),
                _ => ooxml::extract_vsdx(&mut ws, &local_path_str),
            },
            Some("csv-tables") => csv_md::extract_csv(&mut ws, &local_path_str),
            Some("json-pretty") => json_md::extract_json(&mut ws, &local_path_str),
            Some("xml-to-md") => xml_md::extract_xml(&mut ws, &local_path_str),
            Some("zip-iterate") => zip_iter::extract_zip(&mut ws, &local_path_str),
            Some("ocr-vl") => {
                ocr_vl::extract_image(
                    client,
                    &mut ws,
                    &local_path_str,
                    opts.image_handling,
                    opts.image_prompt.as_deref(),
                    opts.max_dim,
                )
                .await
            }
            Some("html-to-md") => html_md::extract_html(&mut ws, &local_path_str),
            _ => Err(QyzsError::Server(format!(
                "内部路由不一致: 格式 '{}' 未匹配到已接入通道",
                detection.format.label()
            ))),
        };
        metrics.record("extract", extract_started.elapsed().as_secs_f64());

        match extracted {
            Ok(raw) => {
                if raw.trim().is_empty() {
                    return (
                        Err(QyzsError::Server(
                            "抽取结果为空 (可能为空白文档、无文本层扫描件或网关未识别出内容)"
                                .to_string(),
                        )),
                        Some(ws),
                    );
                }
                // 档位后处理: speed → 规则清洗; balanced → 分段润色; rigor → 润色
                //   + 双通道交叉验证 + 冲突仲裁 + 全局审查
                // 阶段耗时按不重叠划分记录: 各档位内部已记 polish / cross-verify /
                // arbitrate / global-review, speed 档此处记 rule-clean, 不再另记
                // 粗粒度的 post-process (否则与内层阶段重复计时, 报告里阶段
                // 合计会超过总耗时)
                let outcome = match opts.mode {
                    PipelineMode::Speed => {
                        let started = std::time::Instant::now();
                        let out = Ok(clean::rule_clean(&raw));
                        metrics.record("rule-clean", started.elapsed().as_secs_f64());
                        out
                    }
                    PipelineMode::Balanced => {
                        Self::polish_document(client, opts, &mut ws, &raw, on_progress, metrics)
                            .await
                    }
                    PipelineMode::Rigor => {
                        Self::rigor_document(
                            client, opts, &mut ws, &raw, &local_path_str, &detection.format,
                            on_progress, metrics,
                        )
                        .await
                    }
                };
                match outcome {
                    Ok(md) => (Ok(md), Some(ws)),
                    Err(e) => (Err(e), Some(ws)),
                }
            }
            Err(e) => (Err(e), Some(ws)),
        }
    }

    /// front matter prepend: 在 markdown 头部附加 YAML 元数据块。
    /// 重新嗅探单文件 → size_bytes → 关键字段; 失败降级为原 markdown。
    fn prepend_front_matter(opts: &ScanOptions, source: &str, md: String) -> String {
        let detection = if let Some(t) = &opts.type_override {
            Detection {
                format: t.clone(),
                method: DetectMethod::ManualOverride,
                warning: None,
            }
        } else if is_url_input(source) {
            // URL: 没法本地读, 格式从 -o 路径推不出, 降级 unknown
            Detection {
                format: SourceFormat::Unknown,
                method: DetectMethod::ExtensionFallback,
                warning: None,
            }
        } else {
            let p = std::path::Path::new(source);
            FormatDetector::detect_file(p).unwrap_or(Detection {
                format: SourceFormat::Unknown,
                method: DetectMethod::ExtensionFallback,
                warning: None,
            })
        };

        let size_bytes = if is_url_input(source) {
            0
        } else {
            std::fs::metadata(source).map(|m| m.len()).unwrap_or(0)
        };

        // PDF 专属: 页数 (廉价 lopdf 头解析, 失败降级 None)
        let page_count = if detection.format == SourceFormat::Pdf && !is_url_input(source) {
            PdfExtractor::read_metadata(source).ok().map(|m| m.page_count)
        } else {
            None
        };

        let model = opts.model.clone().unwrap_or_else(|| "dsv4".to_string());
        let fm = front_matter::FrontMatter {
            source,
            format: detection.format.key(),
            detect_method: detection.method,
            size_bytes,
            mode: opts.mode,
            model: &model,
            page_count,
        };
        let mut out = front_matter::render(&fm);
        out.push_str(&md);
        out
    }

    /// 收尾阶段: 文档类型识别 → 类型特化排版 → 图片附件抽取与引用重写
    ///
    /// 顺序不可交换: 排版要在正文结构确定后做 (按编号重建层级), 资产重写要在
    /// 排版之后做 (避免改写后的标题被当作引用处理; 二者作用域不重叠, 但先
    /// 结构后资源更符合"内容定型再挂附件"的语义)。
    async fn finalize(
        client: &QyzsClient,
        opts: &ScanOptions,
        single_input: &str,
        md: String,
        metrics: &mut PipelineMetrics,
    ) -> QyzsResult<String> {
        let started = std::time::Instant::now();

        // 1) 类型识别: 规则优先 (确定且零成本); 规则判不出 (Generic) 或显式
        //    指定 --classify-model 时调用模型。模型失败回退规则结果, 不阻断转换。
        let decided = if opts.no_classify {
            (classify::DocKind::Generic, "已跳过".to_string())
        } else {
            let format = Self::detect_format_for(opts, single_input);
            let rule = classify::decide(&md, &format);
            // 三条路径: 指定模型 → 模型优先; 强命中 → 规则定案;
            // 弱命中 / 判不出 → 模型仲裁 (假阳性此前永远到不了模型, 这是盲区修复)
            match (&opts.classify_model, rule.confidence, rule.kind) {
                (Some(m), _, _) => {
                    match classify::classify_genre_model(client, &md, Some(m)).await {
                        Ok(k) => (k, format!("模型({m})")),
                        Err(e) => {
                            eprintln!(
                                "\x1b[33m[WARN]\x1b[0m {}",
                                crate::msgf!(MdClassifyModelFailed, e)
                            );
                            (rule.kind, format!("规则({:?},模型失败回退)", rule.confidence))
                        }
                    }
                }
                (None, _, classify::DocKind::Generic) => {
                    match classify::classify_genre_model(client, &md, None).await {
                        Ok(k) => (k, "模型(规则判不出兜底)".to_string()),
                        Err(e) => {
                            eprintln!(
                                "\x1b[33m[WARN]\x1b[0m {}",
                                crate::msgf!(MdClassifyFallbackFailed, e)
                            );
                            (classify::DocKind::Generic, "规则(判不出,兜底失败)".to_string())
                        }
                    }
                }
                (None, classify::Confidence::Strong, k) => (k, "规则(强信号)".to_string()),
                (None, classify::Confidence::Weak, k) => {
                    match classify::classify_genre_model(client, &md, None).await {
                        Ok(arbitrated) if arbitrated != k => (
                            arbitrated,
                            format!(
                                "模型仲裁(规则弱命中{},改判 {})",
                                k.key(),
                                arbitrated.key()
                            ),
                        ),
                        Ok(_) => (k, "规则(弱命中,模型确认)".to_string()),
                        Err(e) => {
                            eprintln!(
                                "\x1b[33m[WARN]\x1b[0m {}",
                                crate::msgf!(MdWeakArbitrateFailed, e)
                            );
                            (k, format!("规则(弱{:?},仲裁失败保留)", rule.confidence))
                        }
                    }
                }
            }
        };

        metrics.record("classify", started.elapsed().as_secs_f64());
        if !opts.no_classify {
            eprintln!(
                "\x1b[36m[INFO]\x1b[0m {}",
                crate::msgf!(
                    MdDocKindDetected,
                    decided.0.label(),
                    decided.0.key(),
                    decided.1
                )
            );
        }
        let kind = decided.0;

        // 2) 类型特化排版 (编号层级重建)
        let layout_started = std::time::Instant::now();
        let mut md = profile::apply(kind, &md);
        metrics.record("layout", layout_started.elapsed().as_secs_f64());

        // 2b) 样式信息修正层级: DOCX 用显式样式 (权威), PDF 用大字号行 (补充)。
        //     样式只做精确文本匹配后的层级校正, 不凭空新增标题。
        if !opts.no_classify {
            let style_started = std::time::Instant::now();
            md = Self::apply_style(md, opts, single_input);
            metrics.record("style", style_started.elapsed().as_secs_f64());
        }

        // 3) 图片附件
        if opts.no_assets {
            return Ok(md);
        }
        let assets_started = std::time::Instant::now();
        let format = Self::detect_format_for(opts, single_input);
        let (assets_root, stem, md_dir) = match Self::assets_target(opts, single_input) {
            Some(v) => v,
            None => return Ok(md), // 输出到 stdout: 无落盘位置, 不抽附件
        };
        let assets = assets::extract(single_input, &format, &assets_root, &stem, &md_dir)?;
        metrics.record("assets", assets_started.elapsed().as_secs_f64());
        if assets.is_empty() {
            return Ok(md);
        }
        let (new_md, replaced, unresolved) = assets::rewrite_refs(&md, &assets);
        eprintln!(
            "\x1b[36m[INFO]\x1b[0m {}",
            crate::msgf!(
                MdAssetsExtracted,
                assets.len(),
                assets_root.display().to_string(),
                replaced,
                if unresolved > 0 {
                    format!(", {} unresolved marked", unresolved)
                } else {
                    String::new()
                }
            )
        );
        // 附件存在但正文引用全部未能对应时, 追加索引章节:
        // 文件已在 assets/ 里, 若不在正文留下任何入口, 使用方根本发现不了。
        let new_md = if replaced == 0 {
            Self::append_asset_index(new_md, &assets)
        } else {
            new_md
        };
        Ok(new_md)
    }

    /// 用样式信息修正标题层级
    ///
    /// - DOCX: `word/styles.xml` 的 Heading/标题 样式是权威来源, 直接映射
    /// - PDF: 以最大字号行作为文档标题 (若其已是标题行则收敛到 `#`)
    ///
    /// 两种来源都只做「精确文本匹配后的层级校正」, 不凭空新增标题。
    fn apply_style(md: String, opts: &ScanOptions, single_input: &str) -> String {
        let format = Self::detect_format_for(opts, single_input);
        let levels: Vec<(String, usize)> = match format {
            SourceFormat::Docx => match std::fs::read(single_input) {
                Ok(bytes) => style::docx_headings(&bytes)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|h| (h.text, h.level))
                    .collect(),
                Err(_) => Vec::new(),
            },
            SourceFormat::Pdf => match style::pdf_lines(single_input) {
                Ok(lines) => {
                    let body = style::body_size(&lines);
                    lines
                        .iter()
                        .filter(|l| style::is_large_heading(l, body))
                        .map(|l| (l.text.clone(), 1usize))
                        .collect()
                }
                Err(_) => Vec::new(),
            },
            _ => Vec::new(),
        };
        if levels.is_empty() {
            return md;
        }
        let (out, changed) = profile::apply_style_levels(&md, &levels);
        if changed > 0 {
            eprintln!(
                "\x1b[36m[INFO]\x1b[0m {}",
                crate::msgf!(MdStyleAdjusted, changed)
            );
        }
        out
    }

    /// 附件输出目录、源文件名主干与 Markdown 所在目录 (相对路径基准)
    ///
    /// 默认 `<输出 .md 所在目录>/assets`; 输出为 stdout 时返回 None (无处落盘)。
    fn assets_target(
        opts: &ScanOptions,
        single_input: &str,
    ) -> Option<(PathBuf, String, PathBuf)> {
        let md_dir = match &opts.output {
            Some(out) => {
                let p = PathBuf::from(out);
                if p.extension().is_some() {
                    p.parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| PathBuf::from("."))
                } else {
                    p
                }
            }
            None => return None,
        };
        let root = match &opts.assets_dir {
            Some(d) => PathBuf::from(d),
            None => md_dir.join("assets"),
        };
        let stem = Path::new(single_input)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("document")
            .to_string();
        Some((root, stem, md_dir))
    }

    /// 取单输入的文件格式 (尊重 --type 覆盖)
    fn detect_format_for(opts: &ScanOptions, source: &str) -> SourceFormat {
        if let Some(t) = &opts.type_override {
            return t.clone();
        }
        if is_url_input(source) {
            return SourceFormat::Unknown;
        }
        FormatDetector::detect_file(Path::new(source))
            .map(|d| d.format)
            .unwrap_or(SourceFormat::Unknown)
    }

    /// 追加附件索引章节 (正文位置无法对应时的兜底入口)
    fn append_asset_index(mut md: String, assets: &[assets::ExtractedAsset]) -> String {
        if assets.is_empty() {
            return md;
        }
        md.push_str("\n\n## 图片附件\n\n");
        md.push_str("> ");
        md.push_str(&crate::msg!(MdAssetIndexNote));
        md.push_str("\n\n");
        for a in assets {
            let name = a
                .file
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("asset");
            md.push_str(&format!("- ![]({}) `{}` ({})\n", a.rel, name, a.origin));
        }
        md
    }

    /// balanced 档: 全文术语表提取 → 分段 → 逐段润色 →
    /// light-check (不过则回退原文) → 规则清洗收尾。段级状态入 manifest。
    async fn polish_document(
        client: &QyzsClient,
        opts: &ScanOptions,
        ws: &mut workspace::Workspace,
        raw: &str,
        on_progress: Option<&dyn Fn(usize, usize)>,
        metrics: &mut PipelineMetrics,
    ) -> QyzsResult<String> {
        let model = opts.model.clone().unwrap_or_else(|| "dsv4".to_string());
        let polish_started = std::time::Instant::now();
        let term_table = polish::extract_term_table(raw);
        let segments = polish::segment_markdown(raw, opts.segment_chars);
        let total = segments.len();
        let mut warnings: Vec<String> = Vec::new();

        let mut polished_doc = String::new();
        for (idx, (heading_path, body)) in segments.iter().enumerate() {
            let seg_id = format!("seg-{:03}", idx + 1);

            let text = if ws.stage_done(&seg_id, "polish") {
                ws.read_segment_text(&seg_id)?
            } else {
                let polished =
                    polish::polish_segment(client, &model, body, heading_path, &term_table)
                        .await
                        .map_err(|e| {
                            QyzsError::Server(format!("第 {}/{} 段润色失败: {}", idx + 1, total, e))
                        })?;

                // light-check: 不过则回退原文并记录告警
                let check = light_check::check_polished(body, &polished);
                let accepted = if check.passed {
                    polished
                } else {
                    warnings
                        .extend(check.issues.iter().map(|i| format!("段 {}: {}", idx + 1, i)));
                    body.clone()
                };
                ws.write_segment_text(&seg_id, &accepted)?;
                ws.complete_stage(&seg_id, "polish", &[])?;
                accepted
            };
            if let Some(progress) = on_progress {
                progress(idx + 1, total);
            }
            polished_doc.push_str(&text);
            polished_doc.push_str("\n\n");
        }

        for w in &warnings {
            eprintln!("\x1b[33m[WARN]\x1b[0m {}: {}", crate::msg!(MdPolishFallback), w);
            let _ = ws.add_warning(format!("{}: {}", crate::msg!(MdPolishFallback), w));
        }

        metrics.record("polish", polish_started.elapsed().as_secs_f64());
        Ok(clean::rule_clean(&polished_doc))
    }

    /// OCR 第二通道: 提取扫描件页面内嵌整页图 → 逐页双通道转录
    /// (paddleocrvl 文字 + qwen3-VL 图像理解), 按页合并为交叉验证证据。
    /// 产物持久化为 "secondary-evidence" 段 (manifest 可续跑); 双通道全败
    /// (合并文本为空) 返回 Err 由调用方降级。
    async fn ocr_secondary(
        client: &QyzsClient,
        ws: &mut workspace::Workspace,
        local_path: &str,
        on_progress: Option<&dyn Fn(usize, usize)>,
    ) -> QyzsResult<(String, Vec<String>)> {
        if ws.stage_done("secondary-evidence", "extract") {
            return Ok((ws.read_segment_text("secondary-evidence")?, Vec::new()));
        }
        let dir = ws.root.join("secondary-images");
        let pages = pdf_ocr::extract_page_images(local_path, &dir)?;
        if pages.is_empty() {
            return Err(QyzsError::Server(
                "未提取到任何页面内嵌图 (可能不是图像型扫描件)".to_string(),
            ));
        }
        // 双通道并发执行 (各自内部 4 路并发), 单通道失败不拖垮另一通道
        let ocr_fut = pdf_ocr::ocr_pages(client, &pages, on_progress);
        let vl_fut = pdf_ocr::vl_pages(client, &pages, on_progress);
        let ((ocr_texts, ocr_warns), (vl_texts, vl_warns)) =
            tokio::join!(ocr_fut, vl_fut);
        let evidence: Vec<pdf_ocr::PageEvidence> = pages
            .iter()
            .zip(ocr_texts)
            .zip(vl_texts)
            .map(|((p, ocr), vl)| pdf_ocr::PageEvidence {
                page: p.page,
                ocr,
                vl,
            })
            .collect();
        let text = pdf_ocr::merge_page_evidence(&evidence);
        if text.trim().is_empty() {
            return Err(QyzsError::Server(
                "OCR 与 VL 双通道全部页失败, 无第二通道证据".to_string(),
            ));
        }
        ws.write_segment_text("secondary-evidence", &text)?;
        ws.complete_stage("secondary-evidence", "extract", &[])?;
        Ok((text, [ocr_warns, vl_warns].concat()))
    }

    /// rigor 档: balanced 润色 → 第二通道交叉验证 → 冲突段仲裁 (≤2 轮) →
    /// 不可决段标记 → 全局结构审查。第二通道不可用 (扫描件文本层为空/本地
    /// 通道无独立第二通道) 时诚实降级并告警, 不静默伪装验证通过。
    #[allow(clippy::too_many_arguments)]
    async fn rigor_document(
        client: &QyzsClient,
        opts: &ScanOptions,
        ws: &mut workspace::Workspace,
        raw: &str,
        local_path: &str,
        format: &SourceFormat,
        on_progress: Option<&dyn Fn(usize, usize)>,
        metrics: &mut PipelineMetrics,
    ) -> QyzsResult<String> {
        // 1. 润色 (与 balanced 共用, 段级状态复用)
        let polished = Self::polish_document(client, opts, ws, raw, on_progress, metrics).await?;

        // 2. 第二通道抽取 (manifest 阶段 "secondary")
        let secondary = if ws.stage_done("secondary", "extract") {
            ws.read_segment_text("secondary")?
        } else {
            let text = match format {
                SourceFormat::Pdf => {
                    let pages = crate::pdf::PdfExtractor::read_metadata(local_path)?.page_count;
                    crate::pdf::PdfExtractor::extract_text(
                        local_path,
                        &(1..=pages as u32).collect::<Vec<u32>>(),
                    )?
                }
                SourceFormat::Docx => {
                    let bytes = std::fs::read(local_path)?;
                    ooxml::extract_docx_text(&bytes)?
                }
                _ => String::new(),
            };
            ws.write_segment_text("secondary", &text)?;
            ws.complete_stage("secondary", "extract", &[])?;
            text
        };

        // 3. 第二通道可用性判定: 文本层过薄 (扫描件/无线性文本的格式) 时,
        //    PDF 先尝试 OCR 第二通道 (页面内嵌整页图 → paddleocrvl 逐页),
        //    与主通道 MinerU 构成独立双模型; OCR 通道也不可用才诚实降级。
        let secondary_chars = secondary.trim().chars().count();
        let primary_chars = polished.chars().count();
        let usable_secondary = secondary_chars >= 200 && secondary_chars * 5 >= primary_chars;
        let mut warnings: Vec<String> = Vec::new();
        let mut secondary = secondary;
        if !usable_secondary {
            let ocr_secondary = if *format == SourceFormat::Pdf {
                let started = std::time::Instant::now();
                let r = Self::ocr_secondary(client, ws, local_path, on_progress).await;
                metrics.record("secondary-evidence", started.elapsed().as_secs_f64());
                r
            } else {
                Err(QyzsError::Usage("仅 PDF 支持 OCR 第二通道".to_string()))
            };
            match ocr_secondary {
                Ok((ocr_text, mut ocr_warnings)) => {
                    warnings.append(&mut ocr_warnings);
                    let ocr_chars = ocr_text.trim().chars().count();
                    if ocr_chars >= 200 {
                        secondary = ocr_text;
                    } else {
                        let msg = format!(
                            "第二通道文本层过薄 ({} 字符), OCR 第二通道产出亦不足 ({} 字符), 交叉验证不可用, rigor 降级为 balanced 行为",
                            secondary_chars, ocr_chars
                        );
                        let _ = ws.add_warning(msg.clone());
                        warnings.push(msg);
                        for w in &warnings {
                            eprintln!("\x1b[33m[WARN]\x1b[0m {}", w);
                        }
                        return Ok(polished);
                    }
                }
                Err(e) => {
                    let msg = format!(
                        "第二通道文本层过薄 ({} 字符), OCR 第二通道不可用 ({}), 交叉验证不可用, rigor 降级为 balanced 行为",
                        secondary_chars, e
                    );
                    let _ = ws.add_warning(msg.clone());
                    warnings.push(msg);
                    for w in &warnings {
                        eprintln!("\x1b[33m[WARN]\x1b[0m {}", w);
                    }
                    return Ok(polished);
                }
            }
        }

        // 4. 向量交叉验证 (manifest 阶段 "verify")
        let paragraphs = cross_verify::split_paragraphs(&polished);
        let evidence_chunks = cross_verify::chunk_secondary(&secondary);
        let verify_started = std::time::Instant::now();
        let similarity = cross_verify::verify_paragraphs(client, &paragraphs, &evidence_chunks).await?;
        metrics.record("cross-verify", verify_started.elapsed().as_secs_f64());

        let threshold = opts.min_confidence.unwrap_or(0.70);
        let conflicts: Vec<(usize, f64, usize)> = similarity
            .into_iter()
            .filter(|(_, score, _)| *score < threshold)
            .collect();
        metrics.conflicts = conflicts.len();
        eprintln!(
            "\x1b[36m[INFO]\x1b[0m {}",
            crate::msgf!(MdCrossVerifySummary, paragraphs.len(), conflicts.len(), threshold)
        );

        // 5. 冲突仲裁 (≤2 轮; manifest 阶段 "arbitrate")
        let arbitrate_model = opts.model.clone().unwrap_or_else(|| "dsv4".to_string());
        let arbitrate_started = std::time::Instant::now();
        let mut final_md = polished.clone();
        for (para_idx, score, chunk_idx) in &conflicts {
            let seg_id = format!("arb-{:04}", para_idx);
            if ws.stage_done(&seg_id, "arbitrate") {
                continue; // 状态恢复: 已仲裁段保持现状
            }

            let paragraph = &paragraphs[*para_idx];
            let evidence = &evidence_chunks[*chunk_idx];
            let mut resolved: Option<String> = None;
            for attempt in 1..=cross_verify::ARBITRATION_ROUNDS {
                match cross_verify::arbitrate_paragraph(
                    client,
                    &arbitrate_model,
                    paragraph,
                    evidence,
                    attempt,
                )
                .await
                {
                    Ok(Some(corrected)) => {
                        resolved = Some(corrected);
                        break;
                    }
                    Ok(None) => continue,
                    Err(e) => {
                        warnings.push(format!(
                            "段落仲裁请求失败 (轮 {}): {}",
                            attempt,
                            e
                        ));
                        break;
                    }
                }
            }

            match resolved {
                Some(corrected) if corrected != *paragraph => {
                    // 结构护栏: 仲裁结果不得丢失表格行数。
                    // 第二通道是扁平文本, 模型常把表格「修正」成流水句 ——
                    // 这不是修正, 是结构退化, 一律回退保留原文。
                    let before = cross_verify::count_table_rows(paragraph);
                    let after = cross_verify::count_table_rows(&corrected);
                    if before > 0 && after < before {
                        warnings.push(format!(
                            "段落仲裁结果丢失表格行 ({} → {}), 已保留原文",
                            before, after
                        ));
                    } else {
                        final_md = final_md.replace(paragraph.as_str(), &corrected);
                        warnings.push(crate::msgf!(MdArbitrateCorrected, format!("{:.2}", score)));
                    }
                }
                Some(_) => {
                    warnings.push(crate::msgf!(MdArbitrateConfirmed, format!("{:.2}", score)));
                }
                None => {
                    let marker = format!("<!-- qyzs:unverified (相似度 {:.2}) -->\n\n", score);
                    final_md = final_md.replace(
                        paragraph.as_str(),
                        &format!("{}{}", marker, paragraph),
                    );
                    warnings.push(crate::msgf!(MdArbitrateUnverified, format!("{:.2}", score)));
                }
            }
            ws.complete_stage(&seg_id, "arbitrate", &[])?;
        }

        metrics.record("arbitrate", arbitrate_started.elapsed().as_secs_f64());

        // 6. 全局结构审查 (目录树 + 段摘要, 规模无关; 建议仅报告不自动应用)
        let review_started = std::time::Instant::now();
        let review_result = Self::global_review(client, &arbitrate_model, &final_md).await;
        metrics.record("global-review", review_started.elapsed().as_secs_f64());
        match review_result {
            Ok(review_notes) if !review_notes.is_empty() => {
                // 推理模型 (qwen3.6-27b) 的思维链会偶发泄漏进审查输出;
                // 检出特征标记时丢弃噪声并如实注明, 而非把过程文本当结论
                if review_notes.contains("thinking process")
                    || review_notes.contains("Analyze User Input")
                    || review_notes.contains("Let's check")
                {
                    warnings.push(
                        "全局审查输出含思维链噪声已丢弃 (该步骤建议使用非推理模型)".to_string(),
                    );
                } else {
                    warnings.push(format!("{}: {}", crate::msg!(MdGlobalReview), review_notes));
                }
            }
            Ok(_) => {}
            Err(e) => warnings.push(format!("全局审查调用失败: {}", e)),
        }

        for w in &warnings {
            eprintln!("\x1b[33m[WARN]\x1b[0m {}", w);
            // 持久化: 报告的 warnings 读自 manifest, 只打印会让仲裁修正与
            // 审查结论在报告里不可见 (2026-10-20 三轮实测发现)
            let _ = ws.add_warning(w.clone());
        }
        Ok(clean::rule_clean(&final_md))
    }

    /// 全局轮: 目录树 + 每节 200 字摘要 → 结构问题清单 (仅报告)
    async fn global_review(
        client: &QyzsClient,
        model: &str,
        document: &str,
    ) -> QyzsResult<String> {
        let mut outline = String::new();
        let mut current_heading = String::new();
        for line in document.lines() {
            let is_heading = line.starts_with('#')
                && line.chars().nth(1).is_some_and(|c| c == ' ' || c == '#');
            if is_heading {
                outline.push_str(&format!("\n{}\n", line));
                current_heading = line.trim_start_matches('#').trim().to_string();
            } else if !line.trim().is_empty() && current_heading.len() < 200 {
                // 每节首行前 200 字符作摘要
                if !outline.ends_with('\u{2026}') {
                    let snippet: String = line.trim().chars().take(100).collect();
                    outline.push_str(&format!("  {}\n", snippet));
                    current_heading.push('\u{2026}');
                }
            }
        }

        let resp = client
            .chat(
                &format!(
                    "以下是文档转换后的目录树与节摘要。仅检查结构问题 (编号跳级、层级倒置、孤儿节、重复章), 逐条输出, 每条一行; 无问题输出空。直接输出结论, 不要输出思考过程或分析步骤, 不要改写内容。\n\n{}",
                    outline
                ),
                Some("你是文档结构审查器。只输出发现的结构问题清单, 每条一行; 没有问题则不输出任何内容。"),
                crate::client::ChatOptions {
                    model: Some(model.to_string()),
                    temperature: Some(0.0),
                    max_tokens: Some(2048),
                    top_p: None,
                    stream: false,
                    show_reasoning: false,
                    images: Vec::new(),
                },
            )
            .await?;

        Ok(resp.primary_text())
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
        let (channel, reason) = ScanPipeline::route(&SourceFormat::Ppt);
        assert!(channel.is_none());
        assert!(reason.unwrap().contains(".pptx"));

        let (channel, reason) = ScanPipeline::route(&SourceFormat::Vs);
        assert!(channel.is_none());
        assert!(reason.unwrap().contains(".vsdx"));

        // 网关实测拒绝 .doc (HTTP 400 Unsupported file type) → 路由为不支持
        let (channel, reason) = ScanPipeline::route(&SourceFormat::Doc);
        assert!(channel.is_none());
        assert!(reason.unwrap().contains(".docx"));

        let (channel, _) = ScanPipeline::route(&SourceFormat::Pdf);
        assert_eq!(channel, Some("mineru-slice"));
    }

    #[test]
    fn test_stages_per_mode() {
        let speed = ScanPipeline::stages_for(PipelineMode::Speed, "mineru-slice", ImageHandling::Auto, "dsv4");
        assert_eq!(speed.first().unwrap(), "extract(mineru3.2.3, 15页/批切片)");
        assert!(!speed.iter().any(|s| s.contains("polish")));

        let balanced = ScanPipeline::stages_for(PipelineMode::Balanced, "ocr-vl", ImageHandling::Ocr, "dsv4");
        assert_eq!(balanced.first().unwrap(), "extract(paddleocrvl)");
        assert!(balanced.iter().any(|s| s == "polish(dsv4)"));

        let rigor = ScanPipeline::stages_for(PipelineMode::Rigor, "mineru-document", ImageHandling::Auto, "dsv4");
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
        let input = pdf.to_string_lossy().to_string();
        let opts = ScanOptions {
            input: vec![input.clone()],
            output: None,
            recursive: false,
            mode: PipelineMode::Balanced,
            type_override: None,
            image_handling: ImageHandling::Auto,
            image_prompt: None,
            max_dim: None,
            model: None,
            min_confidence: None,
            report: None,
            work_dir: None,
            segment_chars: 24_000,
            resume: false,
            no_clean: false,
            no_front_matter: false,
            dry_run: true,
            assets_dir: None,
            no_assets: false,
            no_classify: false,
            classify_model: None,
        };
        let plan = ScanPipeline::plan(&opts, &input, 30).await.unwrap();
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
        let input = "/nonexistent/no_such_file.pdf".to_string();
        let opts = ScanOptions {
            input: vec![input.clone()],
            output: None,
            recursive: false,
            mode: PipelineMode::Speed,
            type_override: None,
            image_handling: ImageHandling::Auto,
            image_prompt: None,
            max_dim: None,
            model: None,
            min_confidence: None,
            report: None,
            work_dir: None,
            segment_chars: 24_000,
            resume: false,
            no_clean: false,
            no_front_matter: false,
            dry_run: true,
            assets_dir: None,
            no_assets: false,
            no_classify: false,
            classify_model: None,
        };
        let err = ScanPipeline::plan(&opts, &input, 30).await.unwrap_err();
        assert!(matches!(err, QyzsError::Usage(_)));
    }
}
