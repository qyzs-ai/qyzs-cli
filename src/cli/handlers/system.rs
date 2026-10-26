//! 系统运维处理器 (status/config/auth/models/upgrade/completions)

use super::Ctx;
use crate::client::probe::ProbeResult;
use crate::config::{Config, Credentials};
use crate::error::{QyzsError, QyzsResult};
use crate::term::Printer;
use crate::updater::UpdateManager;
use std::io::Write;

pub async fn status(ctx: &Ctx<'_>, json: bool, no_probe: bool) -> QyzsResult<()> {
    // --no-probe: 跳过远程三网关探活 (每端点 5s 超时), 离线脚本秒回;
    // 未跳过时保持原有全链路诊断行为不变
    let probes = if no_probe {
        None
    } else {
        Some(ctx.client.run_probes().await)
    };
    let creds_path = Credentials::default_path();
    let is_secure = Credentials::is_permission_secure(&creds_path);
    let is_npm = UpdateManager::is_running_under_npm();
    // 配置文件实际可解析性: 配置层按 best-effort 回退默认值 (脏配置不阻断命令),
    // 但诊断命令必须如实报告配置是否损坏, 否则会对用户显示 [OK] 却实际用的是默认值
    let config_problem = Config::load().err().map(|e| e.to_string());
    let config = &ctx.config;
    let printer = ctx.printer;

    if json {
        let status_json = serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "install_mode": if is_npm { "npm" } else { "standalone" },
            "config_path": Config::default_path().to_string_lossy(),
            "config_valid": config_problem.is_none(),
            "config_error": config_problem,
            "credentials_path": creds_path.to_string_lossy(),
            "credentials_secure": is_secure,
            "api_key_source": config.api_key_source,
            "api_key_masked": Credentials::mask_key(&config.api_key),
            "api_key_count": config.api_keys.len(),
            "probes": probes_json(probes.as_ref())
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&status_json).unwrap_or_default()
        );
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
            if is_npm {
                "npm Global Package"
            } else {
                "Standalone Binary"
            }
        );
        match &config_problem {
            Some(_) => println!(
                "  Config File:   {} [WARN 格式错误, 已回退默认值]",
                Config::default_path().display()
            ),
            None => println!("  Config File:   {} [OK]", Config::default_path().display()),
        }
        if let Some(err) = &config_problem {
            println!("    ! {}", err);
        }
        println!(
            "  Credentials:   {} [{}]",
            creds_path.display(),
            if is_secure {
                "OK: 0600 secure"
            } else {
                "WARN: wide permissions"
            }
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
        match &probes {
            Some(p) => {
                print_probe_line(printer, &p.public_api);
                print_probe_line(printer, &p.mineru_api);
                print_probe_line(printer, &p.sam_api);
            }
            // 无探针数据时不输出全局健康结论: 跳过探针 ≠ 全链路正常
            None => printer.dim("  (--no-probe: 已跳过远程探针, 无网关健康数据)"),
        }

        if let Some(p) = &probes {
            if p.all_ok {
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
    }
    Ok(())
}

/// `status --json` 的 `probes` 段: 未跳过探针时展开三个网关, `--no-probe` 时为 null
fn probes_json(probes: Option<&crate::client::probe::GatewayProbes>) -> serde_json::Value {
    match probes {
        None => serde_json::Value::Null,
        Some(p) => serde_json::json!({
            "public_api": { "ok": p.public_api.ok, "status": p.public_api.status_code, "latency_ms": p.public_api.latency_ms },
            "mineru_api": { "ok": p.mineru_api.ok, "status": p.mineru_api.status_code, "latency_ms": p.mineru_api.latency_ms },
            "sam_api": { "ok": p.sam_api.ok, "status": p.sam_api.status_code, "latency_ms": p.sam_api.latency_ms }
        }),
    }
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
    let normalized = model.replace(['.', '-'], "_");
    Ok((normalized, param.to_string(), parsed))
}

pub fn config(ctx: &Ctx<'_>, action: crate::cli::args::ConfigAction) -> QyzsResult<()> {
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
                "model.default_asr" => println!("{}", conf.model.default_asr),
                "model.default_sam" => println!("{}", conf.model.default_sam),
                _ => {
                    // 未知键 fallback: 走 stderr 提示用户键名未在已知表里,
                    // 再将整文件 TOML 输出到 stdout 供参考 (与 read 工具的副作用约定)
                    eprintln!("# 未识别的配置键 '{}', 输出整文件 TOML 供查阅", key);
                    let toml_val = toml::Value::try_from(&conf)
                        .map_err(|e| QyzsError::Config(e.to_string()))?;
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
                "network.timeout" => {
                    conf.network.timeout = value
                        .parse()
                        .map_err(|_| QyzsError::Usage("timeout 必须为整数".to_string()))?
                }
                "ui.language" => conf.ui.language = value.clone(),
                "ui.color" => conf.ui.color = value.clone(),
                "ui.format" => conf.ui.format = value.clone(),
                "model.language" => conf.model.language = value.clone(),
                "model.default_chat" => conf.model.default_chat = value.clone(),
                "model.default_ocr" => conf.model.default_ocr = value.clone(),
                "model.default_parse" => conf.model.default_parse = value.clone(),
                "model.default_embed" => conf.model.default_embed = value.clone(),
                "model.default_rerank" => conf.model.default_rerank = value.clone(),
                "model.default_asr" => conf.model.default_asr = value.clone(),
                "model.default_sam" => conf.model.default_sam = value.clone(),
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
            ctx.printer.ok(&format!(
                "{}: {} = {}",
                crate::msg!(ConfigSaved),
                key,
                value
            ));
        }
        crate::cli::args::ConfigAction::Path => {
            println!("Config:      {}", Config::default_path().display());
            println!("Credentials: {}", Credentials::default_path().display());
        }
        crate::cli::args::ConfigAction::Reset => {
            // 破坏性操作前警告 (替代交互式确认)
            ctx.printer.warn(crate::msg!(WarnConfigResetDestructive));
            Config::reset()?;
            ctx.printer.ok(crate::msg!(ConfigReset));
        }
    }
    Ok(())
}

pub fn auth(ctx: &Ctx<'_>, action: crate::cli::args::AuthAction) -> QyzsResult<()> {
    match action {
        crate::cli::args::AuthAction::Login { key } => {
            let api_keys = if !key.is_empty() {
                key.into_iter().map(|key| key.trim().to_string()).collect()
            } else {
                print!("请输入庆阳智算中心 API Key (sk-...): ");
                std::io::stdout().flush().ok();
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                vec![input.trim().to_string()]
            };

            if api_keys.iter().any(|key| key.trim().is_empty()) {
                return Err(QyzsError::Usage("API Key 不能为空".to_string()));
            }

            let creds = Credentials {
                api_key: api_keys[0].clone(),
                api_keys: api_keys.iter().skip(1).cloned().collect(),
            };
            creds.save()?;
            ctx.printer.ok(&format!(
                "{}: {}（共 {} 个）",
                crate::msg!(AuthLoginSuccess),
                Credentials::mask_key(&api_keys[0]),
                api_keys.len()
            ));
        }
        crate::cli::args::AuthAction::Status => {
            let creds_path = Credentials::default_path();
            let secure = Credentials::is_permission_secure(&creds_path);
            ctx.printer.info(&format!(
                "生效 API Key: {} (来源: {}，共 {} 个)",
                Credentials::mask_key(&ctx.config.api_key),
                ctx.config.api_key_source,
                ctx.config.api_keys.len()
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
            // 破坏性操作前警告 (替代交互式确认, 适配无 TTY 场景)
            ctx.printer.warn(crate::msg!(WarnAuthLogoutDestructive));
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
    /// 在线可用标记: `false` 表示实测禁止调度, 仅 `--all-models` 时展示
    online: bool,
}

pub fn models(ctx: &Ctx<'_>, all_models: bool) -> QyzsResult<()> {
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
    let title = if all_models {
        "庆阳智算中心全量模型矩阵 (含实测禁止调度项, SSOT 校准真值)"
    } else {
        "庆阳智算中心在线可用模型矩阵 (SSOT 校准真值)"
    };
    println!("{}", styler.bold(title));
    println!(
        "{:<20} {:<10} {:<10} 特性与场景",
        "模型代码", "能力分类", "实测延时"
    );
    println!("{}", "-".repeat(75));
    for row in select_rows(&rows, all_models) {
        println!(
            "{:<20} {:<10} {:<10} {}",
            row.code, row.category, row.latency, row.feature
        );
    }
    Ok(())
}

/// 按 `online` 标记筛选矩阵行: `--all-models` 返回全部, 默认仅返回在线项
fn select_rows(rows: &[MatrixRow], all_models: bool) -> Vec<&MatrixRow> {
    rows.iter().filter(|r| all_models || r.online).collect()
}

pub async fn upgrade(
    ctx: &Ctx<'_>,
    repo: String,
    package: Option<String>,
    check: bool,
) -> QyzsResult<()> {
    // --check 与 --package 互斥语义: --package 是「就地装」的语义,
    // --check 只读查询版本, 不应被本地包路径触发
    if check && package.is_some() {
        return Err(QyzsError::Usage(
            "--check 与 --package 互斥: --check 仅查询远程版本, 不接受本地安装包路径".to_string(),
        ));
    }

    if let Some(pkg) = package {
        UpdateManager::install_from_local_package(&pkg)?;
        ctx.printer
            .ok(&format!("已从本地安装包 '{}' 完成就地安装/更新！", pkg));
    } else {
        ctx.printer.info(crate::msg!(SelfUpdateChecking));
        if check {
            // 仅检查: 复用 self_update 的远程查询路径, 但在替换前终止
            // 这里直接调底层查询, 不进入下载/替换分支
            return upgrade_check_only(ctx, &repo).await;
        }
        let updated = UpdateManager::self_update(&repo).await?;
        if updated {
            ctx.printer.ok(crate::msg!(SelfUpdateSuccess));
        } else {
            ctx.printer.info(crate::msg!(SelfUpdateUpToDate));
        }
    }
    Ok(())
}

/// `--check` 仅查询: 取 GitHub 最新 release, 与当前版本对比并打印, 不下载不替换。
/// 复用 `installer.rs::GitHubClient` 但避免走 `self_update` 的下载与原子替换分支
async fn upgrade_check_only(ctx: &Ctx<'_>, repo: &str) -> QyzsResult<()> {
    use crate::updater::github::GitHubClient;

    if UpdateManager::is_running_under_npm() {
        return Err(QyzsError::Usage(
            crate::msg!(SelfUpdateNpmWarning).to_string(),
        ));
    }

    let release = GitHubClient::get_latest_release(repo).await?;
    let remote_tag = release.tag_name.trim_start_matches('v').to_string();
    let current = env!("CARGO_PKG_VERSION").to_string();

    if remote_tag == current {
        ctx.printer.info(crate::msg!(SelfUpdateUpToDate));
    } else {
        ctx.printer.ok(&format!(
            "可升级: 当前 v{} → 远程 v{} (运行 `qyzs upgrade` 执行就地更新)",
            current, remote_tag
        ));
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
        // 破坏性操作前警告 (替代交互式确认; --json 模式下静默走数据通道, 警告仍 stderr 输出)
        ctx.printer.warn(crate::msg!(WarnUsageClearDestructive));
        let removed = usage_ledger::clear();
        if args.json {
            println!("{}", serde_json::json!({ "cleared_records": removed }));
        } else {
            ctx.printer.ok(&crate::msgf!(UsageCleared, removed));
        }
        return Ok(());
    }

    // 读取前时机性压实 (best-effort): 保留期内明细 + 归档行
    usage_ledger::compact_if_needed();

    let all = usage_ledger::read_records();
    let filter = UsageFilter {
        days: args.days,
        model: args.model.as_deref(),
        kind: args.kind.as_deref(),
        key: args.key.as_deref(),
    };
    let records = usage_ledger::filter_records(&all, &filter);
    let summary = usage_ledger::summarize(&records);
    // 归档行贡献 (按小时桶无法容纳, 单独口径展示; 明细超 30 天后也并入此处)
    let archived: (u64, u64) = records
        .iter()
        .filter(|r| r.is_rollup())
        .fold((0, 0), |(n, t), r| {
            (n + r.request_count(), t + r.total_tokens.unwrap_or(0))
        });

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&usage_json(
                &records,
                &summary,
                all.len(),
                &args,
                archived
            ))
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
        crate::msgf!(
            UsageLedgerLine,
            usage_ledger::ledger_path().display(),
            all.len(),
            records.len()
        )
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
        if hourly.archived_requests > 0 {
            println!(
                "{}",
                crate::msgf!(
                    UsageArchivedNote,
                    usage_ledger::format_num(hourly.archived_requests),
                    usage_ledger::format_num(hourly.archived_tokens),
                    usage_ledger::DETAIL_RETENTION_DAYS
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
    archived: (u64, u64),
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
            // 回显实际生效的过滤值 (已 trim, 与 filter_records 同口径)
            "model": args.model.as_deref().map(str::trim),
            "kind": args.kind.as_deref().map(str::trim),
            "key": args.key.as_deref().map(str::trim),
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
        // 归档行 (日粒度历史) 的贡献; 按小时桶无法容纳, 单列口径
        "archived": {
            "requests": archived.0,
            "total_tokens": archived.1,
        },
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
        root["by_hour"] = hourly
            .by_hour
            .iter()
            .map(|(b, st)| {
                (
                    b.clone(),
                    serde_json::json!({
                        "requests": st.requests,
                        "total_tokens": st.total_tokens,
                    }),
                )
            })
            .collect();
    }
    root
}

#[cfg(test)]
mod tests {
    use super::probes_json;
    use super::validate_config_value;
    use crate::client::probe::{GatewayProbes, ProbeResult};

    fn probe(name: &str, ok: bool, status: u16, latency: u64) -> ProbeResult {
        ProbeResult {
            name: name.to_string(),
            endpoint: "ep".to_string(),
            ok,
            status_code: status,
            latency_ms: latency,
            error_msg: None,
        }
    }

    #[test]
    fn status_json_probes_contract() {
        // --no-probe: probes 段为 null (脚本据此判断"未探测"而非"全部失败")
        assert!(probes_json(None).is_null());

        // 正常探测: 三网关平铺
        let p = GatewayProbes {
            base_url: "http://gw".to_string(),
            public_api: probe("public", true, 400, 12),
            mineru_api: probe("mineru", false, 500, 30),
            sam_api: probe("sam", true, 405, 8),
            all_ok: false,
        };
        let v = probes_json(Some(&p));
        assert_eq!(v["public_api"]["ok"], serde_json::json!(true));
        assert_eq!(v["public_api"]["status"], serde_json::json!(400));
        assert_eq!(v["public_api"]["latency_ms"], serde_json::json!(12));
        assert_eq!(v["mineru_api"]["ok"], serde_json::json!(false));
        assert_eq!(v["mineru_api"]["status"], serde_json::json!(500));
        assert_eq!(v["sam_api"]["ok"], serde_json::json!(true));
    }

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
        let (m, p, _) =
            parse_model_params_entry("model.params.qwen3.6-27b.temperature", "0.4").unwrap();
        assert_eq!((m.as_str(), p.as_str()), ("qwen3_6_27b", "temperature"));

        // 非法形态与非数值
        assert!(parse_model_params_entry("model.params.dsv4", "0.7").is_err());
        assert!(parse_model_params_entry("model.params..temperature", "0.7").is_err());
        assert!(parse_model_params_entry("model.params.dsv4.temperature.", "0.7").is_err());
        assert!(parse_model_params_entry("model.params.dsv4.temperature", "auto").is_err());
    }

    #[test]
    fn models_matrix_online_filtering() {
        use super::{select_rows, MatrixRow};

        // 解析编译期嵌入的矩阵 (行缺 `online` 字段会在这里直接失败)
        const MATRIX_TOML: &str = include_str!("../../../capability_matrix.toml");
        let value: toml::Value = toml::from_str(MATRIX_TOML).expect("capability_matrix 可解析");
        let rows: Vec<MatrixRow> = value
            .get("models")
            .and_then(|m| m.get("matrix"))
            .and_then(|mx| mx.get("rows"))
            .cloned()
            .map(|v| v.try_into::<Vec<MatrixRow>>())
            .transpose()
            .expect("models.matrix 可解析")
            .unwrap_or_default();

        // 默认 (--online-only): 只留 online = true
        let online = select_rows(&rows, false);
        assert!(online.iter().all(|r| r.online));
        assert!(!online.is_empty());

        // --all-models: 返回全部, 且应多于在线子集 (存在被过滤掉的已下线项)
        let all = select_rows(&rows, true);
        assert_eq!(all.len(), rows.len());
        assert!(
            all.len() > online.len(),
            "--all-models 应比 --online-only 多出已下线项"
        );

        // 实测禁止调度的模型必须以 online = false 存在
        // (2026-10-26 复核后: qwen36 / paddleocrvl 在线，qwen3 新增且不可调度;
        //  状态翻转须与 capability_matrix.toml 同步)
        for code in [
            "paddleocrvl",
            "deepseek_v3_2",
            "qwen2.5-72b",
            "text-embedding-v4",
            "mymodel-paddleocr-udhyfg",
            "qwen3",
        ] {
            let row = all
                .iter()
                .find(|r| r.code == code)
                .unwrap_or_else(|| panic!("矩阵缺少 {code} 行"));
            assert!(!row.online, "{code} 应标记为 online = false");
        }
    }
}
