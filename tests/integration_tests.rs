//! QYZS 核心集成测试套件

use qyzs::config::{Credentials, ResolvedConfig};
use qyzs::error::{exit_code, QyzsError};
use qyzs::pdf::{PdfExtractor, PdfSplitter};
use qyzs::pipeline::{DatasetManager, OcrVerifier};
use qyzs::term::{detect_environment_language, set_language, Language};
use clap::Parser as _;
use std::io::Write;
use tempfile::tempdir;

#[test]
fn test_error_exit_codes() {
    assert_eq!(QyzsError::Usage("test".into()).exit_code(), exit_code::USAGE_ERR);
    assert_eq!(QyzsError::Auth("test".into()).exit_code(), exit_code::AUTH_ERR);
    assert_eq!(QyzsError::Timeout("test".into()).exit_code(), exit_code::TIMEOUT_ERR);
    assert_eq!(QyzsError::Io("test".into()).exit_code(), exit_code::IO_ERR);
    assert_eq!(QyzsError::Pdf("test".into()).exit_code(), exit_code::IO_ERR);
    assert_eq!(QyzsError::Network("test".into()).exit_code(), exit_code::FAILURE);
    assert_eq!(QyzsError::Server("test".into()).exit_code(), exit_code::FAILURE);
}

#[test]
fn test_i18n_detection() {
    // 设置语言测试
    set_language(Language::Zh);
    assert_eq!(qyzs::msg!(AllSystemsOperational), "系统全链路状态正常");

    set_language(Language::En);
    assert_eq!(qyzs::msg!(AllSystemsOperational), "All systems operational");

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
    ).unwrap();

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
    let split_meta = PdfExtractor::read_metadata(split_out.to_str().unwrap()).expect("读取拆分 PDF 元数据失败");
    assert_eq!(split_meta.page_count, 2);

    // 5. 提取文字
    let text = PdfExtractor::extract_text(split_out.to_str().unwrap(), &[1]).expect("提取第一页文字失败");
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
    let count = DatasetManager::extract_zip_to_dir(&zip_data, target_dir.to_str().unwrap()).expect("解压失败");
    assert_eq!(count, 1);
    let extracted_file = target_dir.join("sample").join("test.txt");
    assert!(extracted_file.exists());
    let content = std::fs::read_to_string(extracted_file).unwrap();
    assert_eq!(content, "Hello Qingyang AI");
}

#[test]
fn test_markitdown_cli_arg_surface() {
    // 完整参数面契约：全部参数可解析且取值正确传递
    let cli = qyzs::Cli::try_parse_from([
        "qyzs",
        "markitdown",
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
    .expect("markitdown 完整参数面应可解析");

    if let qyzs::Commands::Markitdown {
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
        assert_eq!(input, "a.pdf");
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
        panic!("应为 Markitdown 命令变体");
    }
}

#[test]
fn test_markitdown_cli_defaults() {
    // 默认值契约：segment_chars=24000、三个开关关闭、mode 留空 (dispatch 层默认 balanced)
    let cli = qyzs::Cli::try_parse_from(["qyzs", "markitdown", "a.pdf", "--dry-run"])
        .expect("markitdown 最小参数应可解析");

    if let qyzs::Commands::Markitdown {
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
        panic!("应为 Markitdown 命令变体");
    }
}

#[test]
fn test_usage_cli_arg_surface() {
    // 参数面契约：--days/--model/--json 可解析且取值正确
    let cli = qyzs::Cli::try_parse_from([
        "qyzs",
        "usage",
        "--days",
        "7",
        "--model",
        "dsv4",
        "--json",
    ])
    .expect("usage 完整参数面应可解析");

    if let qyzs::Commands::Usage {
        days,
        model,
        json,
        clear,
    } = cli.command
    {
        assert_eq!(days, Some(7));
        assert_eq!(model.as_deref(), Some("dsv4"));
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
        json,
        clear,
    } = defaults.command
    {
        assert_eq!(days, None);
        assert_eq!(model, None);
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
        "qyzs", "-b", "http://gw:9200", "-t", "30", "-l", "zh", "-c", "never", "-f", "json",
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
fn test_subcommand_short_flag_additions() {
    // 短语法规范契约: --model 统一 -m (markitdown 补齐), rerank --top-n 补齐 -n
    let cli = qyzs::Cli::try_parse_from([
        "qyzs",
        "markitdown",
        "a.pdf",
        "-m",
        "dsv4",
        "--dry-run",
    ])
    .expect("markitdown -m 应可解析");
    if let qyzs::Commands::Markitdown { model, .. } = cli.command {
        assert_eq!(model.as_deref(), Some("dsv4"));
    } else {
        panic!("应为 Markitdown 命令变体");
    }

    let cli = qyzs::Cli::try_parse_from(["qyzs", "rerank", "-q", "土地", "-d", "A", "-d", "B", "-n", "2"])
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
