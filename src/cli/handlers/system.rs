//! 系统运维处理器 (status/config/auth/models/upgrade/completions)

use super::Ctx;
use crate::client::probe::ProbeResult;
use crate::config::{Config, Credentials};
use crate::error::{QyzsError, QyzsResult};
use crate::term::Printer;
use crate::updater::UpdateManager;
use std::io::Write;

pub async fn status(ctx: &Ctx<'_>, json: bool) -> QyzsResult<()> {
    let probes = ctx.client.run_probes().await;
    let creds_path = Credentials::default_path();
    let is_secure = Credentials::is_permission_secure(&creds_path);
    let is_npm = UpdateManager::is_running_under_npm();
    let config = &ctx.config;
    let printer = ctx.printer;

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
        println!(
            "{}",
            styler.bold(&format!("[{}]", crate::msg!(StatusDiagnosticsHeader)))
        );
        println!(
            "  Version:       {} ({}/{}, pure-rustls)",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        println!(
            "  Install Mode:  {}",
            if is_npm { "npm Global Package" } else { "Standalone Binary" }
        );
        println!("  Config File:   {} [OK]", Config::default_path().display());
        println!(
            "  Credentials:   {} [{}]",
            creds_path.display(),
            if is_secure { "OK: 0600 secure" } else { "WARN: wide permissions" }
        );
        println!(
            "  Active Key:    {} (Source: {})",
            Credentials::mask_key(&config.api_key),
            config.api_key_source
        );

        println!(
            "\n{}",
            styler.bold(&format!("[{}]", crate::msg!(StatusGatewayHeader)))
        );
        println!("  Gateway Origin: {}", config.base_url);
        print_probe_line(printer, &probes.public_api);
        print_probe_line(printer, &probes.mineru_api);
        print_probe_line(printer, &probes.sam_api);

        if probes.all_ok {
            println!(
                "\n{}",
                styler.green(&format!("[STATUS] {}", crate::msg!(AllSystemsOperational)))
            );
        } else {
            println!(
                "\n{}",
                styler.yellow(&format!("[STATUS] {}", crate::msg!(StatusDegraded)))
            );
        }
    }
    Ok(())
}

fn print_probe_line(printer: &Printer, probe: &ProbeResult) {
    if probe.ok {
        printer.ok(&format!(
            "{:<32} {:>3} OK  ({}ms)",
            probe.name, probe.status_code, probe.latency_ms
        ));
    } else {
        printer.err(&format!(
            "{:<32} {:>3} ERR ({}ms)",
            probe.name, probe.status_code, probe.latency_ms
        ));
    }
}

/// config set 写入前校验: 枚举类配置项复用 resolver 校验器, 与 CLI 参数同一套合法值集;
/// timeout 整数校验在赋值处, model.default_* 为模型名不设枚举限制
fn validate_config_value(key: &str, value: &str) -> QyzsResult<()> {
    match key {
        "network.base_url" => crate::config::resolver::validate_base_url(value),
        "ui.language" if value != "auto" => crate::term::i18n::language_from_tag(value)
            .map(|_| ())
            .ok_or_else(|| {
                QyzsError::Usage(format!("无效的界面语言 '{}': 仅支持 auto | zh | en", value))
            }),
        "ui.color" => crate::config::resolver::parse_color_choice(value).map(|_| ()),
        "ui.format" => value
            .parse::<crate::term::OutputFormat>()
            .map(|_| ())
            .map_err(QyzsError::Usage),
        "model.language" => crate::config::resolver::parse_model_language(value).map(|_| ()),
        _ => Ok(()),
    }
}

/// 解析 per-model 超参键 `model.params.<模型名>.<参数名>` 为 (归一化模型名, TOML 值)。
/// 模型名本身可含点号 (如 qwen3.6-27b), 故取前缀剥离 + 右侧最后一次分割;
/// 模型名按 get_model_param_* 的主查找形态归一 (./- → _)。参数值仅接受数值
/// (i64 优先, 其次 f64), 与现有消费点 (temperature/top_p/max_tokens/top_n/max_dim) 一致。
fn parse_model_params_entry(key: &str, value: &str) -> QyzsResult<(String, String, toml::Value)> {
    let rest = key
        .strip_prefix("model.params.")
        .ok_or_else(|| QyzsError::Usage(format!("未知配置项: '{}'", key)))?;
    let (model, param) = rest.rsplit_once('.').ok_or_else(|| {
        QyzsError::Usage(format!(
            "配置项 '{}' 格式应为 model.params.<模型名>.<参数名>",
            key
        ))
    })?;
    if model.is_empty() || param.is_empty() {
        return Err(QyzsError::Usage(format!(
            "配置项 '{}' 格式应为 model.params.<模型名>.<参数名>",
            key
        )));
    }
    let parsed = value
        .parse::<i64>()
        .map(toml::Value::Integer)
        .or_else(|_| value.parse::<f64>().map(toml::Value::Float))
        .map_err(|_| {
            QyzsError::Usage(format!("超参 '{}' 必须为数值 (当前: '{}')", param, value))
        })?;
    let normalized = model.replace('.', "_").replace('-', "_");
    Ok((normalized, param.to_string(), parsed))
}

pub fn config(
    ctx: &Ctx<'_>,
    action: crate::cli::args::ConfigAction,
) -> QyzsResult<()> {
    match action {
        crate::cli::args::ConfigAction::Get { key } => {
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
                    let toml_val =
                        toml::Value::try_from(&conf).map_err(|e| QyzsError::Config(e.to_string()))?;
                    println!("{}", toml_val);
                }
            }
        }
        crate::cli::args::ConfigAction::Set { key, value } => {
            // 写入前校验: 枚举类配置项与 CLI 参数共用同一套合法值集
            // (读取时仍 best-effort, 存量脏配置不阻断命令)
            validate_config_value(&key, &value)?;
            let mut conf = Config::load().unwrap_or_default();
            match key.as_str() {
                "network.base_url" => conf.network.base_url = value.clone(),
                "network.timeout" => conf.network.timeout = value
                    .parse()
                    .map_err(|_| QyzsError::Usage("timeout 必须为整数".to_string()))?,
                "ui.language" => conf.ui.language = value.clone(),
                "ui.color" => conf.ui.color = value.clone(),
                "ui.format" => conf.ui.format = value.clone(),
                "model.language" => conf.model.language = value.clone(),
                "model.default_chat" => conf.model.default_chat = value.clone(),
                "model.default_ocr" => conf.model.default_ocr = value.clone(),
                "model.default_parse" => conf.model.default_parse = value.clone(),
                "model.default_embed" => conf.model.default_embed = value.clone(),
                "model.default_rerank" => conf.model.default_rerank = value.clone(),
                key_str if key_str.starts_with("model.params.") => {
                    let (model, param, parsed) = parse_model_params_entry(key_str, &value)?;
                    conf.model
                        .params
                        .entry(model)
                        .or_default()
                        .insert(param, parsed);
                }
                _ => return Err(QyzsError::Usage(format!("未知配置项: '{}'", key))),
            }
            conf.save()?;
            ctx.printer
                .ok(&format!("{}: {} = {}", crate::msg!(ConfigSaved), key, value));
        }
        crate::cli::args::ConfigAction::Path => {
            println!("Config:      {}", Config::default_path().display());
            println!("Credentials: {}", Credentials::default_path().display());
        }
        crate::cli::args::ConfigAction::Reset => {
            Config::reset()?;
            ctx.printer.ok(crate::msg!(ConfigReset));
        }
    }
    Ok(())
}

pub fn auth(
    ctx: &Ctx<'_>,
    action: crate::cli::args::AuthAction,
) -> QyzsResult<()> {
    match action {
        crate::cli::args::AuthAction::Login { key } => {
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
            ctx.printer.ok(&format!(
                "{}: {}",
                crate::msg!(AuthLoginSuccess),
                Credentials::mask_key(&api_key)
            ));
        }
        crate::cli::args::AuthAction::Status => {
            let creds_path = Credentials::default_path();
            let secure = Credentials::is_permission_secure(&creds_path);
            ctx.printer.info(&format!(
                "生效 API Key: {} (来源: {})",
                Credentials::mask_key(&ctx.config.api_key),
                ctx.config.api_key_source
            ));
            ctx.printer
                .dim(&format!("凭据文件路径: {}", creds_path.display()));
            if !secure {
                ctx.printer.warn(crate::msg!(AuthFilePermWarning));
            } else {
                ctx.printer.ok("凭据权限安全 (0600)");
            }
        }
        crate::cli::args::AuthAction::Logout => {
            let purged = Credentials::purge()?;
            if purged {
                ctx.printer.ok(crate::msg!(AuthLogoutSuccess));
            } else {
                ctx.printer.dim("本地原本不存在凭据文件");
            }
        }
    }
    Ok(())
}

/// 模型矩阵渲染数据 (来自 capability_matrix.toml [models.matrix], 编译期嵌入)
#[derive(serde::Deserialize)]
struct MatrixRow {
    code: String,
    category: String,
    latency: String,
    feature: String,
}

pub fn models(ctx: &Ctx<'_>) -> QyzsResult<()> {
    const MATRIX_TOML: &str = include_str!("../../../capability_matrix.toml");
    let value: toml::Value = toml::from_str(MATRIX_TOML)
        .map_err(|e| QyzsError::Config(format!("capability_matrix 解析失败: {}", e)))?;
    let rows: Vec<MatrixRow> = value
        .get("models")
        .and_then(|m| m.get("matrix"))
        .and_then(|mx| mx.get("rows"))
        .cloned()
        .map(|v| v.try_into::<Vec<MatrixRow>>())
        .transpose()
        .map_err(|e| QyzsError::Config(format!("models.matrix 解析失败: {}", e)))?
        .unwrap_or_default();

    let styler = ctx.printer.styler();
    println!(
        "{}",
        styler.bold("庆阳智算中心全量高可靠可用模型矩阵 (SSOT 校准真值)")
    );
    println!(
        "{:<20} {:<10} {:<10} {}",
        "模型代码", "能力分类", "实测延时", "特性与场景"
    );
    println!("{}", "-".repeat(75));
    for row in &rows {
        println!(
            "{:<20} {:<10} {:<10} {}",
            row.code, row.category, row.latency, row.feature
        );
    }
    Ok(())
}

pub async fn upgrade(
    ctx: &Ctx<'_>,
    repo: String,
    package: Option<String>,
) -> QyzsResult<()> {
    if let Some(pkg) = package {
        UpdateManager::install_from_local_package(&pkg)?;
        ctx.printer
            .ok(&format!("已从本地安装包 '{}' 完成就地安装/更新！", pkg));
    } else {
        ctx.printer.info(crate::msg!(SelfUpdateChecking));
        let updated = UpdateManager::self_update(&repo).await?;
        if updated {
            ctx.printer.ok(crate::msg!(SelfUpdateSuccess));
        } else {
            ctx.printer.info(crate::msg!(SelfUpdateUpToDate));
        }
    }
    Ok(())
}

pub fn completions(shell: clap_complete::Shell) {
    crate::cli::completions::CompletionGenerator::generate(shell);
}

/// usage 子命令参数聚合
pub struct UsageArgs {
    pub days: Option<u32>,
    pub model: Option<String>,
    pub kind: Option<String>,
    pub key: Option<String>,
    pub hourly: bool,
    pub json: bool,
    pub clear: bool,
}

/// 本地网关用量统计 (台账在 client 层各接口调用点追加, 此处只读聚合)
pub fn usage(ctx: &Ctx<'_>, args: UsageArgs) -> QyzsResult<()> {
    use crate::client::usage_ledger::{self, UsageFilter};

    if args.clear {
        let removed = usage_ledger::clear();
        if args.json {
            println!("{}", serde_json::json!({ "cleared_records": removed }));
        } else {
            ctx.printer.ok(&crate::msgf!(UsageCleared, removed));
        }
        return Ok(());
    }

    let all = usage_ledger::read_records();
    let filter = UsageFilter {
        days: args.days,
        model: args.model.as_deref(),
        kind: args.kind.as_deref(),
        key: args.key.as_deref(),
    };
    let records = usage_ledger::filter_records(&all, &filter);
    let summary = usage_ledger::summarize(&records);

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&usage_json(&records, &summary, all.len(), &args))
                .unwrap_or_default()
        );
        return Ok(());
    }

    let styler = ctx.printer.styler();
    println!(
        "{}",
        styler.bold(&format!("[{}]", crate::msg!(UsageHeader)))
    );
    println!(
        "{}",
        crate::msgf!(UsageLedgerLine, usage_ledger::ledger_path().display(), all.len(), records.len())
    );
    if records.is_empty() {
        println!("  {}", crate::msg!(UsageNoData));
        return Ok(());
    }

    println!(
        "{}",
        crate::msgf!(
            UsageRange,
            summary.first_day.as_deref().unwrap_or("—"),
            summary.last_day.as_deref().unwrap_or("—")
        )
    );
    println!(
        "{}",
        crate::msgf!(
            UsageTotalLine,
            usage_ledger::format_num(summary.requests),
            usage_ledger::format_num(summary.prompt_tokens),
            usage_ledger::format_num(summary.completion_tokens),
            usage_ledger::format_num(summary.total_tokens),
            usage_ledger::format_num(summary.with_tokens)
        )
    );

    // 按调用族 (chat/ocr/asr/embed/rerank)
    println!("\n{}", styler.bold(crate::msg!(UsageByKind)));
    for (name, st) in &summary.by_kind {
        print_group_row(name, 14, st);
    }

    println!("\n{}", styler.bold(crate::msg!(UsageByModel)));
    for (name, st) in &summary.by_model {
        print_group_row(name, 22, st);
    }

    // 按凭据: 仅在匹配记录含多个凭据指纹时展示 (单 key 常态下无信息量)
    if summary.by_key.len() > 1 {
        println!("\n{}", styler.bold(crate::msg!(UsageByKey)));
        for (name, st) in &summary.by_key {
            print_group_row(name, 16, st);
        }
    }

    if args.hourly {
        println!("\n{}", styler.bold(crate::msg!(UsageByHour)));
        let hourly = usage_ledger::summarize_hourly(&records);
        for (bucket, st) in &hourly.by_hour {
            println!(
                "{}",
                crate::msgf!(
                    UsageDayRow,
                    format!("{:<16}", bucket),
                    usage_ledger::format_num(st.requests),
                    usage_ledger::format_num(st.total_tokens)
                )
            );
        }
    } else {
        println!("\n{}", styler.bold(crate::msg!(UsageByDay)));
        for (day, st) in &summary.by_day {
            println!(
                "{}",
                crate::msgf!(
                    UsageDayRow,
                    format!("{:<12}", day),
                    usage_ledger::format_num(st.requests),
                    usage_ledger::format_num(st.total_tokens)
                )
            );
        }
    }
    Ok(())
}

/// 分组行 (调用族/模型/凭据共用): 名称 + 请求数 + 三项 Token
fn print_group_row(name: &str, width: usize, st: &crate::client::usage_ledger::GroupStat) {
    use crate::client::usage_ledger as ledger;
    println!(
        "{}",
        crate::msgf!(
            UsageModelRow,
            format!("{:<width$}", name, width = width),
            ledger::format_num(st.requests),
            ledger::format_num(st.prompt_tokens),
            ledger::format_num(st.completion_tokens),
            ledger::format_num(st.total_tokens)
        )
    );
}

/// `--json` 输出结构 (过滤条件原样回显, 便于脚本核对)
fn usage_json(
    records: &[crate::client::usage_ledger::UsageRecord],
    summary: &crate::client::usage_ledger::UsageSummary,
    total_in_ledger: usize,
    args: &UsageArgs,
) -> serde_json::Value {
    use crate::client::usage_ledger as ledger;
    let group_json = |st: &ledger::GroupStat| {
        serde_json::json!({
            "requests": st.requests,
            "requests_with_tokens": st.with_tokens,
            "prompt_tokens": st.prompt_tokens,
            "completion_tokens": st.completion_tokens,
            "total_tokens": st.total_tokens,
        })
    };

    let mut root = serde_json::json!({
        "ledger_path": ledger::ledger_path().to_string_lossy(),
        "records_in_ledger": total_in_ledger,
        "records_matched": records.len(),
        "filter": {
            "days": args.days,
            "model": args.model,
            "kind": args.kind,
            "key": args.key,
        },
        "requests": summary.requests,
        "requests_with_tokens": summary.with_tokens,
        "prompt_tokens": summary.prompt_tokens,
        "completion_tokens": summary.completion_tokens,
        "total_tokens": summary.total_tokens,
        "range": {
            "first_day": summary.first_day,
            "last_day": summary.last_day,
        },
        "by_kind": summary.by_kind.iter()
            .map(|(k, st)| (k.clone(), group_json(st)))
            .collect::<serde_json::Map<String, serde_json::Value>>(),
        "by_model": summary.by_model.iter()
            .map(|(k, st)| (k.clone(), group_json(st)))
            .collect::<serde_json::Map<String, serde_json::Value>>(),
        "by_key": summary.by_key.iter()
            .map(|(k, st)| (k.clone(), group_json(st)))
            .collect::<serde_json::Map<String, serde_json::Value>>(),
        "by_day": summary.by_day.iter()
            .map(|(d, st)| (d.clone(), serde_json::json!({
                "requests": st.requests,
                "total_tokens": st.total_tokens,
            })))
            .collect::<serde_json::Map<String, serde_json::Value>>(),
    });

    // --hourly: 追加按小时桶 (仅明细行可归桶)
    if args.hourly {
        let hourly = ledger::summarize_hourly(records);
        root["by_hour"] = hourly.by_hour.iter()
            .map(|(b, st)| (b.clone(), serde_json::json!({
                "requests": st.requests,
                "total_tokens": st.total_tokens,
            })))
            .collect();
    }
    root
}

#[cfg(test)]
mod tests {
    use super::validate_config_value;

    #[test]
    fn config_set_validates_enum_values() {
        // 与 CLI 参数同一套合法值集
        assert!(validate_config_value("ui.language", "auto").is_ok());
        assert!(validate_config_value("ui.language", "zh_CN").is_ok());
        assert!(validate_config_value("ui.language", "french").is_err());
        assert!(validate_config_value("ui.color", "never").is_ok());
        assert!(validate_config_value("ui.color", "blue").is_err());
        assert!(validate_config_value("ui.format", "json").is_ok());
        assert!(validate_config_value("ui.format", "yaml").is_err());
        assert!(validate_config_value("model.language", "zh-cn").is_ok());
        assert!(validate_config_value("model.language", "jp").is_err());
        assert!(validate_config_value("network.base_url", "https://gw:9200").is_ok());
        assert!(validate_config_value("network.base_url", "ftp://gw").is_err());

        // 非枚举键不设限 (timeout 整数校验在赋值处, default_* 为模型名)
        assert!(validate_config_value("model.default_chat", "任意模型名").is_ok());
        assert!(validate_config_value("network.timeout", "abc").is_ok());
        assert!(validate_config_value("未知键", "任意值").is_ok());
    }

    #[test]
    fn model_params_entry_parsing() {
        use super::parse_model_params_entry;

        // 常规: i64 优先, 其次 f64
        let (m, p, v) = parse_model_params_entry("model.params.dsv4.max_tokens", "4096").unwrap();
        assert_eq!((m.as_str(), p.as_str()), ("dsv4", "max_tokens"));
        assert_eq!(v, toml::Value::Integer(4096));
        let (m, p, v) = parse_model_params_entry("model.params.dsv4.temperature", "0.7").unwrap();
        assert_eq!((m.as_str(), p.as_str()), ("dsv4", "temperature"));
        assert_eq!(v, toml::Value::Float(0.7));

        // 模型名含点号/连字符: 右侧最后一次分割 + 归一化为查找主形态
        let (m, p, _) = parse_model_params_entry("model.params.qwen3.6-27b.temperature", "0.4").unwrap();
        assert_eq!((m.as_str(), p.as_str()), ("qwen3_6_27b", "temperature"));

        // 非法形态与非数值
        assert!(parse_model_params_entry("model.params.dsv4", "0.7").is_err());
        assert!(parse_model_params_entry("model.params..temperature", "0.7").is_err());
        assert!(parse_model_params_entry("model.params.dsv4.temperature.", "0.7").is_err());
        assert!(parse_model_params_entry("model.params.dsv4.temperature", "auto").is_err());
    }
}
