//! markscan 真实网关集成测试 (门控: #[ignore])
//!
//! CI 离线环境自动跳过; 本地验证需注入凭证后手动放行:
//! ```bash
//! export $(grep -v '^#' .env | grep QYZS_APIKEY)   # 或已有环境变量
//! cargo test --test gateway_tests -- --ignored --nocapture
//! ```
//!
//! 行为契约依据: tests/docs/QYZS-MARKITDOWN.md; 实测校准值见 capability_matrix.toml。

use qyzs::pipeline::markscan::{
    detect::FormatDetector, MarkScanOptions, MarkScanPipeline, PipelineMode,
};
use std::path::Path;

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(rel)
        .to_string_lossy()
        .to_string()
}

fn have_key() -> bool {
    std::env::var("QYZS_APIKEY").map(|v| !v.is_empty()).unwrap_or(false)
}

fn speed_opts(input: &str) -> MarkScanOptions {
    MarkScanOptions {
        input: vec![input.to_string()],
        output: None,
        recursive: false,
        mode: PipelineMode::Speed,
        type_override: None,
        image_handling: crate_default_image_handling(),
        image_prompt: None,
        max_dim: None,
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

// MarkScanOptions.image_handling 类型未在 lib 层暴露 Default;
// 显式构造保持测试自洽
fn crate_default_image_handling() -> qyzs::pipeline::markscan::ImageHandling {
    qyzs::pipeline::markscan::ImageHandling::Auto
}

#[tokio::test]
#[ignore = "需要真实网关凭证 (QYZS_APIKEY): cargo test -- --ignored"]
async fn gateway_text_pdf_speed() {
    if !have_key() {
        panic!("未检测到 QYZS_APIKEY; 请注入凭证后运行");
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
#[ignore = "需要真实网关凭证 (QYZS_APIKEY): cargo test -- --ignored"]
async fn gateway_image_ocr_auto() {
    if !have_key() {
        panic!("未检测到 QYZS_APIKEY; 请注入凭证后运行");
    }
    let config = test_config();
    let client = qyzs::QyzsClient::new(config).unwrap();
    let input = fixture("tests/data/ocr/images/ref0101_p1_redhead_scan.png");
    let md = MarkScanPipeline::convert(&client, &speed_opts(&input), &input, 300, None)
        .await
        .expect("图像 OCR 转换应成功");

    assert!(md.contains("网络安全") || md.contains("自然资源部"), "应含红头文字");
}

#[tokio::test]
#[ignore = "需要真实网关凭证 (QYZS_APIKEY): cargo test -- --ignored"]
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
    qyzs::config::ResolvedConfig::resolve(
        std::env::var("QYZS_APIKEY").ok().as_deref(),
        None,
        Some(300),
        None,
        None,
        Some("never"),
        None,
    )
    .expect("测试配置解析失败")
}
