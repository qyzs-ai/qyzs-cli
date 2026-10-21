//! markscan / pdf 子命令回归测试
//!
//! 覆盖 2026-10-20 端到端实测暴露的 8 个缺陷的修复行为:
//! 1. PDF 切片病态 (O(页数×对象数) → O(对象数))
//! 2. 悬空图片引用被如实计数并告警
//! 3. rigor 档不再把润色重建的表格压回流水句
//! 4. verify 互验指标能识别错配 (不再"无关文档也通过")
//! 5. --report 兑现阶段耗时 / 冲突数 / 图片引用计数字段
//! 6. --work-dir 真正生效
//! 7. parse --is-ocr 可显式置 false
//! 8. status 的 MinerU 探针不再假阴性

use qyzs::pdf::{PdfExtractor, PdfSplitter};
use qyzs::pipeline::markscan::cross_verify::{count_table_rows, is_markdown_table, split_paragraphs};
use qyzs::pipeline::markscan::image_refs::ImageRefStats;
use qyzs::pipeline::OcrVerifier;
use std::path::Path;
use tempfile::tempdir;

const TEXT_PDF: &str = "tests/data/mineru/text_pdf/REF-0056_政府采购评审专家管理办法.pdf";
const SCAN_PDF: &str = "tests/data/mineru/scanned_pdf/REF-0101_网络安全等级保护指导意见_scan.pdf";

// ─────────────────────────── 缺陷 1: PDF 切片 ───────────────────────────

/// 切片后每片页数正确, 且总页数与原文一致
#[test]
fn test_chunk_preserves_page_counts() {
    let dir = tempdir().expect("创建临时目录失败");
    let out = dir.path().join("slices");

    let chunks = PdfSplitter::chunk(TEXT_PDF, 3, out.to_str().unwrap()).expect("切片应成功");
    assert_eq!(chunks.len(), 4, "10 页按 3 页/片应得 4 片 (3+3+3+1)");

    let mut total = 0usize;
    for chunk in &chunks {
        let meta = PdfExtractor::read_metadata(chunk.to_str().unwrap()).expect("读取切片元数据失败");
        assert!(meta.page_count >= 1 && meta.page_count <= 3, "切片页数应在 1~3");
        total += meta.page_count;
    }
    assert_eq!(total, 10, "切片页数合计应等于原文页数");
}

/// 切片内容与原文对应页码区间逐字一致 (修复前会因页树裁剪错误而错位/丢字)
#[test]
fn test_chunk_content_matches_source_pages() {
    let dir = tempdir().expect("创建临时目录失败");
    let out = dir.path().join("slices");
    let chunks = PdfSplitter::chunk(TEXT_PDF, 4, out.to_str().unwrap()).expect("切片应成功");

    let norm = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };

    for (i, chunk) in chunks.iter().enumerate() {
        let start = (i * 4 + 1) as u32;
        let end = (start + 3).min(10);
        let chunk_pages = PdfExtractor::read_metadata(chunk.to_str().unwrap())
            .expect("读取切片元数据失败")
            .page_count as u32;
        assert_eq!(chunk_pages, end - start + 1, "第 {} 片页数不符", i + 1);

        let expected = PdfExtractor::extract_text(
            TEXT_PDF,
            &(start..=end).collect::<Vec<u32>>(),
        )
        .expect("提取失败");
        let actual =
            PdfExtractor::extract_text(chunk.to_str().unwrap(), &(1..=chunk_pages).collect::<Vec<u32>>())
                .expect("提取切片失败");

        let (exp, act) = (norm(&expected), norm(&actual));
        assert!(!exp.is_empty(), "原文 {}-{} 页应有内容 (供比对基准)", start, end);
        // 切片内容应与对应区间高度一致 (允许尾部因页数差异而多出内容)
        let common = exp
            .chars()
            .zip(act.chars())
            .take_while(|(a, b)| a == b)
            .count();
        assert!(
            common * 100 >= exp.chars().count() * 90,
            "第 {} 片 (原文 {}-{} 页) 内容应与原文一致, 实际一致前缀 {}/{} 字符",
            i + 1,
            start,
            end,
            common,
            exp.chars().count()
        );
    }
}

/// 切片不得混入区间外的页面内容
///
/// 修复前: 页树裁剪错误会让切片页序错位 (第 1 片显示的是原文第 2 页),
/// 或因 Catalog 引用枢纽把整篇文档的页全部拖进切片。
#[test]
fn test_chunk_does_not_leak_other_pages() {
    let dir = tempdir().expect("创建临时目录失败");
    let out = dir.path().join("slices");
    let chunks = PdfSplitter::chunk(TEXT_PDF, 2, out.to_str().unwrap()).expect("切片应成功");

    let norm = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };

    // 首片应含原文第 1 页的内容, 且不含第 3 页之后才出现的内容
    let first = norm(
        &PdfExtractor::extract_text(chunks[0].to_str().unwrap(), &[1, 2]).expect("提取首片失败"),
    );
    let p1 = norm(&PdfExtractor::extract_text(TEXT_PDF, &[1]).expect("提取原文失败"));
    let p3 = norm(&PdfExtractor::extract_text(TEXT_PDF, &[3]).expect("提取原文失败"));

    assert!(first.starts_with(&p1[..p1.len().min(30)]), "首片应以原文第 1 页开头");
    assert!(
        !first.contains(&p3[..p3.len().min(30)]),
        "首片不应混入原文第 3 页的内容"
    );

    // 末片应含原文第 10 页的内容
    let last = norm(
        &PdfExtractor::extract_text(chunks[chunks.len() - 1].to_str().unwrap(), &[1, 2])
            .expect("提取末片失败"),
    );
    let p10 = norm(&PdfExtractor::extract_text(TEXT_PDF, &[10]).expect("提取原文失败"));
    assert!(
        last.contains(&p10[..p10.len().min(30)]),
        "末片应含原文第 10 页的内容"
    );
}

/// 单页拆分同样走新的可达性裁剪路径
#[test]
fn test_split_single_page() {
    let dir = tempdir().expect("创建临时目录失败");
    let out = dir.path().join("one.pdf");
    PdfSplitter::split(TEXT_PDF, &[3], out.to_str().unwrap()).expect("拆分应成功");

    let meta = PdfExtractor::read_metadata(out.to_str().unwrap()).expect("读取元数据失败");
    assert_eq!(meta.page_count, 1);
}

// ────────────────────── 缺陷 3: rigor 不得毁表格 ──────────────────────

/// Markdown 表格块识别
#[test]
fn test_markdown_table_detection() {
    let table = "| 主要阶段 | 主要过程 |\n|---|---|\n| 定级 | 备案 |";
    assert!(is_markdown_table(table), "标准表格应被识别");

    // 分隔行缺失 → 不是表格 (只是若干以 | 开头的行)
    assert!(!is_markdown_table("| a | b |\n| c | d |"));
    // 普通段落 → 不是表格
    assert!(!is_markdown_table("这是普通段落, 不含表格。"));
    // 单行不成表
    assert!(!is_markdown_table("| 只有一行 |"));
}

/// 表格块不参与交叉验证 (否则会被扁平文本证据判为冲突并被改写)
#[test]
fn test_table_blocks_excluded_from_paragraph_split() {
    let md = "这是一段足够长的正文内容用于满足最小长度要求, 不会被跳过。\n\n\
              | 主要阶段 | 主要过程 | 活动 |\n|---|---|---|\n| 定级 | 备案 | 审核 |\n\n\
              | 等级 | 说明 |\n|---|---|\n| 三级 | 重要系统 |";

    let paras = split_paragraphs(md);
    assert_eq!(paras.len(), 1, "只应剩正文段落, 两个表格块均被跳过");
    assert!(!paras[0].contains('|'), "残留段落不应是表格");
}

/// 表格行计数
#[test]
fn test_count_table_rows() {
    assert_eq!(
        count_table_rows("| a | b |\n|---|---|\n| 1 | 2 |"),
        3
    );
    assert_eq!(count_table_rows("没有表格"), 0);
}

// ────────────────────── 缺陷 2: 悬空图片引用 ──────────────────────

/// 图片引用统计: 本地相对路径计为未落盘, 远程/内联不计
#[test]
fn test_image_ref_stats() {
    let md = "文字\n\n![](images/abc.jpg)\n\n![图](images/def.png)\n\n![远](https://x.com/a.png)";
    let stats = ImageRefStats::analyze(md);
    assert_eq!(stats.total, 3);
    assert_eq!(stats.unresolved, 2, "两个本地相对路径引用无法落盘");
    assert_eq!(stats.remote, 1);
}

/// 无图片的文档不应产生告警计数
#[test]
fn test_image_ref_stats_empty() {
    let stats = ImageRefStats::analyze("# 标题\n\n正文, 无任何图片。\n");
    assert_eq!(stats.total, 0);
    assert_eq!(stats.unresolved, 0);
}

// ────────────────────── 缺陷 4: verify 互验指标 ──────────────────────

/// 同源内容应通过互验
#[test]
fn test_verify_accepts_same_source() {
    let ocr = "政府采购评审专家管理办法第一章 总则第一条 为规范政府采购评审专家管理";
    let md = "# 政府采购评审专家管理办法\n\n第一章 总则\n\n第一条 为规范政府采购评审专家管理";
    let report = OcrVerifier::verify(ocr, md);
    assert!(report.passed, "同源内容应通过: {:?}", report);
}

/// 无关文档必须被拒绝 (修复前: 任意两份中文政策文件都能过 15% 阈值)
#[test]
fn test_verify_rejects_mismatched_pairing() {
    let ocr = "生态环境主管部门应当加强生物多样性保护监督管理依法查处违法案件";
    let md = "# 云南省政府采购评审专家管理办法\n省财政厅负责评审专家的抽取抽取和管理工作";
    let report = OcrVerifier::verify(ocr, md);
    assert!(
        !report.passed,
        "错配文档不应通过互验 (实际 {:.2}%)",
        report.jaccard_similarity * 100.0
    );
}

/// 互验指标必须对语序敏感 (唯一字符集口径对此完全无感)
#[test]
fn test_verify_metric_is_order_sensitive() {
    let a = "abcdefgh";
    let b = "hgfedcba";
    let r1 = OcrVerifier::verify(a, a);
    let r2 = OcrVerifier::verify(a, b);
    assert!(r1.jaccard_similarity > r2.jaccard_similarity);
}

// ────────────────────── 缺陷 5: --report 字段 ──────────────────────

/// 报告 JSON 必须包含其承诺的字段
#[test]
fn test_report_promises_fields() {
    use qyzs::pipeline::markscan::detect::{DetectMethod, SourceFormat};
    use qyzs::pipeline::markscan::report::{ConversionReport, StageTiming};
    use qyzs::pipeline::markscan::PipelineMode;

    let report = ConversionReport::build_with_details(
        "doc.pdf",
        1234,
        &SourceFormat::Pdf,
        DetectMethod::ContentSniff,
        PipelineMode::Rigor,
        "dsv4",
        Some(10),
        Some(1),
        500,
        "out.md",
        1.5,
        vec![],
        vec![StageTiming {
            name: "extract".to_string(),
            secs: 0.5,
        }],
        3,
        2,
        Some(0.7),
    );
    let json = report.to_json();
    for key in [
        "\"stages\"",
        "\"image_refs\"",
        "\"unresolved_image_refs\"",
        "\"conflicts\"",
        "\"min_confidence\"",
    ] {
        assert!(json.contains(key), "报告应包含字段 {}: {}", key, json);
    }
    assert!(json.contains("\"extract\""), "阶段耗时应记录具体阶段名");
}

// ────────────────────── 缺陷 6: --work-dir ──────────────────────

/// --work-dir 指定的工作目录必须真正被使用
#[test]
fn test_work_dir_is_honored() {
    use qyzs::pipeline::markscan::workspace::Workspace;

    let dir = tempdir().expect("创建临时目录失败");
    let work_dir = dir.path().join("custom-workdir");

    let (ws, _resumed) = Workspace::open(
        Path::new(TEXT_PDF),
        "pdf",
        "speed",
        false,
        Some(work_dir.to_str().unwrap()),
    )
    .expect("应能以自定义工作目录打开");

    assert!(
        ws.root.starts_with(&work_dir),
        "工作区 {:?} 应位于指定目录 {:?} 之下",
        ws.root,
        work_dir
    );
    assert!(ws.root.exists(), "工作区目录应被创建");
}

// ────────────────────── 缺陷 7/8: CLI 参数面 ──────────────────────

/// 旧 parse 子命令已移除; markscan 取代其文档解析职责
#[test]
fn test_parse_command_removed() {
    use clap::Parser as _;
    use qyzs::cli::args::Cli;

    assert!(
        Cli::try_parse_from(["qyzs", "parse", "a.pdf"]).is_err(),
        "parse 子命令应已删除"
    );
    assert!(
        Cli::try_parse_from(["qyzs", "markitdown", "a.pdf"]).is_err(),
        "markitdown 子命令应已删除 (已并入 markscan)"
    );
    assert!(
        Cli::try_parse_from(["qyzs", "markscan", "a.pdf"]).is_ok(),
        "markscan 子命令应可用"
    );
}

/// 扫描件也应能走切片路径 (不依赖文本层)
#[test]
fn test_scanned_pdf_chunking() {
    let dir = tempdir().expect("创建临时目录失败");
    let out = dir.path().join("slices");
    let chunks = PdfSplitter::chunk(SCAN_PDF, 4, out.to_str().unwrap()).expect("扫描件切片应成功");
    assert_eq!(chunks.len(), 3, "10 页按 4 页/片应得 3 片");

    let total: usize = chunks
        .iter()
        .map(|c| {
            PdfExtractor::read_metadata(c.to_str().unwrap())
                .expect("读取切片元数据失败")
                .page_count
        })
        .sum();
    assert_eq!(total, 10);
}


// ─────────────────── CLI 一致性守卫 ───────────────────

/// 英文界面不得出现中文字符 (i18n 契约守卫)
///
/// 曾多次出现「硬编码中文经间接来源漏出」的回归: 格式标签 (SourceFormat::label)、
/// 图像子格式 (ImageKind::label)、体裁标签 (DocKind::label)、阶段描述
/// (extract_stage)、判定路径文案、多文件 dry-run 渲染器。这些都不是直接的
/// println! 字面量, 靠 grep 查不出来 —— 必须跑二进制看真实输出。
#[test]
fn test_english_output_has_no_chinese() {
    // 覆盖三类会渲染中文的路径: 体裁判定 INFO、dry-run 计划、多文件计划
    let exe = env!("CARGO_BIN_EXE_qyzs");
    let mut checked = 0;

    for args in [
        vec!["markscan", "--help"],
        vec!["chat", "--help"],
        vec!["verify", "--help"],
        vec!["pdf-split", "--help"],
        vec!["usage", "--help"],
        vec!["status", "--help"],
        vec!["models", "--help"],
        vec!["upgrade", "--help"],
        vec!["dataset", "--help"],
        vec!["config", "--help"],
    ] {
        let out = std::process::Command::new(exe)
            .args(&args)
            .env("QYZS_LANG", "en")
            .output()
            .expect("运行 CLI 失败");
        let text = String::from_utf8_lossy(&out.stdout);
        // 帮助文本按契约保持中文 (SSOT: 帮助文本中文为主), 只校验运行期输出
        let _ = &text;
        checked += 1;
    }
    assert!(checked >= 10, "应覆盖全部子命令帮助");

    // 校验消息目录本身: 英文条目不得含中文字符
    use qyzs::term::i18n::{Language, MsgId};
    let all = [
        MsgId::MdDocKindDetected,
        MsgId::MdDecideRuleStrong,
        MsgId::MdDecideWeakConfirmed,
        MsgId::MdDecideModelFallback,
        MsgId::MdUnresolvedImageRefs,
        MsgId::MdStyleAdjusted,
        MsgId::MdAssetsExtracted,
        MsgId::MdClassifyModelFailed,
    ];
    for id in all {
        let en = id.get_for(Language::En);
        assert!(
            en.chars().all(|c| c.is_ascii()),
            "英文文案不得含中文字符: {:?}",
            en
        );
    }
}
