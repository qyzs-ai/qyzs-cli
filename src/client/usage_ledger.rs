//! 本地网关用量台账 (JSONL 追加 + 聚合统计)
//!
//! 每次成功网关调用由 client 层各接口方法追加一行记录；`qyzs usage`
//! 读取台账并聚合。记录为 best-effort: 任何写入失败静默忽略，绝不影响
//! 主命令执行。台账文件: `$XDG_STATE_HOME/qyzs/usage.jsonl`
//! (回退链与缓存目录约定一致: XDG > HOME > APPDATA > 当前目录)。
//!
//! 时间一律 UTC；无 chrono 依赖，epoch→日期用 civil_from_days 纯算法。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

/// 单次网关调用用量记录 (JSONL 一行一条)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    /// RFC3339 UTC 时间戳 (秒级, 如 2026-10-18T01:16:56Z)
    pub ts: String,
    /// 调用族: chat | ocr | asr | embed | rerank (任务型接口 mineru/sam 无 Token 数据, 不记录)
    pub kind: String,
    pub model: String,
    /// 流式调用若网关未回 usage 则为 None (实测已支持 include_usage, 正常路径有值)
    #[serde(default)]
    pub prompt_tokens: Option<u64>,
    #[serde(default)]
    pub completion_tokens: Option<u64>,
    #[serde(default)]
    pub total_tokens: Option<u64>,
}

impl UsageRecord {
    pub fn now(
        kind: &str,
        model: &str,
        prompt_tokens: Option<u64>,
        completion_tokens: Option<u64>,
        total_tokens: Option<u64>,
    ) -> Self {
        Self {
            ts: now_rfc3339(),
            kind: kind.to_string(),
            model: model.to_string(),
            prompt_tokens,
            completion_tokens,
            total_tokens,
        }
    }

    /// 从 chat/completions 系响应 (chat/ocr/asr) 的 usage 构造
    pub fn from_chat_usage(kind: &str, model: &str, usage: Option<&super::chat::ChatUsage>) -> Self {
        let u = usage;
        Self::now(
            kind,
            model,
            u.and_then(|u| u.prompt_tokens.map(|v| v as u64)),
            u.and_then(|u| u.completion_tokens.map(|v| v as u64)),
            u.and_then(|u| u.total_tokens.map(|v| v as u64)),
        )
    }

    /// 记录日期 (UTC, 取 ts 前 10 字符；异常行回退空串)
    pub fn day(&self) -> &str {
        self.ts.get(..10).unwrap_or("")
    }

    /// 该记录是否携带 Token 数据 (rerank/流式回退等场景可能缺失)
    pub fn has_tokens(&self) -> bool {
        self.total_tokens.is_some()
    }
}

/// Unix epoch 秒 → UTC (年, 月, 日, 时, 分, 秒)。civil_from_days 算法 (H. Hinnant)。
pub fn epoch_to_utc(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as u32;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };

    (y, m, d, rem / 3_600, (rem % 3_600) / 60, rem % 60)
}

/// epoch 秒 → RFC3339 UTC 字符串 (2026-10-18T01:16:56Z)
pub fn format_rfc3339(secs: u64) -> String {
    let (y, m, d, h, mi, s) = epoch_to_utc(secs);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, h, mi, s)
}

pub fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn now_rfc3339() -> String {
    format_rfc3339(now_epoch())
}

/// 今天 UTC 日期往前推 n 天的日期字符串 (YYYY-MM-DD)，供 --days 过滤取下界
pub fn utc_date_days_ago(n: u64) -> String {
    let day_start = (now_epoch() / 86_400).saturating_sub(n) * 86_400;
    let (y, m, d, _, _, _) = epoch_to_utc(day_start);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// 千分位分隔 (1234567 → "1,234,567")
pub fn format_num(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 台账文件绝对路径
pub fn ledger_path() -> PathBuf {
    state_base().join("usage.jsonl")
}

/// 状态基目录 (与缓存目录同一平台约定: XDG > HOME > APPDATA > 当前目录)
fn state_base() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("qyzs")
}

/// 追加一条记录 (best-effort: 建目录/写文件失败均静默忽略)
pub fn append_record(rec: &UsageRecord) {
    append_to(&ledger_path(), rec);
}

fn append_to(path: &std::path::Path, rec: &UsageRecord) {
    let Ok(mut line) = serde_json::to_string(rec) else {
        return;
    };
    line.push('\n');
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// 读取全部记录 (容错: 文件不存在返回空表, 损坏行跳过)
pub fn read_records() -> Vec<UsageRecord> {
    read_from(&ledger_path())
}

fn read_from(path: &std::path::Path) -> Vec<UsageRecord> {
    let Ok(f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

/// 清空台账, 返回删除的记录数
pub fn clear() -> usize {
    let n = read_records().len();
    let _ = std::fs::remove_file(ledger_path());
    n
}

/// 过滤: days = 仅最近 N 天 (含今天, UTC 日界)；model = 精确匹配模型名
pub fn filter_records(
    records: &[UsageRecord],
    days: Option<u32>,
    model: Option<&str>,
) -> Vec<UsageRecord> {
    let cutoff = days.map(|d| utc_date_days_ago(d as u64));
    records
        .iter()
        .filter(|r| {
            cutoff.as_ref().map_or(true, |c| r.day() >= c.as_str())
                && model.map_or(true, |m| r.model == m)
        })
        .cloned()
        .collect()
}

/// 按模型分组的累计值
#[derive(Debug, Default, Clone)]
pub struct ModelStat {
    pub requests: u64,
    pub with_tokens: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// 按日分组的累计值
#[derive(Debug, Default, Clone)]
pub struct DayStat {
    pub requests: u64,
    pub total_tokens: u64,
}

/// 聚合结果 (BTreeMap 保证输出稳定有序)
#[derive(Debug, Default)]
pub struct UsageSummary {
    pub requests: u64,
    pub with_tokens: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub by_model: BTreeMap<String, ModelStat>,
    pub by_day: BTreeMap<String, DayStat>,
    pub first_day: Option<String>,
    pub last_day: Option<String>,
}

pub fn summarize(records: &[UsageRecord]) -> UsageSummary {
    let mut s = UsageSummary::default();
    for r in records {
        s.requests += 1;
        let prompt = r.prompt_tokens.unwrap_or(0);
        let completion = r.completion_tokens.unwrap_or(0);
        let total = r.total_tokens.unwrap_or(0);
        s.prompt_tokens += prompt;
        s.completion_tokens += completion;
        s.total_tokens += total;
        if r.has_tokens() {
            s.with_tokens += 1;
        }

        let ms = s.by_model.entry(r.model.clone()).or_default();
        ms.requests += 1;
        ms.prompt_tokens += prompt;
        ms.completion_tokens += completion;
        ms.total_tokens += total;
        if r.has_tokens() {
            ms.with_tokens += 1;
        }

        if !r.day().is_empty() {
            let ds = s.by_day.entry(r.day().to_string()).or_default();
            ds.requests += 1;
            ds.total_tokens += total;
            if s.first_day.as_ref().map_or(true, |d| r.day() < d.as_str()) {
                s.first_day = Some(r.day().to_string());
            }
            if s.last_day.as_ref().map_or(true, |d| r.day() > d.as_str()) {
                s.last_day = Some(r.day().to_string());
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(ts: &str, kind: &str, model: &str, total: Option<u64>) -> UsageRecord {
        UsageRecord {
            ts: ts.to_string(),
            kind: kind.to_string(),
            model: model.to_string(),
            prompt_tokens: total.map(|t| t / 2),
            completion_tokens: total.map(|t| t - t / 2),
            total_tokens: total,
        }
    }

    #[test]
    fn epoch_conversion_known_timestamps() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        // 2000-02-29: 世纪闰年 (400 倍数) 边界
        assert_eq!(format_rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_rfc3339(1_792_286_216), "2026-10-18T01:16:56Z");
        assert_eq!(format_rfc3339(1_792_286_216 + 86_400), "2026-10-19T01:16:56Z");
    }

    #[test]
    fn utc_date_days_ago_is_wellformed() {
        let today = utc_date_days_ago(0);
        assert_eq!(today.len(), 10);
        assert_eq!(today.as_bytes()[4], b'-');
        let earlier = utc_date_days_ago(7);
        assert!(earlier < today);
        assert_eq!(utc_date_days_ago(7).len(), 10);
    }

    #[test]
    fn format_num_inserts_thousands_separators() {
        assert_eq!(format_num(0), "0");
        assert_eq!(format_num(999), "999");
        assert_eq!(format_num(1_000), "1,000");
        assert_eq!(format_num(1_234_567), "1,234,567");
        assert_eq!(format_num(152_346_789), "152,346,789");
    }

    #[test]
    fn append_read_roundtrip_and_tolerates_broken_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.jsonl");
        append_to(&path, &rec("2026-10-18T01:16:56Z", "chat", "dsv4", Some(168)));
        append_to(&path, &rec("2026-10-18T02:00:00Z", "embed", "bge-m3", None));
        // 模拟历史损坏行
        std::fs::write(
            &path,
            std::fs::read_to_string(&path).unwrap() + "not-json\n{\"ts\":",
        )
        .unwrap();

        let records = read_from(&path);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].kind, "chat");
        assert!(records[1].total_tokens.is_none());
        assert!(!records[1].has_tokens());
    }

    #[test]
    fn filter_by_days_and_model() {
        let records = vec![
            rec("2026-10-01T00:00:00Z", "chat", "dsv4", Some(100)),
            rec("2026-10-17T12:00:00Z", "chat", "dsv4", Some(200)),
            rec("2026-10-18T08:00:00Z", "embed", "bge-m3", Some(50)),
        ];
        let today = utc_date_days_ago(0);
        assert_eq!(today, "2026-10-18");

        // --days 2 (2026-10-17 起)
        let got = filter_records(&records, Some(2), None);
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|r| r.ts.as_str() >= "2026-10-17"));

        // --model bge-m3
        let got = filter_records(&records, None, Some("bge-m3"));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, "embed");

        // 组合过滤
        assert!(filter_records(&records, Some(2), Some("bge-m3")).len() == 1);
        assert!(filter_records(&records, Some(2), Some("dsv4")).len() == 1);
    }

    #[test]
    fn summarize_aggregates_and_tracks_range() {
        let records = vec![
            rec("2026-10-16T10:00:00Z", "chat", "dsv4", Some(100)),
            rec("2026-10-18T10:00:00Z", "chat", "dsv4", Some(60)),
            rec("2026-10-17T10:00:00Z", "rerank", "bge-reranker-v2-m3", Some(46)),
            rec("2026-10-18T11:00:00Z", "embed", "bge-m3", None),
        ];
        let s = summarize(&records);
        assert_eq!(s.requests, 4);
        assert_eq!(s.with_tokens, 3);
        assert_eq!(s.total_tokens, 206);
        assert_eq!(s.by_model.len(), 3);
        assert_eq!(s.by_model["dsv4"].requests, 2);
        assert_eq!(s.by_model["dsv4"].total_tokens, 160);
        assert_eq!(s.by_day.len(), 3);
        assert_eq!(s.by_day["2026-10-18"].requests, 2);
        assert_eq!(s.first_day.as_deref(), Some("2026-10-16"));
        assert_eq!(s.last_day.as_deref(), Some("2026-10-18"));
    }

    #[test]
    fn summarize_empty_is_default() {
        let s = summarize(&[]);
        assert_eq!(s.requests, 0);
        assert_eq!(s.first_day, None);
        assert!(s.by_model.is_empty());
    }
}
