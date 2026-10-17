//! 命令行子命令执行与分发路由器

use super::args::{AuthAction, Cli, Commands, ConfigAction};
use super::completions::CompletionGenerator;
use crate::client::{ChatOptions, ChatResponse, QyzsClient};
use crate::config::{Config, Credentials, ResolvedConfig};
use crate::error::{QyzsError, QyzsResult};
use crate::pdf::{PdfExtractor, PdfSplitter};
use crate::pipeline::{CrossVerifier, DatasetManager, MarkitdownPipeline};
use crate::term::{set_language, OutputFormat, Printer};
use crate::updater::UpdateManager;
use clap::Parser;
use std::io::Write;
use std::path::Path;

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
            // qwen3-VL-30B-A3B 实测 16 图为可靠工作上限 (QYZS-ANALYZE.md §3.6)，>16 图自动分批
            const VLM_MAX_IMAGES: usize = 16;

            let use_stream = if no_stream { false } else { stream };
            let image_batches: Vec<Vec<String>> = if image.len() > VLM_MAX_IMAGES {
                image.chunks(VLM_MAX_IMAGES).map(<[String]>::to_vec).collect()
            } else {
                vec![image]
            };
            let batch_count = image_batches.len();

            if batch_count > 1 {
                printer.info(&format!(
                    "共 {} 张图片，超过单请求 16 图可靠上限，已自动分批为 {} 个子请求",
                    image_batches.iter().map(Vec::len).sum::<usize>(),
                    batch_count
                ));
            }

            let mut batch_responses: Vec<ChatResponse> = Vec::new();
            for (batch_idx, images) in image_batches.into_iter().enumerate() {
                if batch_count > 1 && config.output_format != OutputFormat::Json {
                    let first = batch_idx * VLM_MAX_IMAGES + 1;
                    printer.info(&format!(
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

                if use_stream && batch_count == 1 && config.output_format != OutputFormat::Json {
                    let styler = printer.styler();
                    client
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
                    let resp = client.chat(&prompt, system.as_deref(), opts).await?;
                    if batch_count > 1 && config.output_format == OutputFormat::Json {
                        batch_responses.push(resp);
                        continue;
                    }
                    match config.output_format {
                        OutputFormat::Json => {
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&resp).unwrap_or_default()
                            );
                        }
                        _ => {
                            if let Some(c) = resp.choices.first() {
                                if let Some(msg) = &c.message {
                                    if reasoning {
                                        if let Some(r) = &msg.reasoning {
                                            printer.dim(&format!("[Thinking]\n{}\n", r));
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
        }

        Commands::Ocr { image, prompt, max_dim } => {
            let resp = client.ocr(&image, prompt.as_deref(), max_dim).await?;
            match config.output_format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&resp.raw_response).unwrap_or_default());
                }
                _ => {
                    println!("{}", resp.text);
                }
            }
        }

        Commands::Parse { file, is_ocr } => {
            let md = client.parse_document(&file, is_ocr).await?;
            println!("{}", md);
        }

        Commands::Embed { text, model } => {
            let resp = client.embed(text, model.as_deref()).await?;
            match config.output_format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&resp).unwrap_or_default());
                }
                _ => {
                    for item in &resp.data {
                        printer.info(&format!("Index {}: 向量维度 {}", item.index, item.embedding.len()));
                        if let Some(first) = item.embedding.first() {
                            printer.dim(&format!("  [{}, ...]", first));
                        }
                    }
                }
            }
        }

        Commands::Rerank {
            query,
            document,
            top_n,
            model,
        } => {
            let resp = client.rerank(&query, document, model.as_deref(), top_n).await?;
            match config.output_format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&resp).unwrap_or_default());
                }
                _ => {
                    for (rank, item) in resp.results.iter().enumerate() {
                        let score_str = format!("Top {} (分数: {:.4})", rank + 1, item.relevance_score);
                        printer.ok(&score_str);
                        if let Some(doc) = &item.document {
                            printer.dim(&format!("  内容: {}", doc));
                        }
                    }
                }
            }
        }

        Commands::Asr { audio, model } => {
            let text = client.transcribe_audio(&audio, model.as_deref()).await?;
            println!("{}", text);
        }

        Commands::Sam {
            before,
            after,
            aoi,
            threshold,
        } => {
            let aoi_content = if let Some(a) = aoi {
                let p = Path::new(&a);
                if p.exists() {
                    Some(std::fs::read_to_string(p)?)
                } else {
                    Some(a)
                }
            } else {
                None
            };

            let res = client.run_remote_sensing(&before, &after, aoi_content.as_deref(), threshold).await?;
            println!("{}", res);
        }

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
            let mode_explicit = mode.is_some();
            let run_output = output.clone();
            let opts = crate::pipeline::markitdown::MarkitdownOptions {
                input,
                output,
                mode: match mode.as_deref() {
                    Some(s) => crate::pipeline::markitdown::PipelineMode::parse(s)?,
                    None => crate::pipeline::markitdown::PipelineMode::Balanced,
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

            if dry_run {
                let plan = MarkitdownPipeline::plan(&opts, config.timeout_seconds).await?;
                if config.output_format == OutputFormat::Json {
                    println!("{}", serde_json::to_string_pretty(&plan).unwrap_or_default());
                } else {
                    print_markitdown_plan(&printer, &plan);
                }
            } else {
                if !mode_explicit {
                    printer.info("默认 balanced 档 (单通道 + dsv4 单轮排版润色 + 轻量质检)");
                }
                let md = MarkitdownPipeline::convert(
                    &client,
                    &opts,
                    config.timeout_seconds,
                    Some(&|done: usize, total: usize| {
                        if total > 1 {
                            printer.info(&format!("── 片段进度 {}/{} ──", done, total));
                        }
                    }),
                )
                .await?;
                if let Some(out_p) = run_output {
                    std::fs::write(&out_p, &md)?;
                    printer.ok(&format!("Markdown 结果已保存至: {}", out_p));
                } else {
                    println!("{}", md);
                }
            }
        }

        Commands::PdfSplit {
            file,
            pages,
            output,
            chunk_size,
            out_dir,
        } => {
            let meta = PdfExtractor::read_metadata(&file)?;
            if let Some(chunk) = chunk_size {
                let target_dir = out_dir.unwrap_or_else(|| "./split_output".to_string());
                let files = PdfSplitter::chunk(&file, chunk, &target_dir)?;
                printer.ok(&format!("文档 (共 {} 页) 已按每 {} 页分块拆分完成，生成 {} 个文件:", meta.page_count, chunk, files.len()));
                for f in files {
                    printer.dim(&format!("  -> {}", f.display()));
                }
            } else if let Some(page_range) = pages {
                let target_out = output.unwrap_or_else(|| "split_result.pdf".to_string());
                let selected = PdfSplitter::parse_page_range(&page_range, meta.page_count as u32)?;
                PdfSplitter::split(&file, &selected, &target_out)?;
                printer.ok(&format!("提取页码 [{}] (共 {} 页) 并保存至: {}", page_range, selected.len(), target_out));
            } else {
                return Err(QyzsError::Usage("请指定 --pages 或 --chunk-size 参数".to_string()));
            }
        }

        Commands::Verify { ocr, markdown } => {
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

            let report = CrossVerifier::verify(&ocr_content, &md_content);
            if report.passed {
                printer.ok(&format!("同源互验通过 (Jaccard 相似度: {:.2}%)", report.jaccard_similarity * 100.0));
                printer.dim(&format!("  OCR 字符: {}, Markdown 字符: {}", report.ocr_char_count, report.mineru_char_count));
                printer.dim(&format!("  仲裁结论: {}", report.notes));
            } else {
                printer.warn(&format!("同源互验未达阈值 (Jaccard 相似度: {:.2}%)", report.jaccard_similarity * 100.0));
                printer.dim(&format!("  建议: {}", report.notes));
            }
        }

        Commands::Dataset { url, dest } => {
            printer.info(crate::msg!(DatasetDownloading));
            let count = DatasetManager::download_and_extract(&url, &dest).await?;
            printer.ok(&format!("{} (共解压 {} 个基准文件到: {})", crate::msg!(DatasetSuccess), count, dest));
        }

        Commands::Status { json } => {
            let probes = client.run_probes().await;
            let creds_path = Credentials::default_path();
            let is_secure = Credentials::is_permission_secure(&creds_path);
            let is_npm = UpdateManager::is_running_under_npm();

            if json {
                let status_json = serde_json::json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "os": std::env::consts::OS,
                    "arch": std::env::consts::ARCH,
                    "install_mode": if is_npm { "npm" } else { "standalone" },
                    "config_path": Config::default_path().to_string_lossy(),
                    "credentials_path": creds_path.to_string_lossy(),
                    "credentials_secure": is_secure,
                    "api_key_source": config.api_key_source,
                    "api_key_masked": Credentials::mask_key(&config.api_key),
                    "probes": {
                        "public_api": { "ok": probes.public_api.ok, "status": probes.public_api.status_code, "latency_ms": probes.public_api.latency_ms },
                        "mineru_api": { "ok": probes.mineru_api.ok, "status": probes.mineru_api.status_code, "latency_ms": probes.mineru_api.latency_ms },
                        "sam_api": { "ok": probes.sam_api.ok, "status": probes.sam_api.status_code, "latency_ms": probes.sam_api.latency_ms }
                    }
                });
                println!("{}", serde_json::to_string_pretty(&status_json).unwrap_or_default());
            } else {
                let styler = printer.styler();
                println!("{}", styler.bold(&format!("[{}]", crate::msg!(StatusDiagnosticsHeader))));
                println!("  Version:       {} ({}/{}, pure-rustls)", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
                println!("  Install Mode:  {}", if is_npm { "npm Global Package" } else { "Standalone Binary" });
                println!("  Config File:   {} [OK]", Config::default_path().display());
                println!("  Credentials:   {} [{}]", creds_path.display(), if is_secure { "OK: 0600 secure" } else { "WARN: wide permissions" });
                println!("  Active Key:    {} (Source: {})", Credentials::mask_key(&config.api_key), config.api_key_source);

                println!("\n{}", styler.bold(&format!("[{}]", crate::msg!(StatusGatewayHeader))));
                println!("  Gateway Origin: {}", config.base_url);
                print_probe_line(&printer, &probes.public_api);
                print_probe_line(&printer, &probes.mineru_api);
                print_probe_line(&printer, &probes.sam_api);

                if probes.all_ok {
                    println!("\n{}", styler.green(&format!("[STATUS] {}", crate::msg!(AllSystemsOperational))));
                } else {
                    println!("\n{}", styler.yellow(&format!("[STATUS] {}", crate::msg!(StatusDegraded))));
                }
            }
        }

        Commands::Config { action } => match action {
            ConfigAction::Get { key } => {
                let conf = Config::load()?;
                match key.as_str() {
                    "network.base_url" => println!("{}", conf.network.base_url),
                    "network.timeout" => println!("{}", conf.network.timeout),
                    "ui.language" => println!("{}", conf.ui.language),
                    "ui.color" => println!("{}", conf.ui.color),
                    "ui.format" => println!("{}", conf.ui.format),
                    "model.language" => println!("{}", conf.model.language),
                    "model.default_chat" => println!("{}", conf.model.default_chat),
                    "model.default_ocr" => println!("{}", conf.model.default_ocr),
                    "model.default_parse" => println!("{}", conf.model.default_parse),
                    "model.default_embed" => println!("{}", conf.model.default_embed),
                    "model.default_rerank" => println!("{}", conf.model.default_rerank),
                    _ => {
                        let toml_val = toml::Value::try_from(&conf).map_err(|e| QyzsError::Config(e.to_string()))?;
                        println!("{}", toml_val);
                    }
                }
            }
            ConfigAction::Set { key, value } => {
                let mut conf = Config::load().unwrap_or_default();
                match key.as_str() {
                    "network.base_url" => conf.network.base_url = value.clone(),
                    "network.timeout" => conf.network.timeout = value.parse().map_err(|_| QyzsError::Usage("timeout 必须为整数".to_string()))?,
                    "ui.language" => conf.ui.language = value.clone(),
                    "ui.color" => conf.ui.color = value.clone(),
                    "ui.format" => conf.ui.format = value.clone(),
                    "model.language" => conf.model.language = value.clone(),
                    "model.default_chat" => conf.model.default_chat = value.clone(),
                    "model.default_ocr" => conf.model.default_ocr = value.clone(),
                    "model.default_parse" => conf.model.default_parse = value.clone(),
                    "model.default_embed" => conf.model.default_embed = value.clone(),
                    "model.default_rerank" => conf.model.default_rerank = value.clone(),
                    _ => return Err(QyzsError::Usage(format!("未知配置项: '{}'", key))),
                }
                conf.save()?;
                printer.ok(&format!("{}: {} = {}", crate::msg!(ConfigSaved), key, value));
            }
            ConfigAction::Path => {
                println!("Config:      {}", Config::default_path().display());
                println!("Credentials: {}", Credentials::default_path().display());
            }
            ConfigAction::Reset => {
                Config::reset()?;
                printer.ok(crate::msg!(ConfigReset));
            }
        },

        Commands::Auth { action } => match action {
            AuthAction::Login { key } => {
                let api_key = if let Some(k) = key {
                    k
                } else {
                    print!("请输入庆阳智算中心 API Key (sk-...): ");
                    std::io::stdout().flush().ok();
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)?;
                    input.trim().to_string()
                };

                if api_key.is_empty() {
                    return Err(QyzsError::Usage("API Key 不能为空".to_string()));
                }

                let creds = Credentials { api_key: api_key.clone() };
                creds.save()?;
                printer.ok(&format!("{}: {}", crate::msg!(AuthLoginSuccess), Credentials::mask_key(&api_key)));
            }
            AuthAction::Status => {
                let creds_path = Credentials::default_path();
                let secure = Credentials::is_permission_secure(&creds_path);
                printer.info(&format!("生效 API Key: {} (来源: {})", Credentials::mask_key(&config.api_key), config.api_key_source));
                printer.dim(&format!("凭据文件路径: {}", creds_path.display()));
                if !secure {
                    printer.warn(crate::msg!(AuthFilePermWarning));
                } else {
                    printer.ok("凭据权限安全 (0600)");
                }
            }
            AuthAction::Logout => {
                let purged = Credentials::purge()?;
                if purged {
                    printer.ok(crate::msg!(AuthLogoutSuccess));
                } else {
                    printer.dim("本地原本不存在凭据文件");
                }
            }
        },

        Commands::Models { online_only: _ } => {
            print_models_matrix(&printer);
        }

        Commands::Upgrade { repo, package } => {
            if let Some(pkg) = package {
                UpdateManager::install_from_local_package(&pkg)?;
                printer.ok(&format!("已从本地安装包 '{}' 完成就地安装/更新！", pkg));
            } else {
                printer.info(crate::msg!(SelfUpdateChecking));
                let updated = UpdateManager::self_update(&repo).await?;
                if updated {
                    printer.ok(crate::msg!(SelfUpdateSuccess));
                } else {
                    printer.info(crate::msg!(SelfUpdateUpToDate));
                }
            }
        }

        Commands::Completions { shell } => {
            CompletionGenerator::generate(shell);
        }
    }

    Ok(())
}

fn print_probe_line(printer: &Printer, probe: &crate::client::probe::ProbeResult) {
    if probe.ok {
        printer.ok(&format!("{:<32} {:>3} OK  ({}ms)", probe.name, probe.status_code, probe.latency_ms));
    } else {
        printer.err(&format!("{:<32} {:>3} ERR ({}ms)", probe.name, probe.status_code, probe.latency_ms));
    }
}

fn print_markitdown_plan(
    printer: &Printer,
    plan: &crate::pipeline::markitdown::ConversionPlan,
) {
    let styler = printer.styler();
    println!("{}", styler.bold("[markitdown] 流水线规划预览 (--dry-run)"));
    println!(
        "  输入:        {} ({}, {})",
        plan.input,
        if plan.input_source == "url" { "网络下载" } else { "本地文件" },
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
        println!(
            "  PDF 页数:    {} 页 → {} 批切片 (每批 ≤15 页)",
            pages, slices
        );
    }
    println!("  处理档位:    {}", plan.mode);
    println!(
        "  规划通道:    {}",
        plan.channel.unwrap_or("—")
    );
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

fn print_models_matrix(printer: &Printer) {
    let styler = printer.styler();
    println!("{}", styler.bold("慶陽智算中心全量高可靠可用模型矩陣 (SSOT 校准真值)"));
    println!("{:<20} {:<10} {:<10} {}", "模型代碼", "能力分類", "實測延時", "特性與場景");
    println!("{}", "-".repeat(75));
    println!("{:<20} {:<10} {:<10} {}", "dsv4", "通用對話", "1.02s", "極速響應，實測 51.8萬 Tokens 100% 召回 (默認主模型)");
    println!("{:<20} {:<10} {:<10} {}", "qwen3.6-27b", "深度推理", "6.02s", "具思維鏈深度推理能力，實測 13.8萬 Tokens 100% 召回");
    println!("{:<20} {:<10} {:<10} {}", "qwen3-VL-30B-A3B", "視覺多模態", "5.62s", "16 圖可靠並發（>16 圖需分批），文本硬限 131,072 Tokens");
    println!("{:<20} {:<10} {:<10} {}", "paddleocrvl", "公文OCR", "1.95s", "毫秒級文字轉錄，限制 <=800px 自動縮放");
    println!("{:<20} {:<10} {:<10} {}", "mineru3.2.3", "文檔解析", "0.59s", "PDF/Word(docx) 一鍵結構化輸出 Markdown 大綱與表格");
    println!("{:<20} {:<10} {:<10} {}", "bge-m3", "向量表徵", "0.27s", "1024 維密集向量，知識庫檢索首選");
    println!("{:<20} {:<10} {:<10} {}", "qwen3-embadding-8b", "超大向量", "0.39s", "4096 維超高分辨率語義向量");
    println!("{:<20} {:<10} {:<10} {}", "qwen3-reranker-8b", "語義重排", "0.28s", "Top1 > 0.90 置信度打分，精排首選");
    println!("{:<20} {:<10} {:<10} {}", "Qwen3-ASR-1.7B", "語音識別", "0.89s", "16kHz 語音直接轉寫");
    println!("{:<20} {:<10} {:<10} {}", "acid-sam3-full", "遙感變化", "異步表單", "雙時相遙感大模型變化檢測流水線");
}
