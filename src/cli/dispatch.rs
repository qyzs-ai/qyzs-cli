//! 命令行子命令执行与分发路由器
//!
//! 只负责: 参数解析 → 级联配置 → 处理器映射。业务逻辑在 `handlers` 按域分组。

use super::args::{Cli, Commands};
use super::validate;
use crate::client::QyzsClient;
use crate::config::ResolvedConfig;
use crate::error::{QyzsError, QyzsResult};
use crate::pipeline::scan::{ScanOptions, PipelineMode};
use crate::term::{set_language, Printer};
use super::handlers::{self, Ctx, ChatArgs};
use clap::Parser;

pub async fn run(args: Vec<String>) -> QyzsResult<()> {
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let code = e.exit_code();
            if code == 0 {
                println!("{}", e);
                std::process::exit(0);
            } else {
                eprintln!("{}", e);
                std::process::exit(code);
            }
        }
    };

    // 级联配置解析 (四层优先级)
    let config = ResolvedConfig::resolve(
        cli.api_key.as_deref(),
        cli.base_url.as_deref(),
        cli.timeout,
        cli.lang.as_deref(),
        cli.model_lang.as_deref(),
        cli.color.as_deref(),
        cli.format.as_deref(),
    )?;

    // 设置全局语言与终端打印器
    set_language(config.ui_language);
    let printer = Printer::new(config.color_choice);
    let client = QyzsClient::new(config.clone())?;

    let ctx = Ctx {
        config,
        printer: &printer,
        client: &client,
    };

    match cli.command {
        Commands::Chat {
            prompt,
            model,
            system,
            temperature,
            max_tokens,
            top_p,
            stream,
            no_stream,
            reasoning,
            image,
        } => {
            // 非空校验: clap 不拒空字符串位置参数, dispatch 层兜住
            if prompt.trim().is_empty() {
                return Err(QyzsError::Usage(
                    "prompt 不能为空字符串或仅含空白".to_string(),
                ));
            }
            // 数值边界校验 (dispatch 层兜住: 与现有 min_confidence 风格一致)
            validate::validate_chat_temperature(temperature)?;
            validate::validate_chat_top_p(top_p)?;
            validate::validate_chat_max_tokens(max_tokens)?;
            validate::validate_chat_images(&image)?;
            handlers::inference::chat(
                &ctx,
                ChatArgs {
                    prompt,
                    model,
                    system,
                    temperature,
                    max_tokens,
                    top_p,
                    stream,
                    no_stream,
                    reasoning,
                    images: image,
                },
            )
            .await?
        }

        Commands::Ocr { image, prompt, max_dim } => {
            validate::validate_ocr_max_dim(max_dim)?;
            handlers::inference::ocr(&ctx, image, prompt, max_dim).await?
        }

        Commands::Embed { text, model } => handlers::inference::embed(&ctx, text, model).await?,

        Commands::Rerank {
            query,
            document,
            top_n,
            model,
        } => {
            validate::validate_rerank_top_n(top_n)?;
            handlers::inference::rerank(&ctx, query, document, top_n, model).await?
        }

        Commands::Asr { audio, model } => handlers::inference::asr(&ctx, audio, model).await?,

        Commands::Sam {
            before,
            after,
            aoi,
            threshold,
        } => {
            validate::validate_sam_threshold(threshold)?;
            handlers::inference::sam(&ctx, before, after, aoi, threshold).await?
        }

        Commands::Scan {
            input,
            output,
            recursive,
            mode,
            r#type,
            image_handling,
            model,
            min_confidence,
            report,
            work_dir,
            segment_chars,
            resume,
            no_clean,
            no_front_matter,
            dry_run,
            assets_dir,
            no_assets,
            no_classify,
            classify_model,
        } => {
            // 段长上界校验 (scan 的软警告从 mod.rs 提升为 dispatch 硬拒)
            validate::validate_scan_segment_chars(segment_chars)?;
            let opts = ScanOptions {
                input,
                output,
                recursive,
                mode: match mode.as_deref() {
                    Some(s) => PipelineMode::parse(s)?,
                    None => PipelineMode::Balanced,
                },
                type_override: match r#type.as_deref() {
                    Some(s) => Some(crate::pipeline::scan::detect::SourceFormat::parse(s)?),
                    None => None,
                },
                image_handling: match image_handling.as_deref() {
                    Some(s) => crate::pipeline::scan::ImageHandling::parse(s)?,
                    None => crate::pipeline::scan::ImageHandling::Auto,
                },
                model,
                min_confidence: match min_confidence {
                    Some(c) if !(0.0..=1.0).contains(&c) => {
                        return Err(QyzsError::Usage(format!(
                            "--min-confidence 取值必须在 0.0~1.0 之间，当前: {}",
                            c
                        )));
                    }
                    other => other,
                },
                report,
                work_dir,
                segment_chars,
                resume,
                no_clean,
                no_front_matter,
                dry_run,
                assets_dir,
                no_assets,
                no_classify,
                classify_model,
            };
            handlers::documents::scan(&ctx, opts).await?
        }

        Commands::PdfSplit {
            file,
            pages,
            output,
            chunk_size,
            out_dir,
        } => {
            validate::validate_pdf_split_chunk_size(chunk_size)?;
            handlers::documents::pdf_split(&ctx, file, pages, output, chunk_size, out_dir)
                .await?
        }

        Commands::Verify { ocr, markdown } => handlers::documents::verify(&ctx, ocr, markdown).await?,

        Commands::Dataset { url, dest } => handlers::documents::dataset(&ctx, url, dest).await?,

        Commands::Status { json, no_probe } => handlers::system::status(&ctx, json, no_probe).await?,

        Commands::Config { action } => handlers::system::config(&ctx, action)?,

        Commands::Auth { action } => handlers::system::auth(&ctx, action)?,

        Commands::Models {
            online_only: _,
            all_models: _,
        } => handlers::system::models(&ctx)?,

        Commands::Usage {
            days,
            model,
            kind,
            key,
            hourly,
            json,
            clear,
        } => {
            validate::validate_usage_days(days)?;
            validate::validate_usage_kind(kind.as_deref())?;
            handlers::system::usage(
                &ctx,
                handlers::system::UsageArgs {
                    days,
                    model,
                    kind,
                    key,
                    hourly,
                    json,
                    clear,
                },
            )?
        }

        Commands::Upgrade { repo, package, check } => {
            handlers::system::upgrade(&ctx, repo, package, check).await?
        }

        Commands::Completions { shell } => handlers::system::completions(shell),
    }

    Ok(())
}
