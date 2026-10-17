//! 模型调用直通层处理器 (chat/ocr/parse/embed/rerank/asr/sam)

use super::Ctx;
use crate::client::{ChatOptions, ChatResponse};
use crate::error::QyzsResult;
use crate::term::OutputFormat;
use std::io::Write;

/// chat 命令参数聚合 (字段较多, 显式具名构造)
pub struct ChatArgs {
    pub prompt: String,
    pub model: Option<String>,
    pub system: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<u32>,
    pub top_p: Option<f64>,
    pub stream: bool,
    pub no_stream: bool,
    pub reasoning: bool,
    pub images: Vec<String>,
}

pub async fn chat(ctx: &Ctx<'_>, args: ChatArgs) -> QyzsResult<()> {
    let ChatArgs {
        prompt,
        model,
        system,
        temperature,
        max_tokens,
        top_p,
        stream,
        no_stream,
        reasoning,
        images,
    } = args;

    // qwen3-VL-30B-A3B 实测 16 图为可靠工作上限 (QYZS-ANALYZE.md §3.6)，>16 图自动分批
    const VLM_MAX_IMAGES: usize = 16;

    let use_stream = if no_stream { false } else { stream };
    let image_batches: Vec<Vec<String>> = if images.len() > VLM_MAX_IMAGES {
        images.chunks(VLM_MAX_IMAGES).map(<[String]>::to_vec).collect()
    } else {
        vec![images]
    };
    let batch_count = image_batches.len();

    if batch_count > 1 {
        ctx.printer.info(&format!(
            "共 {} 张图片，超过单请求 16 图可靠上限，已自动分批为 {} 个子请求",
            image_batches.iter().map(Vec::len).sum::<usize>(),
            batch_count
        ));
    }

    let mut batch_responses: Vec<ChatResponse> = Vec::new();
    for (batch_idx, images) in image_batches.into_iter().enumerate() {
        if batch_count > 1 && ctx.config.output_format != OutputFormat::Json {
            let first = batch_idx * VLM_MAX_IMAGES + 1;
            ctx.printer.info(&format!(
                "── 批次 {}/{}（第 {}~{} 张图）──",
                batch_idx + 1,
                batch_count,
                first,
                first + images.len() - 1
            ));
        }

        let opts = ChatOptions {
            model: model.clone(),
            temperature,
            max_tokens,
            top_p,
            stream: use_stream && batch_count == 1,
            show_reasoning: reasoning,
            images,
        };

        if use_stream && batch_count == 1 && ctx.config.output_format != OutputFormat::Json {
            let styler = ctx.printer.styler();
            ctx.client
                .chat_stream(&prompt, system.as_deref(), opts, |delta, is_reasoning| {
                    if is_reasoning {
                        print!("{}", styler.dim(delta));
                    } else {
                        print!("{}", delta);
                    }
                    std::io::stdout().flush().ok();
                    Ok(())
                })
                .await?;
            println!();
        } else {
            let resp = ctx.client.chat(&prompt, system.as_deref(), opts).await?;
            if batch_count > 1 && ctx.config.output_format == OutputFormat::Json {
                batch_responses.push(resp);
                continue;
            }
            match ctx.config.output_format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&resp).unwrap_or_default());
                }
                _ => {
                    if let Some(c) = resp.choices.first() {
                        if let Some(msg) = &c.message {
                            if reasoning {
                                if let Some(r) = &msg.reasoning {
                                    ctx.printer.dim(&format!("[Thinking]\n{}\n", r));
                                }
                            }
                            if let Some(content) = &msg.content {
                                println!("{}", content);
                            }
                        }
                    }
                }
            }
        }
    }

    if !batch_responses.is_empty() {
        println!(
            "{}",
            serde_json::to_string_pretty(&batch_responses).unwrap_or_default()
        );
    }
    Ok(())
}

pub async fn ocr(
    ctx: &Ctx<'_>,
    image: String,
    prompt: Option<String>,
    max_dim: Option<u32>,
) -> QyzsResult<()> {
    let resp = ctx.client.ocr(&image, prompt.as_deref(), max_dim).await?;
    match ctx.config.output_format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&resp.raw_response).unwrap_or_default()
            );
        }
        _ => {
            println!("{}", resp.text);
        }
    }
    Ok(())
}

pub async fn parse(ctx: &Ctx<'_>, file: String, is_ocr: bool) -> QyzsResult<()> {
    let md = ctx.client.parse_document(&file, is_ocr).await?;
    println!("{}", md);
    Ok(())
}

pub async fn embed(ctx: &Ctx<'_>, text: Vec<String>, model: Option<String>) -> QyzsResult<()> {
    let resp = ctx.client.embed(text, model.as_deref()).await?;
    match ctx.config.output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&resp).unwrap_or_default());
        }
        _ => {
            for item in &resp.data {
                ctx.printer
                    .info(&format!("Index {}: 向量维度 {}", item.index, item.embedding.len()));
                if let Some(first) = item.embedding.first() {
                    ctx.printer.dim(&format!("  [{}, ...]", first));
                }
            }
        }
    }
    Ok(())
}

pub async fn rerank(
    ctx: &Ctx<'_>,
    query: String,
    document: Vec<String>,
    top_n: Option<usize>,
    model: Option<String>,
) -> QyzsResult<()> {
    let resp = ctx
        .client
        .rerank(&query, document, model.as_deref(), top_n)
        .await?;
    match ctx.config.output_format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&resp).unwrap_or_default());
        }
        _ => {
            for (rank, item) in resp.results.iter().enumerate() {
                let score_str = format!("Top {} (分数: {:.4})", rank + 1, item.relevance_score);
                ctx.printer.ok(&score_str);
                if let Some(doc) = &item.document {
                    ctx.printer.dim(&format!("  内容: {}", doc));
                }
            }
        }
    }
    Ok(())
}

pub async fn asr(ctx: &Ctx<'_>, audio: String, model: Option<String>) -> QyzsResult<()> {
    let text = ctx.client.transcribe_audio(&audio, model.as_deref()).await?;
    println!("{}", text);
    Ok(())
}

pub async fn sam(
    ctx: &Ctx<'_>,
    before: String,
    after: String,
    aoi: Option<String>,
    threshold: Option<f64>,
) -> QyzsResult<()> {
    let aoi_content = if let Some(a) = aoi {
        let p = std::path::Path::new(&a);
        if p.exists() {
            Some(std::fs::read_to_string(p)?)
        } else {
            Some(a)
        }
    } else {
        None
    };

    let res = ctx
        .client
        .run_remote_sensing(&before, &after, aoi_content.as_deref(), threshold)
        .await?;
    println!("{}", res);
    Ok(())
}
