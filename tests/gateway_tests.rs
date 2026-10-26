//! markscan 真实网关集成测试 (门控: #[ignore])
//!
//! CI 离线环境自动跳过; 本地验证需注入凭证后手动放行:
//! ```bash
//! set -a; source .env; set +a   # 或已有环境变量
//! cargo test --test gateway_tests -- --ignored --nocapture
//! ```
//!
//! 行为契约依据: tests/docs/QYZS-MARKSCAN.md; 实测校准值见 capability_matrix.toml。

use qyzs::pipeline::markscan::{
    detect::FormatDetector, document::DocumentOutputFormat, MarkScanOptions, MarkScanPipeline,
    PipelineMode,
};
use std::path::Path;

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(rel)
        .to_string_lossy()
        .to_string()
}

fn have_key() -> bool {
    let from_env = ["QYZS_APIKEYS", "QYZS_APIKEY"].iter().any(|name| {
        std::env::var(name)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    });
    from_env
        || qyzs::config::Credentials::load()
            .ok()
            .flatten()
            .map(|credentials| !credentials.all_keys().is_empty())
            .unwrap_or(false)
}

fn speed_opts(input: &str) -> MarkScanOptions {
    MarkScanOptions {
        input: vec![input.to_string()],
        output: None,
        output_format: qyzs::pipeline::markscan::document::DocumentOutputFormat::Markdown,
        recursive: false,
        mode: PipelineMode::Speed,
        type_override: None,
        image_handling: crate_default_image_handling(),
        image_prompt: None,
        max_dim: None,
        concurrency: 4,
        model: None,
        min_confidence: None,
        report: None,
        work_dir: None,
        segment_chars: 24_000,
        resume: false,
        no_clean: false,
        no_front_matter: false,
        dry_run: false,
        assets_dir: None,
        no_assets: false,
        no_classify: false,
        classify_model: None,
    }
}

fn assert_contains_at_least(text: &str, expected: &[&str], minimum: usize, context: &str) {
    let matched: Vec<_> = expected
        .iter()
        .copied()
        .filter(|token| text.contains(token))
        .collect();
    assert!(
        matched.len() >= minimum,
        "{context}: 至少应命中 {minimum} 个关键字符串，实际命中 {matched:?}，输出前 500 字: {}",
        text.chars().take(500).collect::<String>()
    );
}

// MarkScanOptions.image_handling 类型未在 lib 层暴露 Default;
// 显式构造保持测试自洽
fn crate_default_image_handling() -> qyzs::pipeline::markscan::ImageHandling {
    qyzs::pipeline::markscan::ImageHandling::Auto
}

#[tokio::test]
#[ignore = "需要真实网关凭证 (QYZS_APIKEYS/QYZS_APIKEY): cargo test -- --ignored"]
async fn gateway_text_pdf_speed() {
    if !have_key() {
        panic!("未检测到 QYZS_APIKEYS/QYZS_APIKEY; 请注入凭证后运行");
    }
    let config = test_config();
    let client = qyzs::QyzsClient::new(config).unwrap();
    let input = fixture("tests/data/mineru/text_pdf/REF-0056_政府采购评审专家管理办法.pdf");
    let md = MarkScanPipeline::convert(&client, &speed_opts(&input), &input, 300, None)
        .await
        .expect("文本 PDF speed 转换应成功");

    assert!(md.contains("政府采购评审专家管理办法"), "应含文档标题");
    assert!(md.contains("财政部"), "应含发文机关");
    assert!(!md.contains("task_id"), "不得泄漏原始任务 JSON");
}

#[tokio::test]
#[ignore = "需要真实网关凭证 (QYZS_APIKEYS/QYZS_APIKEY): cargo test -- --ignored"]
async fn gateway_image_ocr_auto() {
    if !have_key() {
        panic!("未检测到 QYZS_APIKEYS/QYZS_APIKEY; 请注入凭证后运行");
    }
    let config = test_config();
    let client = qyzs::QyzsClient::new(config).unwrap();
    let input = fixture("tests/data/ocr/images/ref0101_p1_redhead_scan.png");
    let md = MarkScanPipeline::convert(&client, &speed_opts(&input), &input, 300, None)
        .await
        .expect("图像 OCR 转换应成功");

    assert!(
        md.contains("网络安全") || md.contains("自然资源部"),
        "应含红头文字"
    );
}

#[tokio::test]
#[ignore = "需要真实网关凭证 (QYZS_APIKEYS/QYZS_APIKEY): cargo test -- --ignored"]
async fn gateway_paddleocrvl_real_redhead_sample() {
    if !have_key() {
        panic!("未检测到 QYZS_APIKEYS/QYZS_APIKEY; 请注入凭证后运行");
    }
    let client = qyzs::QyzsClient::new(test_config()).unwrap();
    let input = fixture("tests/data/ocr/images/ref0101_p1_redhead_scan.png");
    let result = client
        .ocr_with_model(
            &input,
            Some(
                "逐字转录图片中的全部文字，保留标题、段落、表格、文号、日期和印章内容。只输出转录结果，不要解释。",
            ),
            Some(1280),
            "paddleocrvl",
        )
        .await;
    let capability = qyzs::model_capabilities::ocr_model("paddleocrvl").unwrap();
    if !capability.online_at_test {
        let error = result.expect_err("能力矩阵标记离线时，实时探针不应成功");
        let message = error.to_string();
        assert!(
            message.contains("base_url_missing") || message.contains("timed out"),
            "离线错误应与能力矩阵证据一致，实际: {message}"
        );
        return;
    }
    let response = result.expect("能力矩阵标记在线时，真实红头样本应识别成功");
    assert_eq!(response.model, "paddleocrvl");
    assert_contains_at_least(
        &response.text,
        &[
            "自然资源部网络安全和信息化领导小组办公室文件",
            "自然资网信办发〔2024〕5号",
            "关于进一步加强自然资源部网络安全等级保护工作的指导意见",
        ],
        2,
        "paddleocrvl 红头样本",
    );
}

#[tokio::test]
#[ignore = "需要真实网关凭证 (QYZS_APIKEYS/QYZS_APIKEY): cargo test -- --ignored"]
async fn gateway_qwen3vl_real_seal_sample() {
    if !have_key() {
        panic!("未检测到 QYZS_APIKEYS/QYZS_APIKEY; 请注入凭证后运行");
    }
    let client = qyzs::QyzsClient::new(test_config()).unwrap();
    let input = fixture("tests/data/ocr/images/ref0101_p8_seal_scan.png");
    let response = client
        .chat(
            "逐字转录图片中的正文、日期和印章文字，只输出转录结果。",
            None,
            qyzs::client::ChatOptions {
                model: Some("qwen3-VL-30B-A3B".to_string()),
                temperature: Some(0.0),
                max_tokens: Some(4096),
                top_p: None,
                stream: false,
                show_reasoning: false,
                images: vec![input],
                image_max_dim: Some(2048),
            },
        )
        .await
        .expect("qwen3-VL 真实印章样本应识别成功");
    assert_contains_at_least(
        &response.primary_text(),
        &["自然资源部网络安全和信息化领导小组办公室", "印发", "抄送"],
        1,
        "qwen3-VL 印章样本",
    );
}

#[tokio::test]
#[ignore = "需要真实网关凭证 (QYZS_APIKEYS/QYZS_APIKEY): cargo test -- --ignored"]
async fn gateway_qwen36_real_redhead_sample() {
    if !have_key() {
        panic!("未检测到 QYZS_APIKEYS/QYZS_APIKEY; 请注入凭证后运行");
    }
    let client = qyzs::QyzsClient::new(test_config()).unwrap();
    let input = fixture("tests/data/ocr/images/ref0101_p1_redhead_scan.png");
    let response = client
        .chat(
            "逐字转录图片中的标题、文号和正文，只输出转录结果。",
            None,
            qyzs::client::ChatOptions {
                model: Some("qwen36".to_string()),
                temperature: Some(0.0),
                max_tokens: Some(4096),
                top_p: None,
                stream: false,
                show_reasoning: false,
                images: vec![input],
                image_max_dim: Some(2048),
            },
        )
        .await
        .expect("qwen36 真实红头样本应识别成功");
    assert_contains_at_least(
        &response.primary_text(),
        &[
            "自然资源部网络安全和信息化领导小组办公室文件",
            "自然资网信办发〔2024〕5号",
            "关于进一步加强自然资源部网络安全等级保护工作的指导意见",
        ],
        2,
        "qwen36 红头样本",
    );
}

#[tokio::test]
#[ignore = "需要真实网关凭证 (QYZS_APIKEYS/QYZS_APIKEY): cargo test -- --ignored"]
async fn gateway_text_pdf_html_and_txt_outputs() {
    if !have_key() {
        panic!("未检测到 QYZS_APIKEYS/QYZS_APIKEY; 请注入凭证后运行");
    }
    let client = qyzs::QyzsClient::new(test_config()).unwrap();
    let input = fixture("tests/data/mineru/text_pdf/REF-0056_政府采购评审专家管理办法.pdf");

    let mut html_opts = speed_opts(&input);
    html_opts.output_format = DocumentOutputFormat::Html;
    let html = MarkScanPipeline::convert(&client, &html_opts, &input, 300, None)
        .await
        .expect("真实文本 PDF HTML 输出应成功");
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("政府采购评审专家管理办法"));

    let mut txt_opts = speed_opts(&input);
    txt_opts.output_format = DocumentOutputFormat::Text;
    let text = MarkScanPipeline::convert(&client, &txt_opts, &input, 300, None)
        .await
        .expect("真实文本 PDF TXT 输出应成功");
    assert!(text.contains("政府采购评审专家管理办法"));
    assert!(!text.contains("# 政府采购评审专家管理办法"));
}

#[tokio::test]
#[ignore = "真实扫描 PDF rigor 全流程，需网关凭证且耗时较长"]
async fn gateway_scanned_pdf_rigor_full_pipeline() {
    if !have_key() {
        panic!("未检测到 QYZS_APIKEYS/QYZS_APIKEY; 请注入凭证后运行");
    }
    let client = qyzs::QyzsClient::new(test_config()).unwrap();
    let input = fixture("tests/data/mineru/scanned_pdf/REF-0101_网络安全等级保护指导意见_scan.pdf");
    let mut opts = speed_opts(&input);
    opts.mode = PipelineMode::Rigor;
    opts.segment_chars = 12_000;
    let markdown = MarkScanPipeline::convert(&client, &opts, &input, 900, None)
        .await
        .expect("真实扫描 PDF rigor 全流程应成功");
    assert_contains_at_least(
        &markdown,
        &["自然资源部", "网络安全", "等级保护", "指导意见"],
        3,
        "扫描 PDF rigor 产物",
    );
    assert!(markdown.len() > 2_000, "10 页扫描 PDF 产物不应异常短");
}

#[tokio::test]
#[ignore = "需要真实网关凭证 (QYZS_APIKEYS/QYZS_APIKEY): cargo test -- --ignored"]
async fn gateway_html_url() {
    let config = test_config();
    let client = qyzs::QyzsClient::new(config).unwrap();
    let url = "https://example.com/";
    let md = MarkScanPipeline::convert(&client, &speed_opts(url), url, 60, None)
        .await
        .expect("HTML URL 转换应成功");
    assert!(md.contains("Example Domain"));
}

#[test]
fn gateway_offline_route_guard() {
    // 无凭证也应通过的路由层测试: .doc 路由为不支持 (网关实测拒绝)
    let det = FormatDetector::detect_file(Path::new(&fixture(
        "tests/data/mineru/public_doc/REF-0017_国土空间规划实施监测网络工作方案.doc",
    )))
    .unwrap();
    assert_eq!(det.format.key(), "doc");
}

fn test_config() -> qyzs::config::ResolvedConfig {
    qyzs::config::ResolvedConfig::resolve(None, None, Some(300), None, None, Some("never"), None)
        .expect("测试配置解析失败")
}
