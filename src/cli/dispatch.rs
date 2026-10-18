//! 命令行子命令执行与分发路由器
//!
//! 只负责: 参数解析 → 级联配置 → 处理器映射。业务逻辑在 `handlers` 按域分组。

use super::args::{Cli, Commands};
use crate::client::QyzsClient;
use crate::config::ResolvedConfig;
use crate::error::{QyzsError, QyzsResult};
use crate::pipeline::markitdown::{MarkitdownOptions, PipelineMode};
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
            handlers::inference::ocr(&ctx, image, prompt, max_dim).await?
        }

        Commands::Parse { file, is_ocr } => handlers::inference::parse(&ctx, file, is_ocr).await?,

        Commands::Embed { text, model } => handlers::inference::embed(&ctx, text, model).await?,

        Commands::Rerank {
            query,
            document,
            top_n,
            model,
        } => handlers::inference::rerank(&ctx, query, document, top_n, model).await?,

        Commands::Asr { audio, model } => handlers::inference::asr(&ctx, audio, model).await?,

        Commands::Sam {
            before,
            after,
            aoi,
            threshold,
        } => handlers::inference::sam(&ctx, before, after, aoi, threshold).await?,

        Commands::Markitdown {
            input,
            output,
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
            dry_run,
        } => {
            let opts = MarkitdownOptions {
                input,
                output,
                mode: match mode.as_deref() {
                    Some(s) => PipelineMode::parse(s)?,
                    None => PipelineMode::Balanced,
                },
                type_override: match r#type.as_deref() {
                    Some(s) => Some(crate::pipeline::markitdown::detect::SourceFormat::parse(s)?),
                    None => None,
                },
                image_handling: match image_handling.as_deref() {
                    Some(s) => crate::pipeline::markitdown::ImageHandling::parse(s)?,
                    None => crate::pipeline::markitdown::ImageHandling::Auto,
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
                dry_run,
            };
            handlers::documents::markitdown(&ctx, opts).await?
        }

        Commands::PdfSplit {
            file,
            pages,
            output,
            chunk_size,
            out_dir,
        } => {
            handlers::documents::pdf_split(&ctx, file, pages, output, chunk_size, out_dir)
                .await?
        }

        Commands::Verify { ocr, markdown } => handlers::documents::verify(&ctx, ocr, markdown).await?,

        Commands::Dataset { url, dest } => handlers::documents::dataset(&ctx, url, dest).await?,

        Commands::Status { json } => handlers::system::status(&ctx, json).await?,

        Commands::Config { action } => handlers::system::config(&ctx, action)?,

        Commands::Auth { action } => handlers::system::auth(&ctx, action)?,

        Commands::Models { online_only: _ } => handlers::system::models(&ctx)?,

        Commands::Usage {
            days,
            model,
            kind,
            key,
            hourly,
            json,
            clear,
        } => handlers::system::usage(
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
        )?,

        Commands::Upgrade { repo, package } => handlers::system::upgrade(&ctx, repo, package).await?,

        Commands::Completions { shell } => handlers::system::completions(shell),
    }

    Ok(())
}
