//! 本地网关用量台账 (JSONL 追加 + 聚合统计)
//!
//! 每次成功网关调用由 client 层各接口方法追加一行记录；`qyzs usage`
//! 读取台账并聚合。记录为 best-effort: 任何写入失败静默忽略，绝不影响
//! 主命令执行。台账文件: `$XDG_STATE_HOME/qyzs/usage.jsonl`
//! (回退链与缓存目录约定一致: XDG > HOME > APPDATA > 当前目录)。
//!
//! 时间一律 UTC；无 chrono 依赖，epoch→日期用 civil_from_days 纯算法。
//!
//! 行两类:
//! - 明细行: 一次请求一行, `count`/`key` 缺省;
//! - 归档行: 历史压实产物 (见 compact_if_needed), 携带 `count` = 聚合请求数,
//!   ts 取日精度, 键为 (日, 模型, 调用族)。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

/// 单次网关调用用量记录 (JSONL 一行一条)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    /// RFC3339 UTC 时间戳 (秒级, 如 2026-10-18T01:16:56Z; 归档行为日精度)
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
    /// 归档行: 聚合的请求数 (存在即标记该行为归档行); 明细行缺省
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    /// 记录时生效的 API Key 掩码指纹 (Credentials::mask_key 同款, 脱敏不落明文)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
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
            count: None,
            key: None,
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

    /// 该行代表的请求数 (归档行 = count, 明细行 = 1)
    pub fn request_count(&self) -> u64 {
        self.count.unwrap_or(1)
    }

    /// 是否归档行
    pub fn is_rollup(&self) -> bool {
        self.count.is_some()
    }

    /// 该记录是否携带 Token 数据 (rerank/流式回退等场景可能缺失)
    pub fn has_tokens(&self) -> bool {
        self.total_tokens.is_some()
    }
}

/// 查询过滤条件 (days 按 UTC 日界; 其余为精确匹配)
#[derive(Debug, Default, Clone)]
pub struct UsageFilter<'a> {
    pub days: Option<u32>,
    pub model: Option<&'a str>,
    pub kind: Option<&'a str>,
    /// 凭据掩码指纹 (如 sk-***8816), 与台账内 key 字段精确匹配
    pub key: Option<&'a str>,
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

/// 过滤: days = 仅最近 N 天 (含今天, UTC 日界)；model/kind/key = 精确匹配
pub fn filter_records(records: &[UsageRecord], filter: &UsageFilter<'_>) -> Vec<UsageRecord> {
    let cutoff = filter.days.map(|d| utc_date_days_ago(d as u64));
    records
        .iter()
        .filter(|r| {
            cutoff.as_ref().map_or(true, |c| r.day() >= c.as_str())
                && filter.model.map_or(true, |m| r.model == m)
                && filter.kind.map_or(true, |k| r.kind == k)
                && filter.key.map_or(true, |k| r.key.as_deref() == Some(k))
        })
        .cloned()
        .collect()
}

/// 分组累计值 (按模型 / 按调用族共用)
#[derive(Debug, Default, Clone)]
pub struct GroupStat {
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
    pub by_model: BTreeMap<String, GroupStat>,
    pub by_kind: BTreeMap<String, GroupStat>,
    /// 按凭据掩码指纹分组 (无指纹的旧行不计入)
    pub by_key: BTreeMap<String, GroupStat>,
    pub by_day: BTreeMap<String, DayStat>,
    pub first_day: Option<String>,
    pub last_day: Option<String>,
}

pub fn summarize(records: &[UsageRecord]) -> UsageSummary {
    let mut s = UsageSummary::default();
    for r in records {
        let n = r.request_count();
        s.requests += n;
        let prompt = r.prompt_tokens.unwrap_or(0);
        let completion = r.completion_tokens.unwrap_or(0);
        let total = r.total_tokens.unwrap_or(0);
        s.prompt_tokens += prompt;
        s.completion_tokens += completion;
        s.total_tokens += total;
        if r.has_tokens() {
            // 归档行的 Token 数据是整组聚合值, 有则整组计入
            s.with_tokens += n;
        }

        let ms = s.by_model.entry(r.model.clone()).or_default();
        ms.requests += n;
        ms.prompt_tokens += prompt;
        ms.completion_tokens += completion;
        ms.total_tokens += total;
        if r.has_tokens() {
            ms.with_tokens += n;
        }
        let ks = s.by_kind.entry(r.kind.clone()).or_default();
        ks.requests += n;
        ks.prompt_tokens += prompt;
        ks.completion_tokens += completion;
        ks.total_tokens += total;
        if r.has_tokens() {
            ks.with_tokens += n;
        }
        if let Some(fp) = &r.key {
            let fs = s.by_key.entry(fp.clone()).or_default();
            fs.requests += n;
            fs.prompt_tokens += prompt;
            fs.completion_tokens += completion;
            fs.total_tokens += total;
            if r.has_tokens() {
                fs.with_tokens += n;
            }
        }

        if !r.day().is_empty() {
            let ds = s.by_day.entry(r.day().to_string()).or_default();
            ds.requests += n;
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

/// 按小时聚合 (UTC 小时桶, 如 2026-10-18T01)。
/// 归档行只有日精度时间戳, 无法归入小时桶, 单独以 archived 计数返回,
/// 供展示层标注 "另有归档历史"。
#[derive(Debug, Default)]
pub struct HourlySummary {
    pub by_hour: BTreeMap<String, DayStat>,
    pub archived_requests: u64,
    pub archived_tokens: u64,
}

pub fn summarize_hourly(records: &[UsageRecord]) -> HourlySummary {
    let mut h = HourlySummary::default();
    for r in records {
        if r.is_rollup() {
            h.archived_requests += r.request_count();
            h.archived_tokens += r.total_tokens.unwrap_or(0);
            continue;
        }
        let bucket = match r.ts.get(..13) {
            Some(b) if b.len() == 13 => b.to_string(),
            _ => continue,
        };
        let e = h.by_hour.entry(bucket).or_default();
        e.requests += 1;
        e.total_tokens += r.total_tokens.unwrap_or(0);
    }
    h
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
            count: None,
            key: None,
        }
    }

    fn rollup(ts: &str, kind: &str, model: &str, total: u64, count: u64) -> UsageRecord {
        let mut r = rec(ts, kind, model, Some(total));
        r.count = Some(count);
        r
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
    fn old_format_lines_deserialize_without_new_fields() {
        // 历史台账行无 count/key 字段: serde default 必须兼容
        let line = r#"{"ts":"2026-10-18T01:28:52Z","kind":"chat","model":"dsv4","prompt_tokens":9,"completion_tokens":2,"total_tokens":11}"#;
        let r: UsageRecord = serde_json::from_str(line).unwrap();
        assert_eq!(r.request_count(), 1);
        assert!(!r.is_rollup());
        assert_eq!(r.key, None);
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
    fn filter_by_days_model_kind_key() {
        let mut a = rec("2026-10-17T12:00:00Z", "chat", "dsv4", Some(200));
        a.key = Some("sk-abc***8816".to_string());
        let mut b = rec("2026-10-18T08:00:00Z", "embed", "bge-m3", Some(50));
        b.key = Some("sk-xyz***0001".to_string());
        let records = vec![
            rec("2026-10-01T00:00:00Z", "chat", "dsv4", Some(100)),
            a,
            b,
        ];

        assert_eq!(utc_date_days_ago(0), "2026-10-18");

        // --days 2 (2026-10-17 起)
        let got = filter_records(
            &records,
            &UsageFilter { days: Some(2), ..Default::default() },
        );
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|r| r.ts.as_str() >= "2026-10-17"));

        // --model / --kind / --key
        assert_eq!(filter_records(&records, &UsageFilter { model: Some("bge-m3"), ..Default::default() }).len(), 1);
        assert_eq!(filter_records(&records, &UsageFilter { kind: Some("chat"), ..Default::default() }).len(), 2);
        let got = filter_records(&records, &UsageFilter { key: Some("sk-xyz***0001"), ..Default::default() });
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, "embed");

        // 组合过滤
        assert_eq!(filter_records(&records, &UsageFilter { days: Some(2), model: Some("dsv4"), ..Default::default() }).len(), 1);
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
        assert_eq!(s.by_kind.len(), 3);
        assert_eq!(s.by_kind["chat"].total_tokens, 160);
        assert_eq!(s.by_day.len(), 3);
        assert_eq!(s.by_day["2026-10-18"].requests, 2);
        assert_eq!(s.first_day.as_deref(), Some("2026-10-16"));
        assert_eq!(s.last_day.as_deref(), Some("2026-10-18"));
    }

    #[test]
    fn summarize_counts_rollup_lines_by_count_field() {
        let records = vec![
            rec("2026-10-18T10:00:00Z", "chat", "dsv4", Some(60)),
            rollup("2026-10-01T00:00:00Z", "chat", "dsv4", 9_000, 57),
        ];
        let s = summarize(&records);
        assert_eq!(s.requests, 58);
        assert_eq!(s.with_tokens, 58);
        assert_eq!(s.total_tokens, 9_060);
        assert_eq!(s.by_model["dsv4"].requests, 58);
        assert_eq!(s.by_kind["chat"].requests, 58);
        assert_eq!(s.by_day["2026-10-01"].requests, 57);
    }

    #[test]
    fn hourly_buckets_details_and_archives_rollups() {
        let records = vec![
            rec("2026-10-18T01:16:56Z", "chat", "dsv4", Some(11)),
            rec("2026-10-18T01:30:00Z", "chat", "dsv4", Some(22)),
            rec("2026-10-18T09:00:00Z", "ocr", "paddleocrvl", Some(768)),
            rollup("2026-10-01T00:00:00Z", "chat", "dsv4", 9_000, 57),
        ];
        let h = summarize_hourly(&records);
        assert_eq!(h.by_hour.len(), 2);
        assert_eq!(h.by_hour["2026-10-18T01"].requests, 2);
        assert_eq!(h.by_hour["2026-10-18T01"].total_tokens, 33);
        assert_eq!(h.by_hour["2026-10-18T09"].total_tokens, 768);
        assert_eq!(h.archived_requests, 57);
        assert_eq!(h.archived_tokens, 9_000);
    }

    #[test]
    fn summarize_empty_is_default() {
        let s = summarize(&[]);
        assert_eq!(s.requests, 0);
        assert_eq!(s.first_day, None);
        assert!(s.by_model.is_empty());
    }
}
