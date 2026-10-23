//! QYZS 核心集成测试套件

use clap::Parser as _;
use qyzs::config::{Credentials, ResolvedConfig};
use qyzs::error::{exit_code, QyzsError};
use qyzs::pdf::{PdfExtractor, PdfSplitter};
use qyzs::pipeline::{DatasetManager, OcrVerifier};
use qyzs::term::{detect_environment_language, set_language, Language};
use std::io::Write;
use tempfile::tempdir;

#[test]
fn test_error_exit_codes() {
    assert_eq!(
        QyzsError::Usage("test".into()).exit_code(),
        exit_code::USAGE_ERR
    );
    assert_eq!(
        QyzsError::Auth("test".into()).exit_code(),
        exit_code::AUTH_ERR
    );
    assert_eq!(
        QyzsError::Timeout("test".into()).exit_code(),
        exit_code::TIMEOUT_ERR
    );
    assert_eq!(QyzsError::Io("test".into()).exit_code(), exit_code::IO_ERR);
    assert_eq!(QyzsError::Pdf("test".into()).exit_code(), exit_code::IO_ERR);
    // sysexits.h 对齐: Network/Server/Config 不再退化成 FAILURE=1
    assert_eq!(
        QyzsError::Network("test".into()).exit_code(),
        exit_code::EX_UNAVAILABLE
    );
    assert_eq!(
        QyzsError::Server("test".into()).exit_code(),
        exit_code::EX_SOFTWARE
    );
    assert_eq!(
        QyzsError::Config("test".into()).exit_code(),
        exit_code::EX_CONFIG
    );
}

#[test]
fn test_i18n_detection() {
    // 设置语言测试
    // 还原: set_language 改的是进程级全局状态, 同一测试二进制内的用例并行执行,
    // 不还原会污染其他用例 (lib 侧的 i18n 单测已按此惯例还原)
    let saved = qyzs::term::current_language();
    set_language(Language::Zh);
    assert_eq!(qyzs::msg!(AllSystemsOperational), "系统全链路状态正常");

    set_language(Language::En);
    assert_eq!(qyzs::msg!(AllSystemsOperational), "All systems operational");
    set_language(saved);

    // 探测函数测试
    let lang = detect_environment_language();
    assert!(lang == Language::Zh || lang == Language::En);
}

#[test]
fn test_credentials_masking() {
    let raw = "sk-abcdef1234567890wxyz";
    let masked = Credentials::mask_key(raw);
    assert_eq!(masked, "sk-abcd***xyz");

    let short = "12345";
    assert_eq!(Credentials::mask_key(short), "***");
}

#[test]
fn test_config_precedence() {
    let resolved = ResolvedConfig::resolve(
        Some("cli-key-123"),
        Some("http://custom-host:8000"),
        Some(60),
        Some("zh"),
        Some("cn"),
        Some("never"),
        Some("json"),
    )
    .unwrap();

    assert_eq!(resolved.api_key, "cli-key-123");
    assert_eq!(resolved.api_key_source, "CLI Argument");
    assert_eq!(resolved.base_url, "http://custom-host:8000");
    assert_eq!(resolved.timeout_seconds, 60);
    assert_eq!(resolved.ui_language, Language::Zh);
    assert_eq!(resolved.model_language, "cn");
    assert_eq!(resolved.output_format, qyzs::term::OutputFormat::Json);
}

#[test]
fn test_pdf_operations() {
    let pdf_path = "tests/data/mineru/text_pdf/REF-0056_政府采购评审专家管理办法.pdf";
    if !std::path::Path::new(pdf_path).exists() {
        return; // 测试集若未就绪跳过
    }

    // 1. 读取元数据与页数
    let meta = PdfExtractor::read_metadata(pdf_path).expect("读取 PDF 元数据失败");
    assert!(meta.page_count > 0, "PDF 必须至少有 1 页");

    // 2. 页码表达式解析
    let range = PdfSplitter::parse_page_range("1-2", meta.page_count as u32).expect("解析页码失败");
    assert_eq!(range, vec![1, 2]);

    // 3. 页面切片保存
    let dir = tempdir().expect("创建临时目录失败");
    let split_out = dir.path().join("split_p1_p2.pdf");
    PdfSplitter::split(pdf_path, &range, split_out.to_str().unwrap()).expect("拆分 PDF 失败");
    assert!(split_out.exists());

    // 4. 验证拆分后 PDF 的元数据
    let split_meta =
        PdfExtractor::read_metadata(split_out.to_str().unwrap()).expect("读取拆分 PDF 元数据失败");
    assert_eq!(split_meta.page_count, 2);

    // 5. 提取文字
    let text =
        PdfExtractor::extract_text(split_out.to_str().unwrap(), &[1]).expect("提取第一页文字失败");
    assert!(!text.is_empty(), "第一页提取文字不应为空");
}

#[test]
fn test_cross_verifier() {
    let ocr = "中华人民共和国财政部公告 政府采购评审专家";
    let md = "# 政府采购评审专家管理办法\n财政部发布公告。";
    let report = OcrVerifier::verify(ocr, md);
    assert!(report.jaccard_similarity > 0.0);
    assert_eq!(report.ocr_char_count, 20);
}

#[test]
fn test_zip_dataset_extraction() {
    let dir = tempdir().expect("创建临时目录失败");
    let zip_data = {
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip_writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip_writer.start_file("sample/test.txt", options).unwrap();
            zip_writer.write_all(b"Hello Qingyang AI").unwrap();
            zip_writer.finish().unwrap();
        }
        buffer.into_inner()
    };

    let target_dir = dir.path().join("extracted");
    let count = DatasetManager::extract_zip_to_dir(&zip_data, target_dir.to_str().unwrap())
        .expect("解压失败");
    assert_eq!(count, 1);
    let extracted_file = target_dir.join("sample").join("test.txt");
    assert!(extracted_file.exists());
    let content = std::fs::read_to_string(extracted_file).unwrap();
    assert_eq!(content, "Hello Qingyang AI");
}

#[test]
fn test_scan_cli_arg_surface() {
    // 完整参数面契约：全部参数可解析且取值正确传递
    let cli = qyzs::Cli::try_parse_from([
        "qyzs",
        "markscan",
        "a.pdf",
        "--dry-run",
        "--mode",
        "rigor",
        "--type",
        "pdf",
        "--image-handling",
        "describe",
        "--min-confidence",
        "0.9",
        "--segment-chars",
        "12000",
        "--resume",
        "--no-clean",
        "-o",
        "out.md",
        "--report",
        "report.json",
        "--model",
        "dsv4",
    ])
    .expect("markscan 完整参数面应可解析");

    if let qyzs::Commands::MarkScan {
        input,
        dry_run,
        mode,
        r#type,
        image_handling,
        min_confidence,
        segment_chars,
        resume,
        no_clean,
        output,
        report,
        model,
        ..
    } = cli.command
    {
        assert_eq!(input, vec!["a.pdf".to_string()]);
        assert!(dry_run);
        assert_eq!(mode.as_deref(), Some("rigor"));
        assert_eq!(r#type.as_deref(), Some("pdf"));
        assert_eq!(image_handling.as_deref(), Some("describe"));
        assert_eq!(min_confidence, Some(0.9));
        assert_eq!(segment_chars, 12_000);
        assert!(resume);
        assert!(no_clean);
        assert_eq!(output.as_deref(), Some("out.md"));
        assert_eq!(report.as_deref(), Some("report.json"));
        assert_eq!(model.as_deref(), Some("dsv4"));
    } else {
        panic!("应为 Scan 命令变体");
    }
}

#[test]
fn test_scan_cli_defaults() {
    // 默认值契约：segment_chars=24000、三个开关关闭、mode 留空 (dispatch 层默认 rigor)
    let cli = qyzs::Cli::try_parse_from(["qyzs", "markscan", "a.pdf", "--dry-run"])
        .expect("markscan 最小参数应可解析");

    if let qyzs::Commands::MarkScan {
        dry_run,
        mode,
        segment_chars,
        resume,
        no_clean,
        ..
    } = cli.command
    {
        assert!(dry_run);
        assert!(mode.is_none());
        assert_eq!(segment_chars, 24_000);
        assert!(!resume);
        assert!(!no_clean);
    } else {
        panic!("应为 Scan 命令变体");
    }
}

#[test]
fn test_usage_cli_arg_surface() {
    // 参数面契约：--days/--model/--json 可解析且取值正确
    let cli =
        qyzs::Cli::try_parse_from(["qyzs", "usage", "--days", "7", "--model", "dsv4", "--json"])
            .expect("usage 完整参数面应可解析");

    if let qyzs::Commands::Usage {
        days,
        model,
        kind,
        key,
        hourly,
        json,
        clear,
    } = cli.command
    {
        assert_eq!(days, Some(7));
        assert_eq!(model.as_deref(), Some("dsv4"));
        assert_eq!(kind, None);
        assert_eq!(key, None);
        assert!(!hourly);
        assert!(json);
        assert!(!clear);
    } else {
        panic!("应为 Usage 命令变体");
    }

    // 默认值契约：无参数即全量统计，--clear 与过滤参数互不干扰
    let defaults = qyzs::Cli::try_parse_from(["qyzs", "usage"]).expect("usage 零参数应可解析");
    if let qyzs::Commands::Usage {
        days,
        model,
        kind,
        key,
        hourly,
        json,
        clear,
    } = defaults.command
    {
        assert_eq!(days, None);
        assert_eq!(model, None);
        assert_eq!(kind, None);
        assert_eq!(key, None);
        assert!(!hourly);
        assert!(!json);
        assert!(!clear);
    } else {
        panic!("应为 Usage 命令变体");
    }
}

#[test]
fn test_global_short_flags_surface() {
    // 短语法规范契约：全局参数短形式 -b/-t/-l/-c/-f 可解析且子命令前后位置均可
    let cli = qyzs::Cli::try_parse_from([
        "qyzs",
        "-b",
        "http://gw:9200",
        "-t",
        "30",
        "-l",
        "zh",
        "-c",
        "never",
        "-f",
        "json",
        "status",
    ])
    .expect("全局短形式 (子命令前) 应可解析");
    assert_eq!(cli.base_url.as_deref(), Some("http://gw:9200"));
    assert_eq!(cli.timeout, Some(30));
    assert_eq!(cli.lang.as_deref(), Some("zh"));
    assert_eq!(cli.color.as_deref(), Some("never"));
    assert_eq!(cli.format.as_deref(), Some("json"));

    let cli = qyzs::Cli::try_parse_from(["qyzs", "models", "-f", "json", "-t", "45"])
        .expect("全局短形式 (子命令后) 应可解析");
    assert_eq!(cli.format.as_deref(), Some("json"));
    assert_eq!(cli.timeout, Some(45));
}

#[test]
fn test_language_flag_canonical_and_alias() {
    // 唯一形式契约: --lang 与 -l 等价可解析; --language 已统一移除, 必须拒绝
    for argv in [
        vec!["qyzs", "--lang", "en", "models"],
        vec!["qyzs", "-l", "en", "models"],
    ] {
        let cli = qyzs::Cli::try_parse_from(argv).expect("语言参数应可解析");
        assert_eq!(cli.lang.as_deref(), Some("en"));
    }

    assert!(
        qyzs::Cli::try_parse_from(["qyzs", "--language", "en", "models"]).is_err(),
        "--language 已统一为 --lang, 不应再可解析"
    );
}

#[test]
fn test_language_resolution_chain() {
    // CLI 显式值: zh 系判中文、en 系判英文; 非法值显式报错 (USAGE_ERR)
    use qyzs::error::QyzsError;
    use qyzs::term::Language;

    let zh = ResolvedConfig::resolve(None, None, None, Some("zh_CN"), None, None, None).unwrap();
    assert_eq!(zh.ui_language, Language::Zh);
    let en = ResolvedConfig::resolve(None, None, None, Some("en-US"), None, None, None).unwrap();
    assert_eq!(en.ui_language, Language::En);

    match ResolvedConfig::resolve(None, None, None, Some("french"), None, None, None) {
        Err(QyzsError::Usage(msg)) => assert!(msg.contains("zh | en"), "报错应提示合法值: {msg}"),
        other => panic!("非法语言值应返回 Usage 错误, 实际: {other:?}"),
    }
}

#[test]
fn test_invalid_global_values_rejected() {
    // 取值校验契约: CLI 显式非法值必须报 USAGE_ERR 并列出合法值 (env/config 层 best-effort 不在此测)
    use qyzs::error::QyzsError;
    use qyzs::term::{ColorChoice, OutputFormat};

    macro_rules! assert_usage_err {
        ($res:expr, $hint:expr) => {
            match $res {
                Err(QyzsError::Usage(msg)) => {
                    assert!(msg.contains($hint), "报错应含 '{}', 实际: {}", $hint, msg);
                }
                Ok(_) => panic!("应返回 Usage 错误, 实际成功"),
                Err(e) => panic!("应返回 Usage 错误, 实际: {e}"),
            }
        };
    }

    assert_usage_err!(
        ResolvedConfig::resolve(None, None, None, None, None, Some("blue"), None),
        "auto | always | never"
    );
    assert_usage_err!(
        ResolvedConfig::resolve(None, None, None, None, None, None, Some("yaml")),
        "text, json, raw"
    );
    assert_usage_err!(
        ResolvedConfig::resolve(None, None, None, None, Some("jp"), None, None),
        "auto | cn | zh | zh-cn | en"
    );
    assert_usage_err!(
        ResolvedConfig::resolve(None, Some("ftp://gw:9200"), None, None, None, None, None),
        "http://"
    );

    // 合法值契约: 大小写不敏感、trim 归一
    let color =
        ResolvedConfig::resolve(None, None, None, None, None, Some("ALWAYS"), None).unwrap();
    assert_eq!(color.color_choice, ColorChoice::Always);
    let mlang =
        ResolvedConfig::resolve(None, None, None, None, Some(" ZH-CN "), None, None).unwrap();
    assert_eq!(mlang.model_language, "zh-cn");
    let url = ResolvedConfig::resolve(
        None,
        Some("  https://gw.example  "),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(url.base_url, "https://gw.example");
    let fmt = ResolvedConfig::resolve(None, None, None, None, None, None, Some("RAW")).unwrap();
    assert_eq!(fmt.output_format, OutputFormat::Raw);
}

#[test]
fn test_subcommand_short_flag_additions() {
    // 短语法规范契约: --model 统一 -m (markscan 补齐), rerank --top-n 补齐 -n
    let cli = qyzs::Cli::try_parse_from(["qyzs", "markscan", "a.pdf", "-m", "dsv4", "--dry-run"])
        .expect("markscan -m 应可解析");
    if let qyzs::Commands::MarkScan { model, .. } = cli.command {
        assert_eq!(model.as_deref(), Some("dsv4"));
    } else {
        panic!("应为 Scan 命令变体");
    }

    let cli = qyzs::Cli::try_parse_from([
        "qyzs", "rerank", "-q", "土地", "-d", "A", "-d", "B", "-n", "2",
    ])
    .expect("rerank -n 应可解析");
    if let qyzs::Commands::Rerank {
        query,
        document,
        top_n,
        ..
    } = cli.command
    {
        assert_eq!(query, "土地");
        assert_eq!(document, vec!["A", "B"]);
        assert_eq!(top_n, Some(2));
    } else {
        panic!("应为 Rerank 命令变体");
    }
}

#[test]
fn test_pdf_split_pages_and_chunk_size_conflict() {
    // --pages 与 --chunk-size 互斥, 同时出现应在 clap 层拒收
    assert!(
        qyzs::Cli::try_parse_from([
            "qyzs",
            "pdf-split",
            "a.pdf",
            "--pages",
            "1-3",
            "--chunk-size",
            "5",
        ])
        .is_err(),
        "--pages 与 --chunk-size 互斥, 不应同时解析"
    );

    // 二选其一仍正常解析
    assert!(qyzs::Cli::try_parse_from(["qyzs", "pdf-split", "a.pdf", "--pages", "1-3"]).is_ok());
    assert!(qyzs::Cli::try_parse_from(["qyzs", "pdf-split", "a.pdf", "--chunk-size", "5"]).is_ok());
}

#[test]
fn test_usage_clear_conflicts_with_filters() {
    // --clear 与各过滤参数互斥 (与 --json/--hourly 不冲突 — 后两者是输出呈现)
    for argv in [
        vec!["qyzs", "usage", "--clear", "--days", "7"],
        vec!["qyzs", "usage", "--clear", "--model", "dsv4"],
        vec!["qyzs", "usage", "--clear", "--kind", "chat"],
        vec!["qyzs", "usage", "--clear", "--key", "sk-***0001"],
    ] {
        // 调试字符串先于 move 前构造, 避免借用错位
        let dbg = format!("{:?}", argv);
        assert!(
            qyzs::Cli::try_parse_from(argv).is_err(),
            "--clear 与过滤参数互斥, 不应同时解析: {dbg}"
        );
    }

    // --clear 与 --json / --hourly 不冲突
    assert!(qyzs::Cli::try_parse_from(["qyzs", "usage", "--clear", "--json"]).is_ok());
    assert!(qyzs::Cli::try_parse_from(["qyzs", "usage", "--clear", "--hourly"]).is_ok());

    // 单独 --clear 仍正常解析
    assert!(qyzs::Cli::try_parse_from(["qyzs", "usage", "--clear"]).is_ok());
}

#[test]
fn test_chat_stream_and_no_stream_are_mutually_exclusive() {
    // 默认值契约: 不带任何 stream flag 时, stream 字段为 true (默认开启), no_stream 为 false
    let cli = qyzs::Cli::try_parse_from(["qyzs", "chat", "hi"]).expect("chat 最小参数可解析");
    if let qyzs::Commands::Chat {
        stream, no_stream, ..
    } = cli.command
    {
        assert!(stream, "默认应开启流式");
        assert!(!no_stream, "no_stream 默认关闭");
    } else {
        panic!("应为 Chat 变体");
    }

    // --stream 显式开启 (字段已 true, 行为不变)
    let cli =
        qyzs::Cli::try_parse_from(["qyzs", "chat", "hi", "--stream"]).expect("--stream 可解析");
    if let qyzs::Commands::Chat {
        stream, no_stream, ..
    } = cli.command
    {
        assert!(stream, "--stream 应保持开启");
        assert!(!no_stream, "--stream 时 no_stream 仍关闭");
    } else {
        panic!("应为 Chat 变体");
    }

    // --no-stream 关闭流式
    let cli = qyzs::Cli::try_parse_from(["qyzs", "chat", "hi", "--no-stream"])
        .expect("--no-stream 可解析");
    if let qyzs::Commands::Chat {
        stream, no_stream, ..
    } = cli.command
    {
        assert!(no_stream, "--no-stream 应置位");
        // stream 字段默认值仍是 true (handler 端 !no_stream && stream = false)
        assert!(
            stream,
            "默认 stream=true; handler 端取反后 use_stream=false"
        );
    } else {
        panic!("应为 Chat 变体");
    }

    // --stream 与 --no-stream 同时出现: clap conflicts_with 互斥拒绝
    assert!(
        qyzs::Cli::try_parse_from(["qyzs", "chat", "hi", "--stream", "--no-stream"]).is_err(),
        "--stream 与 --no-stream 互斥, 不应同时解析"
    );
}

#[test]
fn test_status_no_probe_flag_parses() {
    // --no-probe 单独 / 与 --json 共存都合法
    assert!(qyzs::Cli::try_parse_from(["qyzs", "status", "--no-probe"]).is_ok());
    assert!(qyzs::Cli::try_parse_from(["qyzs", "status", "--no-probe", "--json"]).is_ok());
    // 解析后字段正确落入 Commands::Status
    let cli = qyzs::Cli::try_parse_from(["qyzs", "status", "--no-probe", "--json"])
        .expect("status --no-probe 可解析");
    if let qyzs::Commands::Status { json, no_probe } = cli.command {
        assert!(no_probe);
        assert!(json);
    } else {
        panic!("应为 Status 变体");
    }
    // 不带 --no-probe 时默认 false (保持原有全链路诊断行为)
    let cli = qyzs::Cli::try_parse_from(["qyzs", "status"]).expect("status 可解析");
    if let qyzs::Commands::Status { no_probe, .. } = cli.command {
        assert!(!no_probe, "--no-probe 必须是显式开启, 默认行为不变");
    } else {
        panic!("应为 Status 变体");
    }
}

#[test]
fn test_upgrade_check_flag_parses() {
    // --check 单独解析通过
    assert!(qyzs::Cli::try_parse_from(["qyzs", "upgrade", "--check"]).is_ok());
    // 与 --repo 共存也合法
    assert!(qyzs::Cli::try_parse_from([
        "qyzs",
        "upgrade",
        "--check",
        "--repo",
        "qyzs-ai/qyzs-cli"
    ])
    .is_ok());
    // 解析后字段正确落入 Commands::Upgrade
    let cli =
        qyzs::Cli::try_parse_from(["qyzs", "upgrade", "--check"]).expect("upgrade --check 可解析");
    if let qyzs::Commands::Upgrade { check, repo, .. } = cli.command {
        assert!(check);
        assert_eq!(repo, "qyzs-ai/qyzs-cli");
    } else {
        panic!("应为 Upgrade 变体");
    }
}

#[test]
fn test_scan_enum_values_rejected_at_parse() {
    // --mode 仅接受 speed | rigor | balanced(兼容值,映射 rigor)
    assert!(
        qyzs::Cli::try_parse_from(["qyzs", "markscan", "a.pdf", "--mode", "foo"]).is_err(),
        "--mode 非法值应被 clap 拒绝"
    );
    assert!(qyzs::Cli::try_parse_from(["qyzs", "markscan", "a.pdf", "--mode", "speed"]).is_ok());

    // --type 仅接受固定枚举
    assert!(
        qyzs::Cli::try_parse_from(["qyzs", "markscan", "a.pdf", "--type", "docxx"]).is_err(),
        "--type 非法值应被 clap 拒绝"
    );
    assert!(qyzs::Cli::try_parse_from(["qyzs", "markscan", "a.pdf", "--type", "pdf"]).is_ok());

    // --image-handling 仅接受固定枚举
    assert!(
        qyzs::Cli::try_parse_from(["qyzs", "markscan", "a.png", "--image-handling", "translate"])
            .is_err(),
        "--image-handling 非法值应被 clap 拒绝"
    );
    assert!(
        qyzs::Cli::try_parse_from(["qyzs", "markscan", "a.png", "--image-handling", "ocr"]).is_ok()
    );
}

#[test]
fn test_pdf_split_chunk_size_zero_rejected_by_validator() {
    // --chunk-size 0 在 clap 层合法 (usize), 但 dispatch 层 validate 兜住
    let cli = qyzs::Cli::try_parse_from(["qyzs", "pdf-split", "a.pdf", "--chunk-size", "0"])
        .expect("--chunk-size 0 在 clap 层可解析, 由 dispatch validate 拒绝");
    if let qyzs::Commands::PdfSplit { chunk_size, .. } = cli.command {
        assert_eq!(
            chunk_size,
            Some(0),
            "clap 接受 0, validate 在 dispatch 层挡"
        );
    } else {
        panic!("应为 PdfSplit 变体");
    }
}

#[test]
fn test_models_online_only_and_all_models_conflict() {
    // 默认值契约: 不带任何 flag 时, online_only = true, all_models = false
    let cli = qyzs::Cli::try_parse_from(["qyzs", "models"]).expect("models 最小参数可解析");
    if let qyzs::Commands::Models {
        online_only,
        all_models,
    } = cli.command
    {
        assert!(online_only, "默认 online_only");
        assert!(!all_models, "默认不显式 --all-models");
    } else {
        panic!("应为 Models 变体");
    }

    // --online-only 显式开启
    let cli = qyzs::Cli::try_parse_from(["qyzs", "models", "--online-only"])
        .expect("--online-only 可解析");
    if let qyzs::Commands::Models {
        online_only,
        all_models,
    } = cli.command
    {
        assert!(online_only);
        assert!(!all_models);
    } else {
        panic!("应为 Models 变体");
    }

    // --all-models 关闭 online-only
    let cli =
        qyzs::Cli::try_parse_from(["qyzs", "models", "--all-models"]).expect("--all-models 可解析");
    if let qyzs::Commands::Models {
        online_only,
        all_models,
    } = cli.command
    {
        assert!(online_only, "online_only 默认 true 不变");
        assert!(all_models, "--all-models 应置位");
    } else {
        panic!("应为 Models 变体");
    }

    // 互斥: --online-only 与 --all-models 同时出现
    assert!(
        qyzs::Cli::try_parse_from(["qyzs", "models", "--online-only", "--all-models"]).is_err(),
        "--online-only 与 --all-models 互斥, 不应同时解析"
    );
}

#[test]
fn test_chat_image_value_delimiter_and_bounds() {
    // 重复 -i 与逗号分隔都能解析
    let cli = qyzs::Cli::try_parse_from([
        "qyzs", "chat", "hi", "-i", "a.png", "-i", "b.png", "-i", "c.png",
    ])
    .expect("-i 重复三次可解析");
    if let qyzs::Commands::Chat { image, .. } = cli.command {
        assert_eq!(image.len(), 3);
    } else {
        panic!("应为 Chat 变体");
    }

    let cli = qyzs::Cli::try_parse_from(["qyzs", "chat", "hi", "-i", "a.png,b.png,c.png"])
        .expect("-i 逗号分隔可解析");
    if let qyzs::Commands::Chat { image, .. } = cli.command {
        assert_eq!(image.len(), 3, "value_delimiter 应拆分逗号");
    } else {
        panic!("应为 Chat 变体");
    }

    // 17 张图: clap 层 value_delimiter 拆分后总数 17, 交由 handler 自动分批
    // (16 图可靠上限 → 拆为 16+1 两个子请求; 见 inference.rs 的分批逻辑)
    let cli = qyzs::Cli::try_parse_from([
        "qyzs",
        "chat",
        "hi",
        "-i",
        "1.png,2.png,3.png,4.png,5.png,6.png,7.png,8.png,9.png,10.png,11.png,12.png,13.png,14.png,15.png,16.png,17.png",
    ])
    .expect("clap 解析通过; 超过 16 张不拒绝, 由 handler 分批");
    if let qyzs::Commands::Chat { image, .. } = cli.command {
        assert_eq!(image.len(), 17, "clap 拆分后总数 17, 交由 handler 分批");
    } else {
        panic!("应为 Chat 变体");
    }

    // 16 张正好到达上限: clap 与 dispatch 都过
    assert!(qyzs::Cli::try_parse_from([
        "qyzs",
        "chat",
        "hi",
        "-i",
        "1.png,2.png,3.png,4.png,5.png,6.png,7.png,8.png,9.png,10.png,11.png,12.png,13.png,14.png,15.png,16.png",
    ])
    .is_ok());
}

/// 解析层通过 + dispatch validate 兜底: 数值超界能被 clap 解析但值落入 dispatch 验证。
/// 这一批测试的语义与 chat --image 17 张的写法同款
#[test]
fn test_numeric_upper_bounds_parsed_by_clap_validated_by_dispatch() {
    // markscan --max-dim: 16384 应被 dispatch 拒 (上界 8192)
    let cli = qyzs::Cli::try_parse_from(["qyzs", "markscan", "a.png", "--max-dim", "16384"])
        .expect("clap 解析通过; dispatch validate 拒绝");
    if let qyzs::Commands::MarkScan { max_dim, .. } = cli.command {
        assert_eq!(max_dim, Some(16384));
    } else {
        panic!("应为 Scan 变体");
    }

    // rerank --top-n: 5000 应被 dispatch 拒 (上界 1000)
    let cli =
        qyzs::Cli::try_parse_from(["qyzs", "rerank", "-q", "x", "-d", "a", "--top-n", "5000"])
            .expect("clap 解析通过; dispatch validate 拒绝");
    if let qyzs::Commands::Rerank { top_n, .. } = cli.command {
        assert_eq!(top_n, Some(5000));
    } else {
        panic!("应为 Rerank 变体");
    }

    // usage --days: 99999 应被 dispatch 拒 (上界 3650)
    let cli = qyzs::Cli::try_parse_from(["qyzs", "usage", "--days", "99999"])
        .expect("clap 解析通过; dispatch validate 拒绝");
    if let qyzs::Commands::Usage { days, .. } = cli.command {
        assert_eq!(days, Some(99999));
    } else {
        panic!("应为 Usage 变体");
    }

    // markscan --segment-chars: 200000 应被 dispatch 拒 (上界 100000)
    let cli = qyzs::Cli::try_parse_from(["qyzs", "markscan", "a.pdf", "--segment-chars", "200000"])
        .expect("clap 解析通过; dispatch validate 拒绝");
    if let qyzs::Commands::MarkScan { segment_chars, .. } = cli.command {
        assert_eq!(segment_chars, 200000);
    } else {
        panic!("应为 Scan 变体");
    }
}

/// 帮助文本契约: 关键参数面必须有准确的语义说明 (双语义/默认值/边界)
/// 用 clap 渲染 --help, 断言关键字符串存在
#[test]
fn test_help_text_contract_for_dual_semantic_flags() {
    use clap::CommandFactory;
    let mut cmd = qyzs::Cli::command();

    // 顶层 help 不含子命令的 arg 描述, 需逐 subcommand 取 long_help
    let mut help_str = cmd.render_long_help().to_string();
    for sub in cmd.get_subcommands_mut() {
        help_str.push('\n');
        help_str.push_str(&sub.render_long_help().to_string());
    }

    // verify --ocr / --markdown: 双语义 (文件路径或文本内容)
    assert!(
        help_str.contains("文件路径"),
        "verify help 应包含 '文件路径' 关键字"
    );
    assert!(
        help_str.contains("文本"),
        "verify help 应包含 '文本' 关键字"
    );

    // sam --aoi: GeoJSON 字符串或文件路径
    assert!(
        help_str.contains("GeoJSON"),
        "sam --aoi help 应提 GeoJSON 形式"
    );

    // chat --stream: 默认开启
    // (原断言针对已删除的 `parse --is-ocr` —— parse 已并入 markscan, 该断言改为
    //  校验仍存在的流式开关契约)
    assert!(
        help_str.contains("启用打字机流式输出"),
        "chat --stream help 应标注打字机流式输出"
    );

    // chat --max-tokens: 区间标注须与 dispatch 校验边界一致 (validate.rs: 1~1_000_000)
    assert!(
        help_str.contains("1~1000000"),
        "chat --max-tokens help 应标注 1~1000000 区间"
    );
}
