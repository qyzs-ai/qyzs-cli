//! markscan YAML front matter 生成
//!
//! 在每个 .md 输出开头 prepend YAML 元数据块, 便于 RAG 链路消费结构化信息。
//! 默认开启, --no-front-matter 关闭。
//!
//! 字段:
//! - source: 原始输入路径/URL
//! - format: 检测到的格式键
//! - detect_method: content-sniff / extension-fallback / manual-override
//! - size_bytes: 输入文件字节数
//! - mode: speed/rigor
//! - model: 润色/理解模型
//! - page_count: PDF 专属, 可选
//! - converted_at: RFC3339 UTC 时间戳
//! - tool: qyzs-cli/<version>

use super::detect::DetectMethod;
use super::PipelineMode;
use std::time::{SystemTime, UNIX_EPOCH};

/// 字段元数据 (prepend 到 markdown 开头)
pub struct FrontMatter<'a> {
    pub source: &'a str,
    pub format: &'a str,
    pub detect_method: DetectMethod,
    pub size_bytes: u64,
    pub mode: PipelineMode,
    pub model: &'a str,
    pub page_count: Option<usize>,
}

/// 渲染 YAML front matter 块 (含开闭 `---`)
pub fn render(fm: &FrontMatter<'_>) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("---\n");
    out.push_str(&format!("source: {}\n", yaml_scalar(fm.source)));
    out.push_str(&format!("format: {}\n", yaml_scalar(fm.format)));
    out.push_str(&format!(
        "detect_method: {}\n",
        match fm.detect_method {
            DetectMethod::ContentSniff => "content-sniff",
            DetectMethod::ExtensionFallback => "extension-fallback",
            DetectMethod::ManualOverride => "manual-override",
        }
    ));
    out.push_str(&format!("size_bytes: {}\n", fm.size_bytes));
    out.push_str(&format!("mode: {}\n", fm.mode.key()));
    out.push_str(&format!("model: {}\n", yaml_scalar(fm.model)));
    if let Some(p) = fm.page_count {
        out.push_str(&format!("page_count: {}\n", p));
    }
    out.push_str(&format!("converted_at: {}\n", current_rfc3339()));
    out.push_str(&format!("tool: qyzs-cli/{}\n", env!("CARGO_PKG_VERSION")));
    out.push_str("---\n\n");
    out
}

/// YAML scalar 值转义: 含特殊字符时整体加双引号并转义内部双引号/反斜杠
fn yaml_scalar(s: &str) -> String {
    if s.is_empty() {
        return "\"\"".to_string();
    }
    // 判定: ASCII 安全字符 (字母/数字/_/-/./:/~//) 与非 ASCII 字符 (含中文)
    // 视为 unquoted scalar 合法; 其他 (空格/特殊符号) 加双引号并转义
    let needs_quote = s.chars().any(|c| !is_yaml_unquoted_char(c));
    if !needs_quote {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn is_yaml_unquoted_char(c: char) -> bool {
    if c.is_ascii_alphanumeric() {
        return true;
    }
    // 显式允许的标点 (URL/path 中常见)
    if matches!(c, '_' | '-' | '.' | '/' | ':' | '~') {
        return true;
    }
    // 非 ASCII 字符 (中文等) YAML 1.2 视为合法
    if (c as u32) >= 0x80 {
        return true;
    }
    false
}

/// 当前 UTC 时间戳 (RFC3339 格式: YYYY-MM-DDTHH:MM:SSZ)
fn current_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_rfc3339(secs)
}

/// 与 current_rfc3339 同语义, 暴露给兄弟模块 (report) 使用
pub fn current_rfc3339_for_test() -> String {
    current_rfc3339()
}

fn format_rfc3339(epoch_secs: u64) -> String {
    // 天数从 1970-01-01 起算 (Gregorian, 简化处理闰年)
    let secs_per_day = 86_400u64;
    let days = epoch_secs / secs_per_day;
    let secs_of_day = (epoch_secs % secs_per_day) as u32;
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    let (year, month, day) = days_to_ymd(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

fn days_to_ymd(days: u64) -> (i32, u32, u32) {
    // Howard Hinnant 算法: 转换 days since 1970-01-01 → 民用日期
    // 移位常数: 1970-01-01 是 unix epoch, 算法核心
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_basic() {
        let fm = FrontMatter {
            source: "doc.pdf",
            format: "pdf",
            detect_method: DetectMethod::ContentSniff,
            size_bytes: 12345,
            mode: PipelineMode::Rigor,
            model: "dsv4",
            page_count: Some(42),
        };
        let out = render(&fm);
        assert!(out.starts_with("---\n"));
        assert!(out.ends_with("\n\n"));
        assert!(out.contains("source: doc.pdf"));
        assert!(out.contains("format: pdf"));
        assert!(out.contains("detect_method: content-sniff"));
        assert!(out.contains("size_bytes: 12345"));
        assert!(out.contains("mode: rigor"));
        assert!(out.contains("model: dsv4"));
        assert!(out.contains("page_count: 42"));
        assert!(out.contains("tool: qyzs-cli/"));
    }

    #[test]
    fn test_render_no_page_count() {
        let fm = FrontMatter {
            source: "data.csv",
            format: "csv",
            detect_method: DetectMethod::ExtensionFallback,
            size_bytes: 999,
            mode: PipelineMode::Speed,
            model: "dsv4",
            page_count: None,
        };
        let out = render(&fm);
        assert!(!out.contains("page_count"));
        assert!(out.contains("detect_method: extension-fallback"));
        assert!(out.contains("mode: speed"));
    }

    #[test]
    fn test_yaml_scalar_simple() {
        assert_eq!(yaml_scalar("hello"), "hello");
        assert_eq!(yaml_scalar("foo.pdf"), "foo.pdf");
        // 中文字符属于合法 unquoted scalar (>= 0x80 在允许集)
        assert_eq!(yaml_scalar("中文路径"), "中文路径");
        // URL 含 : 与 / 也是合法的
        assert_eq!(yaml_scalar("https://example.com/a.pdf"), "https://example.com/a.pdf");
    }

    #[test]
    fn test_yaml_scalar_quoted() {
        assert_eq!(yaml_scalar(""), "\"\"");
        assert_eq!(yaml_scalar("a b"), "\"a b\"");
        assert_eq!(yaml_scalar("a\"b"), "\"a\\\"b\"");
        assert_eq!(yaml_scalar("a\nb"), "\"a\\nb\"");
    }

    #[test]
    fn test_format_rfc3339_epoch_zero() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn test_format_rfc3339_known_date() {
        // 2026-10-19 00:00:00 UTC = epoch 1792368000 (实测验证)
        assert_eq!(format_rfc3339(1_792_368_000), "2026-10-19T00:00:00Z");
    }
}
