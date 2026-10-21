//! scan 多文件/目录输入编排
//!
//! 展开 ScanOptions.input:
//! - 本地路径是目录 → 顶层 (--recursive 递归) 列出文件
//! - 多个 input → 依次处理
//! - URL 原样 (单文件输入才允许 URL)
//!
//! 输出策略:
//! - 单文件输入: -o 是文件 (默认 stdout, 与历史契约一致)
//! - 多文件输入: -o 必须是目录 (自动创建), 每输入一个 .md, 文件名 = 输入 basename + .md
//!
//! 失败处理: 当前默认 abort (任一文件失败终止整批); 不在此处引入
//! --continue-on-error 之类的开关 (未在 CLI 定义, 避免前瞻性设计)。

use super::detect::SourceFormat;
use super::ScanOptions;
use super::ScanPipeline;
use super::PipelineMode;
use crate::client::QyzsClient;
use crate::error::{QyzsError, QyzsResult};
use std::path::{Path, PathBuf};

/// 一次实际要处理的单文件任务 (经目录/多文件展开后)
#[derive(Debug, Clone)]
pub struct ResolvedInput {
    /// 原始用户输入
    pub raw: String,
    /// 处理后的输出文件路径 (None = stdout, 仅单文件输入允许)
    pub output: Option<PathBuf>,
}

/// 展开 opts.input 为有序单文件列表
///
/// 规则:
/// - URL 输入 (http:// / https://) 原样保留, 不做目录展开
/// - 本地路径是文件 → 原样
/// - 本地路径是目录 → 按 opts.recursive 决定是否递归, 收集内部文件
/// - 文件按名字典序排序, 保证确定性输出
pub fn resolve_inputs(opts: &ScanOptions) -> QyzsResult<Vec<ResolvedInput>> {
    let inputs = &opts.input;
    if inputs.is_empty() {
        return Err(QyzsError::Usage("未提供任何输入".to_string()));
    }

    let is_multi = inputs.len() > 1
        || inputs.iter().any(|p| !is_url(p) && Path::new(p).is_dir());

    // 多文件场景: -o 必须是目录 (已存在或待创建)
    let output_dir: Option<PathBuf> = if is_multi {
        match &opts.output {
            Some(o) => {
                let p = PathBuf::from(o);
                if p.exists() && !p.is_dir() {
                    return Err(QyzsError::Usage(format!(
                        "多文件输入时 -o 必须是目录, 但 '{}' 是已存在文件",
                        o
                    )));
                }
                std::fs::create_dir_all(&p)?;
                Some(p)
            }
            None => {
                // 默认多文件输出到 ./scan_out/
                let default = PathBuf::from("scan_out");
                std::fs::create_dir_all(&default)?;
                Some(default)
            }
        }
    } else {
        // 单文件输入: -o 是文件 (或 None = stdout)
        if let Some(o) = &opts.output {
            let p = PathBuf::from(o);
            if p.exists() && p.is_dir() {
                return Err(QyzsError::Usage(format!(
                    "单文件输入时 -o 必须是文件, 但 '{}' 是已存在目录",
                    o
                )));
            }
        }
        None
    };

    let mut resolved: Vec<ResolvedInput> = Vec::new();
    for raw in inputs {
        if is_url(raw) {
            // URL 单文件
            let out = match &output_dir {
                Some(dir) => Some(dir.join(slugify_url(raw) + ".md")),
                None => opts.output.as_ref().map(PathBuf::from),
            };
            resolved.push(ResolvedInput {
                raw: raw.clone(),
                output: out,
            });
            continue;
        }

        let path = PathBuf::from(raw);
        if !path.exists() {
            return Err(QyzsError::Usage(format!("输入路径不存在: {}", raw)));
        }

        if path.is_file() {
            let out = match &output_dir {
                Some(dir) => Some(dir.join(stem_with_ext_md(&path))),
                None => opts.output.as_ref().map(PathBuf::from),
            };
            resolved.push(ResolvedInput {
                raw: raw.clone(),
                output: out,
            });
        } else if path.is_dir() {
            let mut files: Vec<PathBuf> = Vec::new();
            collect_files(&path, opts.recursive, &mut files)?;
            files.sort();
            for f in files {
                let out = match &output_dir {
                    Some(dir) => {
                        // 保留子目录结构: dir/relpath/of/input/file.md
                        let rel = f.strip_prefix(&path).unwrap_or(&f);
                        Some(dir.join(rel).with_extension("md"))
                    }
                    None => None,
                };
                resolved.push(ResolvedInput {
                    raw: f.to_string_lossy().into_owned(),
                    output: out,
                });
            }
        }
    }

    if resolved.is_empty() {
        return Err(QyzsError::Usage(
            "未解析到任何可处理文件 (目录可能为空)".to_string(),
        ));
    }
    Ok(resolved)
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// 收集目录下所有常规文件 (按 opts.recursive 决定是否递归)
fn collect_files(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) -> QyzsResult<()> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| QyzsError::Io(format!("读取目录失败 {}: {}", dir.display(), e)))?;
    for entry in entries {
        let entry = entry.map_err(|e| QyzsError::Io(e.to_string()))?;
        let p = entry.path();
        if p.is_file() {
            out.push(p);
        } else if p.is_dir() && recursive {
            collect_files(&p, recursive, out)?;
        }
    }
    Ok(())
}

/// 路径 → "stem.md" (basename 去原扩展名, 加 .md)
fn stem_with_ext_md(p: &Path) -> PathBuf {
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    PathBuf::from(format!("{}.md", stem))
}

/// URL → 文件名 slug (去掉 scheme 与查询串, 非法字符替换)
fn slugify_url(url: &str) -> String {
    let stripped = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let without_query = stripped.split('?').next().unwrap_or(stripped);
    let mut out = String::with_capacity(without_query.len());
    for ch in without_query.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("output");
    }
    out
}

/// 顶层多文件编排入口 (dry-run)
pub async fn plan_multi(
    opts: &ScanOptions,
    timeout_secs: u64,
) -> QyzsResult<Vec<super::ConversionPlan>> {
    let inputs = resolve_inputs(opts)?;
    let mut plans = Vec::with_capacity(inputs.len());
    for r in &inputs {
        let plan = ScanPipeline::plan(opts, &r.raw, timeout_secs).await?;
        plans.push(plan);
    }
    Ok(plans)
}

/// 顶层多文件编排入口 (实际转换)
///
/// 对每个 ResolvedInput 串行调用 convert; 失败立即返回错误 (中止整批)。
pub async fn convert_multi(
    client: &QyzsClient,
    opts: &ScanOptions,
    timeout_secs: u64,
    on_progress: Option<&dyn Fn(usize, usize)>,
) -> QyzsResult<Vec<(ResolvedInput, String)>> {
    let inputs = resolve_inputs(opts)?;
    let total = inputs.len();
    let mut results = Vec::with_capacity(total);
    for (idx, r) in inputs.iter().enumerate() {
        // 覆盖 opts.recursive 意义 (展开已发生, 子项按文件处理)
        let md = match ScanPipeline::convert(client, opts, &r.raw, timeout_secs, on_progress).await
        {
            Ok(md) => md,
            // 判定为照片等情形: 跳过该文件但继续整批, 不写输出
            Err(QyzsError::Skipped(why)) => {
                eprintln!("\x1b[33m[SKIP]\x1b[0m [{}/{}] {}", idx + 1, total, why);
                if let Some(cb) = on_progress {
                    cb(idx + 1, total);
                }
                continue;
            }
            Err(e) => {
                return Err(QyzsError::Server(format!(
                    "[{}/{}] {} 转换失败: {}",
                    idx + 1,
                    total,
                    r.raw,
                    e
                )))
            }
        };
        if let Some(out_p) = &r.output {
            if let Some(parent) = out_p.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            std::fs::write(out_p, &md)?;
        }
        if let Some(cb) = on_progress {
            cb(idx + 1, total);
        }
        results.push((r.clone(), md));
    }
    Ok(results)
}

/// 一次 dry-run 输出多文件时, 把每个 plan 串行渲染为人类可读
///
/// 文案与单文件 dry-run 走同一套 i18n 目录 (此前此处硬编码中文, 英文界面
/// 会中英混排, 且字段集比单文件版少: 缺输入源/大小/润色模型/原因标签)。
pub fn render_plans_human(plans: &[super::ConversionPlan]) {
    for (i, plan) in plans.iter().enumerate() {
        if plans.len() > 1 {
            println!("\n────── [{} / {}] ──────", i + 1, plans.len());
        }
        println!(
            "{}{} ({}, {})",
            crate::msg!(MdLabelInput),
            plan.input,
            if plan.input_source == "url" {
                crate::msg!(MdInputUrl)
            } else {
                crate::msg!(MdInputLocal)
            },
            super::human_size(plan.size_bytes)
        );
        println!(
            "{}{} ({})",
            crate::msg!(MdLabelFormat),
            plan.detected_label,
            match plan.detect_method {
                "content-sniff" => crate::msg!(MdDetectContent),
                "extension-fallback" => crate::msg!(MdDetectExtension),
                _ => crate::msg!(MdDetectManual),
            }
        );
        if let (Some(pages), Some(slices)) = (plan.page_count, plan.slice_count) {
            println!(
                "{}",
                crate::msgf!(
                    MdLabelPdfPages,
                    pages,
                    slices,
                    super::pdf::SLICE_BATCH_PAGES
                )
            );
        }
        println!("{}{}", crate::msg!(MdLabelMode), plan.mode);
        println!("{}{}", crate::msg!(MdLabelChannel), plan.channel.unwrap_or("—"));
        if plan.supported {
            println!("{}{}", crate::msg!(MdLabelStages), plan.stages.join(" → "));
        } else {
            println!(
                "{}{}",
                crate::msg!(MdReason),
                plan.unsupported_reason.as_deref().unwrap_or("—")
            );
        }
        if plan.mode != "speed" {
            println!("{}{}", crate::msg!(MdPolishModel), plan.polish_model);
        }
        println!("{}{}", crate::msg!(MdOutputTarget), plan.output_target);
        for w in &plan.warnings {
            println!("  ⚠ {}", w);
        }
    }
}

#[allow(dead_code)]
fn _pipeline_mode_marker() -> PipelineMode {
    PipelineMode::Balanced
}

#[allow(dead_code)]
fn _source_format_marker() -> SourceFormat {
    SourceFormat::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(inputs: Vec<String>, output: Option<String>, recursive: bool) -> ScanOptions {
        ScanOptions {
            input: inputs,
            output,
            recursive,
            mode: PipelineMode::Balanced,
            type_override: None,
            image_handling: super::super::ImageHandling::Auto,
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

    #[test]
    fn test_single_file_input() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.csv");
        std::fs::write(&f, b"k,v\n1,2\n").unwrap();
        let o = opts(vec![f.to_string_lossy().into_owned()], None, false);
        let resolved = resolve_inputs(&o).unwrap();
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].output.is_none());
    }

    #[test]
    fn test_directory_input_creates_default_output() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.csv"), b"k,v\n1,2\n").unwrap();
        std::fs::write(dir.path().join("b.json"), br#"{"x":1}"#).unwrap();
        let o = opts(vec![dir.path().to_string_lossy().into_owned()], None, false);
        let resolved = resolve_inputs(&o).unwrap();
        assert_eq!(resolved.len(), 2);
        // 默认输出目录 = ./scan_out/
        assert!(resolved[0].output.is_some());
        // 文件名 = 原 basename + .md
        let names: Vec<String> = resolved
            .iter()
            .map(|r| {
                r.output
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert!(names.contains(&"a.md".to_string()));
        assert!(names.contains(&"b.md".to_string()));
        // 清理默认目录
        let _ = std::fs::remove_dir_all("scan_out");
    }

    #[test]
    fn test_recursive_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(dir.path().join("top.csv"), b"a\n1\n").unwrap();
        std::fs::write(sub.join("deep.json"), br#"{"x":1}"#).unwrap();
        let o = opts(
            vec![dir.path().to_string_lossy().into_owned()],
            Some(tempfile::tempdir().unwrap().path().to_string_lossy().into_owned()),
            true,
        );
        let resolved = resolve_inputs(&o).unwrap();
        assert_eq!(resolved.len(), 2);
    }

    #[test]
    fn test_non_recursive_skips_subdir() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(dir.path().join("top.csv"), b"a\n1\n").unwrap();
        std::fs::write(sub.join("deep.json"), br#"{"x":1}"#).unwrap();
        let o = opts(
            vec![dir.path().to_string_lossy().into_owned()],
            Some(tempfile::tempdir().unwrap().path().to_string_lossy().into_owned()),
            false,
        );
        let resolved = resolve_inputs(&o).unwrap();
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].raw.ends_with("top.csv"));
    }

    #[test]
    fn test_multi_file_output_dir_required() {
        let dir = tempfile::tempdir().unwrap();
        let f1 = dir.path().join("a.csv");
        let f2 = dir.path().join("b.json");
        std::fs::write(&f1, b"a\n1\n").unwrap();
        std::fs::write(&f2, b"{\"x\":1}").unwrap();
        // -o 指向已存在文件 → 错误
        let existing_file = dir.path().join("not_a_dir");
        std::fs::write(&existing_file, b"x").unwrap();
        let o = opts(
            vec![f1.to_string_lossy().into_owned(), f2.to_string_lossy().into_owned()],
            Some(existing_file.to_string_lossy().into_owned()),
            false,
        );
        let err = resolve_inputs(&o).unwrap_err();
        assert!(matches!(err, QyzsError::Usage(_)));
    }

    #[test]
    fn test_single_file_output_must_not_be_dir() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.csv");
        std::fs::write(&f, b"a\n1\n").unwrap();
        let existing_dir = dir.path().join("not_a_file");
        std::fs::create_dir(&existing_dir).unwrap();
        let o = opts(
            vec![f.to_string_lossy().into_owned()],
            Some(existing_dir.to_string_lossy().into_owned()),
            false,
        );
        let err = resolve_inputs(&o).unwrap_err();
        assert!(matches!(err, QyzsError::Usage(_)));
    }

    #[test]
    fn test_slugify_url() {
        // scheme 去掉, query 串去掉 (split '?'), 路径分隔符 / 变 _
        assert_eq!(slugify_url("https://example.com/path/file.pdf"), "example.com_path_file.pdf");
        assert_eq!(slugify_url("http://x.com/a?b=1&c=2"), "x.com_a");
        assert_eq!(slugify_url("https://x.com/"), "x.com_");
    }

    #[test]
    fn test_stem_with_ext_md() {
        let p = PathBuf::from("/tmp/foo.csv");
        assert_eq!(stem_with_ext_md(&p), PathBuf::from("foo.md"));
        let p = PathBuf::from("/tmp/no_ext");
        assert_eq!(stem_with_ext_md(&p), PathBuf::from("no_ext.md"));
    }
}
