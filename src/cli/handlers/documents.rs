//! 文档流水线处理器 (markitdown/pdf-split/verify/dataset)

use super::Ctx;
use crate::error::{QyzsError, QyzsResult};
use crate::pdf::{PdfExtractor, PdfSplitter};
use crate::pipeline::markitdown::{ConversionPlan, MarkitdownOptions, MarkitdownPipeline};
use crate::pipeline::{DatasetManager, OcrVerifier};
use crate::term::OutputFormat;
use std::path::Path;

pub async fn markitdown(ctx: &Ctx<'_>, opts: MarkitdownOptions) -> QyzsResult<()> {
    let dry_run = opts.dry_run;
    let run_output = opts.output.clone();

    if dry_run {
        let plan = MarkitdownPipeline::plan(&opts, ctx.config.timeout_seconds).await?;
        if ctx.config.output_format == OutputFormat::Json {
            println!("{}", serde_json::to_string_pretty(&plan).unwrap_or_default());
        } else {
            print_markitdown_plan(ctx.printer, &plan);
        }
    } else {
        let md = MarkitdownPipeline::convert(
            ctx.client,
            &opts,
            ctx.config.timeout_seconds,
            Some(&|done: usize, total: usize| {
                if total > 1 {
                    ctx.printer.info(&format!("── 片段进度 {}/{} ──", done, total));
                }
            }),
        )
        .await?;
        if let Some(out_p) = run_output {
            std::fs::write(&out_p, &md)?;
            ctx.printer
                .ok(&format!("Markdown 结果已保存至: {}", out_p));
        } else {
            println!("{}", md);
        }
    }
    Ok(())
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
            meta.page_count, chunk, files.len()
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

fn print_markitdown_plan(printer: &crate::term::Printer, plan: &ConversionPlan) {
    let styler = printer.styler();
    println!(
        "{}",
        styler.bold("[markitdown] 流水线规划预览 (--dry-run)")
    );
    println!(
        "  输入:        {} ({}, {})",
        plan.input,
        if plan.input_source == "url" {
            "网络下载"
        } else {
            "本地文件"
        },
        crate::pipeline::markitdown::human_size(plan.size_bytes)
    );
    println!(
        "  识别格式:    {} ({})",
        plan.detected_label,
        match plan.detect_method {
            "content-sniff" => "内容嗅探",
            "extension-fallback" => "扩展名兜底",
            _ => "手动指定",
        }
    );
    if let (Some(pages), Some(slices)) = (plan.page_count, plan.slice_count) {
        println!("  PDF 页数:    {} 页 → {} 批切片 (每批 ≤15 页)", pages, slices);
    }
    println!("  处理档位:    {}", plan.mode);
    println!("  规划通道:    {}", plan.channel.unwrap_or("—"));
    if plan.supported {
        println!("  执行阶段:    {}", plan.stages.join(" → "));
    } else {
        printer.err("  状态:        ✗ 该格式暂不支持转换");
        println!(
            "  原因:        {}",
            plan.unsupported_reason.as_deref().unwrap_or("—")
        );
    }
    if plan.mode != "speed" {
        println!("  润色模型:    {}", plan.polish_model);
    }
    println!("  输出目标:    {}", plan.output_target);
    for w in &plan.warnings {
        printer.warn(&format!("  ⚠ {}", w));
    }
}
