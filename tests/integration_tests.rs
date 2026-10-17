//! QYZS 核心集成测试套件

use qyzs::config::{Credentials, ResolvedConfig};
use qyzs::error::{exit_code, QyzsError};
use qyzs::pdf::{PdfExtractor, PdfSplitter};
use qyzs::pipeline::{CrossVerifier, DatasetManager};
use qyzs::term::{detect_environment_language, set_language, Language};
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
    let report = CrossVerifier::verify(ocr, md);
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
