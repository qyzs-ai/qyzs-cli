//! 文档流水线处理器 (markscan/pdf-split/verify/dataset)

use super::Ctx;
use crate::error::{QyzsError, QyzsResult};
use crate::pdf::{PdfExtractor, PdfSplitter};
use crate::pipeline::markscan::{multi, ConversionPlan, MarkScanOptions, MarkScanPipeline};
use crate::pipeline::{DatasetManager, OcrVerifier};
use crate::term::OutputFormat;
use std::path::Path;

pub async fn markscan(ctx: &Ctx<'_>, opts: MarkScanOptions) -> QyzsResult<()> {
    let limited_client = ctx.client.with_max_concurrency(opts.concurrency);
    // 多文件/目录输入: 走 multi 编排; 单文件输入: 走单文件路径
    let is_multi = opts.input.len() > 1
        || opts
            .input
            .iter()
            .any(|p| !is_url_input(p) && Path::new(p).is_dir());

    if is_multi {
        return markscan_multi(ctx, &limited_client, opts).await;
    }

    // 单文件路径 (兼容历史契约)
    let single = match opts.input.first() {
        Some(s) => s.clone(),
        None => return Err(QyzsError::Usage("未提供输入".to_string())),
    };
    let run_output = opts.output.clone();

    if opts.dry_run {
        let plan = MarkScanPipeline::plan(&opts, &single, ctx.config.timeout_seconds).await?;
        if ctx.config.output_format == OutputFormat::Json {
            println!(
                "{}",
                serde_json::to_string_pretty(&plan).unwrap_or_default()
            );
        } else {
            print_scan_plan(ctx.printer, &plan);
        }
    } else {
        let output = match MarkScanPipeline::convert(
            &limited_client,
            &opts,
            &single,
            ctx.config.timeout_seconds,
            Some(&|done: usize, total: usize| {
                if total > 1 {
                    ctx.printer
                        .info(&crate::msgf!(MdSliceProgress, done, total));
                }
            }),
        )
        .await
        {
            Ok(md) => md,
            // 判定为照片等情形: 明确告知已跳过, 不产出空文件
            Err(QyzsError::Skipped(why)) => {
                ctx.printer.warn(&format!("已跳过: {}", why));
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        if let Some(out_p) = run_output {
            std::fs::write(&out_p, &output)?;
            ctx.printer.ok(&crate::msgf!(MdSaved, out_p));
        } else {
            print!("{}", output);
            if !output.ends_with('\n') {
                println!();
            }
        }
    }
    Ok(())
}

/// 多文件/目录输入编排: 展开后逐个转换，按请求格式写入输出目录。
async fn markscan_multi(
    ctx: &Ctx<'_>,
    client: &crate::client::QyzsClient,
    opts: MarkScanOptions,
) -> QyzsResult<()> {
    if opts.dry_run {
        let plans = multi::plan_multi(&opts, ctx.config.timeout_seconds).await?;
        if ctx.config.output_format == OutputFormat::Json {
            println!(
                "{}",
                serde_json::to_string_pretty(&plans).unwrap_or_default()
            );
        } else {
            multi::render_plans_human(&plans);
        }
        return Ok(());
    }

    let total = multi::resolve_inputs(&opts)?.len();
    let results = multi::convert_multi(
        client,
        &opts,
        ctx.config.timeout_seconds,
        Some(&|done: usize, _total: usize| {
            ctx.printer
                .info(&crate::msgf!(MdSliceProgress, done, total));
        }),
    )
    .await?;
    for (r, _md) in &results {
        if let Some(out_p) = &r.output {
            ctx.printer
                .ok(&crate::msgf!(MdSaved, out_p.display().to_string()));
        }
    }
    Ok(())
}

fn is_url_input(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

pub async fn pdf_split(
    ctx: &Ctx<'_>,
    file: String,
    pages: Option<String>,
    output: Option<String>,
    chunk_size: Option<usize>,
    out_dir: Option<String>,
) -> QyzsResult<()> {
    let meta = PdfExtractor::read_metadata(&file)?;
    if let Some(chunk) = chunk_size {
        let target_dir = out_dir.unwrap_or_else(|| "./split_output".to_string());
        let files = PdfSplitter::chunk(&file, chunk, &target_dir)?;
        ctx.printer.ok(&format!(
            "文档 (共 {} 页) 已按每 {} 页分块拆分完成，生成 {} 个文件:",
            meta.page_count,
            chunk,
            files.len()
        ));
        for f in files {
            ctx.printer.dim(&format!("  -> {}", f.display()));
        }
    } else if let Some(page_range) = pages {
        let target_out = output.unwrap_or_else(|| "split_result.pdf".to_string());
        let selected = PdfSplitter::parse_page_range(&page_range, meta.page_count as u32)?;
        PdfSplitter::split(&file, &selected, &target_out)?;
        ctx.printer.ok(&format!(
            "提取页码 [{}] (共 {} 页) 并保存至: {}",
            page_range,
            selected.len(),
            target_out
        ));
    } else {
        return Err(QyzsError::Usage(
            "请指定 --pages 或 --chunk-size 参数".to_string(),
        ));
    }
    Ok(())
}

pub async fn verify(ctx: &Ctx<'_>, ocr: String, markdown: String) -> QyzsResult<()> {
    let ocr_content = if Path::new(&ocr).exists() {
        std::fs::read_to_string(&ocr)?
    } else {
        ocr
    };

    let md_content = if Path::new(&markdown).exists() {
        std::fs::read_to_string(&markdown)?
    } else {
        markdown
    };

    let report = OcrVerifier::verify(&ocr_content, &md_content);
    if report.passed {
        ctx.printer.ok(&format!(
            "同源互验通过 (Jaccard 相似度: {:.2}%)",
            report.jaccard_similarity * 100.0
        ));
        ctx.printer.dim(&format!(
            "  OCR 字符: {}, Markdown 字符: {}",
            report.ocr_char_count, report.mineru_char_count
        ));
        ctx.printer.dim(&format!("  仲裁结论: {}", report.notes));
    } else {
        ctx.printer.warn(&format!(
            "同源互验未达阈值 (Jaccard 相似度: {:.2}%)",
            report.jaccard_similarity * 100.0
        ));
        ctx.printer.dim(&format!("  建议: {}", report.notes));
    }
    Ok(())
}

pub async fn dataset(ctx: &Ctx<'_>, url: String, dest: String) -> QyzsResult<()> {
    ctx.printer.info(crate::msg!(DatasetDownloading));
    let count = DatasetManager::download_and_extract(&url, &dest).await?;
    ctx.printer.ok(&format!(
        "{} (共解压 {} 个基准文件到: {})",
        crate::msg!(DatasetSuccess),
        count,
        dest
    ));
    Ok(())
}

fn print_scan_plan(printer: &crate::term::Printer, plan: &ConversionPlan) {
    let styler = printer.styler();
    println!("{}", styler.bold(crate::msg!(MdPlanHeader)));
    println!(
        "{}{} ({}, {})",
        crate::msg!(MdLabelInput),
        plan.input,
        if plan.input_source == "url" {
            crate::msg!(MdInputUrl)
        } else {
            crate::msg!(MdInputLocal)
        },
        crate::pipeline::markscan::human_size(plan.size_bytes)
    );
    println!(
        "{}{} ({})",
        crate::msg!(MdLabelFormat),
        plan.detected_label,
        match plan.detect_method {
            "content-sniff" => crate::msg!(MdDetectContent),
            "extension-fallback" => crate::msg!(MdDetectExtension),
            _ => crate::msg!(MdDetectManual),
        }
    );
    if let (Some(pages), Some(slices)) = (plan.page_count, plan.slice_count) {
        println!(
            "{}",
            crate::msgf!(
                MdLabelPdfPages,
                pages,
                slices,
                crate::pipeline::markscan::pdf::SLICE_BATCH_PAGES
            )
        );
    }
    println!("{}{}", crate::msg!(MdLabelMode), plan.mode);
    println!(
        "{}{}",
        crate::msg!(MdLabelChannel),
        plan.channel.unwrap_or("—")
    );
    if plan.supported {
        println!("{}{}", crate::msg!(MdLabelStages), plan.stages.join(" → "));
    } else {
        printer.err(crate::msg!(MdUnsupportedStatus));
        println!(
            "{}{}",
            crate::msg!(MdReason),
            plan.unsupported_reason.as_deref().unwrap_or("—")
        );
    }
    if plan.mode != "speed" {
        println!("{}{}", crate::msg!(MdPolishModel), plan.polish_model);
    }
    println!(
        "{}{} ({})",
        crate::msg!(MdOutputTarget),
        plan.output_target,
        plan.output_format
    );
    for w in &plan.warnings {
        printer.warn(&format!("  ⚠ {}", w));
    }
}
