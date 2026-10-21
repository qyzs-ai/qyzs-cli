//! markscan 质量报告 (`--report` JSON 输出)
//!
//! CLI 早已声明 `--report` 参数但未实际生成; 本模块兑现该功能, 输出结构化
//! JSON 报告便于程序化消费 (CI 归档、失败回溯、批量对比)。
//!
//! 字段设计原则: 全部 serde-rename (Traceable) + 简洁, LLM/工具友好。

use super::detect::{DetectMethod, SourceFormat};
use super::PipelineMode;
use serde::Serialize;

/// markscan 转换质量报告 (序列化输出到 --report 指定路径)
#[derive(Debug, Serialize)]
pub struct ConversionReport {
    /// 输入路径/URL
    pub input: String,
    /// 输入字节数 (URL 时为 0)
    pub size_bytes: u64,
    /// 格式规范键 (pdf/docx/...)
    pub format: String,
    /// 中文格式标签 (PDF 文档 / CSV 文本表格 / ...)
    pub format_label: String,
    /// 嗅探方式
    pub detect_method: String,
    /// 流水线档位
    pub mode: String,
    /// 润色/理解模型
    pub model: String,
    /// PDF 页数 (其他格式为 null)
    pub page_count: Option<usize>,
    /// MinerU 切片批数 (PDF 大文档 >15 页时)
    pub slice_count: Option<usize>,
    /// 输出 Markdown 字节数
    pub output_bytes: usize,
    /// 输出路径 (stdout 时为 "stdout")
    pub output_target: String,
    /// 总耗时 (秒, 浮点)
    pub duration_secs: f64,
    /// RFC3339 UTC 时间戳
    pub converted_at: String,
    /// CLI 名称与版本
    pub tool: String,
    /// 非致命警告 (如 PDF 单片解析失败被跳过)
    pub warnings: Vec<String>,
    /// 阶段耗时细分 (秒)
    pub stages: Vec<StageTiming>,
    /// 产物中的图片引用总数
    pub image_refs: usize,
    /// 其中指向本地相对路径、但网关未提供字节因而无法落盘的引用数
    pub unresolved_image_refs: usize,
    /// rigor 档交叉验证判定为冲突的段落数 (非 rigor 档为 0)
    pub conflicts: usize,
    /// rigor 档实际生效的相似度阈值 (非 rigor 档为 null)
    pub min_confidence: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct StageTiming {
    pub name: String,
    pub secs: f64,
}

impl ConversionReport {
    /// 构造报告 (前端 caller 已收集字段)
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        input: &str,
        size_bytes: u64,
        format: &SourceFormat,
        detect_method: DetectMethod,
        mode: PipelineMode,
        model: &str,
        page_count: Option<usize>,
        slice_count: Option<usize>,
        output_bytes: usize,
        output_target: &str,
        duration_secs: f64,
        warnings: Vec<String>,
    ) -> Self {
        Self::build_with_details(
            input,
            size_bytes,
            format,
            detect_method,
            mode,
            model,
            page_count,
            slice_count,
            output_bytes,
            output_target,
            duration_secs,
            warnings,
            Vec::new(),
            0,
            0,
            None,
        )
    }

    /// 构造报告 (含阶段耗时/图片引用/冲突数等明细)
    #[allow(clippy::too_many_arguments)]
    pub fn build_with_details(
        input: &str,
        size_bytes: u64,
        format: &SourceFormat,
        detect_method: DetectMethod,
        mode: PipelineMode,
        model: &str,
        page_count: Option<usize>,
        slice_count: Option<usize>,
        output_bytes: usize,
        output_target: &str,
        duration_secs: f64,
        warnings: Vec<String>,
        stages: Vec<StageTiming>,
        image_refs: usize,
        unresolved_image_refs: usize,
        min_confidence: Option<f64>,
    ) -> Self {
        let detect_method_str = match detect_method {
            DetectMethod::ContentSniff => "content-sniff",
            DetectMethod::ExtensionFallback => "extension-fallback",
            DetectMethod::ManualOverride => "manual-override",
        };
        let stages = if stages.is_empty() {
            vec![StageTiming {
                name: "convert".to_string(),
                secs: duration_secs,
            }]
        } else {
            stages
        };
        Self {
            input: input.to_string(),
            size_bytes,
            format: format.key().to_string(),
            format_label: format.label(),
            detect_method: detect_method_str.to_string(),
            mode: mode.key().to_string(),
            model: model.to_string(),
            page_count,
            slice_count,
            output_bytes,
            output_target: output_target.to_string(),
            duration_secs,
            converted_at: super::front_matter::current_rfc3339_for_test(),
            tool: format!("qyzs-cli/{}", env!("CARGO_PKG_VERSION")),
            warnings,
            stages,
            image_refs,
            unresolved_image_refs,
            conflicts: 0,
            min_confidence,
        }
    }

    /// 序列化为 pretty JSON 字符串
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// 写入文件 (UTF-8 pretty JSON; 父目录不存在时自动创建)
    pub fn write_to(&self, path: &str) -> std::io::Result<()> {
        let p = std::path::Path::new(path);
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(path, self.to_json())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_report_basic() {
        let report = ConversionReport::build(
            "doc.pdf",
            12345,
            &SourceFormat::Pdf,
            DetectMethod::ContentSniff,
            PipelineMode::Rigor,
            "dsv4",
            Some(42),
            Some(3),
            6789,
            "stdout",
            12.34,
            vec!["切片 2/3 失败已跳过".to_string()],
        );
        let json = report.to_json();
        assert!(json.contains("\"input\": \"doc.pdf\""));
        assert!(json.contains("\"format\": \"pdf\""));
        assert!(json.contains("\"page_count\": 42"));
        assert!(json.contains("\"slice_count\": 3"));
        assert!(json.contains("\"warnings\""));
        assert!(json.contains("切片 2/3"));
        assert!(json.contains("\"tool\": \"qyzs-cli/"));
    }

    #[test]
    fn test_report_no_warnings_field_omitted_is_still_emitted() {
        // warnings 是 Vec, 即便为空也会作为 [] 输出 (而非 omit), 便于解析
        // page_count/slice_count 是 Option, None 序列化为 null (字段保留, 便于结构消费)
        let report = ConversionReport::build(
            "data.csv",
            999,
            &SourceFormat::Csv,
            DetectMethod::ExtensionFallback,
            PipelineMode::Speed,
            "dsv4",
            None,
            None,
            100,
            "stdout",
            0.5,
            vec![],
        );
        let json = report.to_json();
        assert!(json.contains("\"warnings\": []"));
        assert!(json.contains("\"page_count\": null"));
        assert!(json.contains("\"slice_count\": null"));
    }

    #[test]
    fn test_report_write_to_creates_parent_dirs() {
        // 父目录嵌套不存在: write_to 应自动 create_dir_all 后再写文件
        let tmp = tempfile::tempdir().expect("创建临时目录失败");
        let nested = tmp.path().join("a").join("b").join("c").join("report.json");
        let report = ConversionReport::build(
            "x.pdf",
            1,
            &SourceFormat::Pdf,
            DetectMethod::ContentSniff,
            PipelineMode::Speed,
            "dsv4",
            None,
            None,
            10,
            "stdout",
            0.0,
            vec![],
        );
        report
            .write_to(nested.to_string_lossy().as_ref())
            .expect("嵌套父目录应自动创建");
        assert!(nested.exists(), "报告文件应被写入");
        let content = std::fs::read_to_string(&nested).expect("读报告");
        assert!(content.contains("\"input\": \"x.pdf\""));
    }
}