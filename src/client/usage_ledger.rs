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
    pub fn from_chat_usage(
        kind: &str,
        model: &str,
        usage: Option<&super::chat::ChatUsage>,
    ) -> Self {
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
    /// 凭据掩码指纹 (如 sk-***8816), 与台账内 key 字段匹配 (两端空白自动忽略)
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
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
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
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
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

/// 明细行保留天数: 超期明细在 usage 读取路径被时机性压实为归档行
pub const DETAIL_RETENTION_DAYS: u32 = 30;

/// 锁文件认定僵尸的时长 (持有进程崩溃后自动解锁)
const LOCK_STALE_SECS: u64 = 60;

/// 时机性压实 (best-effort): 将超过保留期的明细行折叠为
/// (日 × 模型 × 调用族 × 凭据) 归档行, 原有归档行同组合并。
/// 锁被占用即跳过 (下次 usage 再试); 重写失败静默放弃。
pub fn compact_if_needed() -> bool {
    compact_at(&ledger_path(), DETAIL_RETENTION_DAYS)
}

fn compact_at(path: &std::path::Path, retain_days: u32) -> bool {
    compact_stale(
        path,
        retain_days,
        std::time::Duration::from_secs(LOCK_STALE_SECS),
    )
}

fn compact_stale(
    path: &std::path::Path,
    retain_days: u32,
    stale_after: std::time::Duration,
) -> bool {
    let cutoff = utc_date_days_ago(retain_days as u64);
    let needs = read_from(path)
        .iter()
        .any(|r| !r.is_rollup() && r.day().len() == 10 && r.day() < cutoff.as_str());
    if !needs {
        return false;
    }

    // 锁内重读以缩小与并发追加者的竞态窗口。追加路径不加锁 (记录是
    // best-effort 单行 append), 毫秒级窗口内落入的追加行可能丢失, 接受。
    let Some(_lock) = LedgerLock::acquire_stale(lock_path_for(path), stale_after) else {
        return false;
    };
    let records = read_from(path);
    let cutoff = utc_date_days_ago(retain_days as u64);

    // 保留: 保留期内明细 + 时间戳异常无法归日的行 (宁留不删)
    let mut kept: Vec<UsageRecord> = Vec::new();
    // 折叠池: 超期明细 + 既有归档行, 按 (日, 模型, 调用族, 凭据) 合并
    let mut pool: std::collections::BTreeMap<
        (String, String, String, Option<String>),
        UsageRecord,
    > = std::collections::BTreeMap::new();

    for r in records {
        let archivable = r.day().len() == 10 && r.day() < cutoff.as_str();
        if !r.is_rollup() && !archivable {
            kept.push(r);
            continue;
        }
        let entry = pool
            .entry((
                r.day().to_string(),
                r.model.clone(),
                r.kind.clone(),
                r.key.clone(),
            ))
            .or_insert_with(|| {
                let mut rollup = UsageRecord::now(&r.kind, &r.model, None, None, None);
                rollup.ts = format!("{}T00:00:00Z", r.day());
                rollup.count = Some(0);
                rollup.key = r.key.clone();
                rollup
            });
        entry.count = Some(entry.count.unwrap_or(0) + r.request_count());
        entry.prompt_tokens = sum_opt(entry.prompt_tokens, r.prompt_tokens);
        entry.completion_tokens = sum_opt(entry.completion_tokens, r.completion_tokens);
        entry.total_tokens = sum_opt(entry.total_tokens, r.total_tokens);
    }

    let mut out: Vec<UsageRecord> = pool.into_values().collect();
    out.extend(kept);
    out.sort_by(|a, b| a.ts.cmp(&b.ts));

    let mut body = String::new();
    for r in &out {
        if let Ok(line) = serde_json::to_string(r) {
            body.push_str(&line);
            body.push('\n');
        }
    }

    // 同目录临时文件 + rename 原子替换; 失败静默放弃 (下次再压实)
    let tmp = path.with_file_name("usage.jsonl.tmp");
    if std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, path).is_ok() {
        true
    } else {
        let _ = std::fs::remove_file(&tmp);
        false
    }
}

/// Option<u64> 聚合: 任一有值则求和, 全无则 None
fn sum_opt(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (None, None) => None,
        (x, y) => Some(x.unwrap_or(0) + y.unwrap_or(0)),
    }
}

fn lock_path_for(path: &std::path::Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.lock",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("usage.jsonl")
    ))
}

/// 压实互斥锁: create_new 抢占; 既有锁超过 stale_after 视为僵尸删除重试一次,
/// 仍占用则放弃本次压实。Drop 时释放。
struct LedgerLock {
    path: PathBuf,
}

impl LedgerLock {
    fn acquire_stale(path: PathBuf, stale_after: std::time::Duration) -> Option<Self> {
        for _ in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    let _ = writeln!(f, "pid={} ts={}", std::process::id(), now_rfc3339());
                    return Some(Self { path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_is_stale(&path, stale_after) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    return None;
                }
                Err(_) => return None,
            }
        }
        None
    }
}

impl Drop for LedgerLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn lock_is_stale(path: &std::path::Path, stale_after: std::time::Duration) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return true;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    modified
        .elapsed()
        .map(|age| age >= stale_after)
        .unwrap_or(true)
}

/// 过滤: days = 仅最近 N 天 (含今天, UTC 日界)；model/kind/key = 精确匹配
pub fn filter_records(records: &[UsageRecord], filter: &UsageFilter<'_>) -> Vec<UsageRecord> {
    let cutoff = filter.days.map(|d| utc_date_days_ago(d as u64));
    // 过滤值统一 trim: 用户常直接从表格复制 --key/--model (掩码列按宽度左对齐带尾随空格),
    // 全等比对会静默匹配不到并返回空结果 (退出码仍为 0), 比报错更难排查
    let model = filter.model.map(str::trim);
    let kind = filter.kind.map(str::trim);
    let key = filter.key.map(str::trim);
    records
        .iter()
        .filter(|r| {
            cutoff.as_ref().is_none_or(|c| r.day() >= c.as_str())
                && model.is_none_or(|m| r.model == m)
                && kind.is_none_or(|k| r.kind == k)
                && key.is_none_or(|k| r.key.as_deref() == Some(k))
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
            if s.first_day.as_ref().is_none_or(|d| r.day() < d.as_str()) {
                s.first_day = Some(r.day().to_string());
            }
            if s.last_day.as_ref().is_none_or(|d| r.day() > d.as_str()) {
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

    /// 相对「今天」造 UTC 时间戳 (`ts_of(1, 12)` = 昨天 12:00Z), 使 --days 窗口断言
    /// 不随日历漂移
    fn ts_of(days_ago: u64, hour: u32) -> String {
        format!("{}T{:02}:00:00Z", utc_date_days_ago(days_ago), hour)
    }

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
        assert_eq!(
            format_rfc3339(1_792_286_216 + 86_400),
            "2026-10-19T01:16:56Z"
        );
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
        append_to(
            &path,
            &rec("2026-10-18T01:16:56Z", "chat", "dsv4", Some(168)),
        );
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
        // 时间戳相对「今天」生成: 此前硬编码 2026-10-18, 过了该日 --days 断言必然失败
        let mut a = rec(&ts_of(1, 12), "chat", "dsv4", Some(200));
        a.key = Some("sk-abc***8816".to_string());
        let mut b = rec(&ts_of(0, 8), "embed", "bge-m3", Some(50));
        b.key = Some("sk-xyz***0001".to_string());
        let records = vec![rec(&ts_of(30, 0), "chat", "dsv4", Some(100)), a, b];

        // --days 2 (前天起)
        let got = filter_records(
            &records,
            &UsageFilter {
                days: Some(2),
                ..Default::default()
            },
        );
        assert_eq!(got.len(), 2);
        let cutoff = utc_date_days_ago(2);
        assert!(got.iter().all(|r| r.day() >= cutoff.as_str()));

        // --model / --kind / --key
        assert_eq!(
            filter_records(
                &records,
                &UsageFilter {
                    model: Some("bge-m3"),
                    ..Default::default()
                }
            )
            .len(),
            1
        );
        assert_eq!(
            filter_records(
                &records,
                &UsageFilter {
                    kind: Some("chat"),
                    ..Default::default()
                }
            )
            .len(),
            2
        );
        let got = filter_records(
            &records,
            &UsageFilter {
                key: Some("sk-xyz***0001"),
                ..Default::default()
            },
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, "embed");

        // 组合过滤
        assert_eq!(
            filter_records(
                &records,
                &UsageFilter {
                    days: Some(2),
                    model: Some("dsv4"),
                    ..Default::default()
                }
            )
            .len(),
            1
        );
    }

    #[test]
    fn filter_values_tolerate_surrounding_whitespace() {
        let mut r = rec(&ts_of(0, 8), "chat", "dsv4", Some(200));
        r.key = Some("sk-abc***8816".to_string());
        let records = vec![r];

        // 用户从 [按凭据] 表格列复制掩码时会带上左对齐填充的尾随空格;
        // trim 前静默返回空结果 (退出码仍 0), 比报错更难排查
        for raw in [
            "sk-abc***8816",
            "sk-abc***8816 ",
            "sk-abc***8816   ",
            "  sk-abc***8816",
            " sk-abc***8816 ",
            "\tsk-abc***8816\n",
        ] {
            assert_eq!(
                filter_records(
                    &records,
                    &UsageFilter {
                        key: Some(raw),
                        ..Default::default()
                    }
                )
                .len(),
                1,
                "--key {:?} 应匹配 (空白被忽略)",
                raw
            );
        }
        // 前后空白对 --model / --kind 同样容忍
        assert_eq!(
            filter_records(
                &records,
                &UsageFilter {
                    model: Some("dsv4 "),
                    ..Default::default()
                }
            )
            .len(),
            1
        );
        assert_eq!(
            filter_records(
                &records,
                &UsageFilter {
                    kind: Some(" chat"),
                    ..Default::default()
                }
            )
            .len(),
            1
        );
        // trim 不放宽真实差异: 掩码本身不同仍匹配不到
        assert_eq!(
            filter_records(
                &records,
                &UsageFilter {
                    key: Some("sk-xyz***0001"),
                    ..Default::default()
                }
            )
            .len(),
            0
        );
    }

    #[test]
    fn summarize_aggregates_and_tracks_range() {
        let records = vec![
            rec("2026-10-16T10:00:00Z", "chat", "dsv4", Some(100)),
            rec("2026-10-18T10:00:00Z", "chat", "dsv4", Some(60)),
            rec(
                "2026-10-17T10:00:00Z",
                "rerank",
                "bge-reranker-v2-m3",
                Some(46),
            ),
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

    // ── 压实 ──

    fn write_lines(path: &std::path::Path, lines: &[UsageRecord]) {
        let body: String = lines
            .iter()
            .map(|r| serde_json::to_string(r).unwrap())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn compaction_folds_expired_details_merges_existing_rollups() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.jsonl");
        // 超期明细 3 行 (同日同模型同族同凭据) + 既有归档行 (同组合并) + 保留期明细 2 行
        write_lines(
            &path,
            &[
                rec("2026-09-01T01:00:00Z", "chat", "dsv4", Some(10)),
                rec("2026-09-01T02:00:00Z", "chat", "dsv4", Some(20)),
                rollup("2026-09-01T00:00:00Z", "chat", "dsv4", 9_000, 57),
                rec("2026-09-02T03:00:00Z", "ocr", "paddleocrvl", Some(768)),
                rec("2026-10-18T10:00:00Z", "chat", "dsv4", Some(60)),
                rec("2026-10-17T10:00:00Z", "embed", "bge-m3", Some(6)),
            ],
        );

        assert!(compact_at(&path, 30));
        let out = read_from(&path);
        // 期望: 2 条归档行 (09-01 chat 合并 59+57=116 次请求, 09-02 ocr 1 次) + 2 条保留明细
        assert_eq!(out.len(), 4);
        let chat_arch = out
            .iter()
            .find(|r| r.is_rollup() && r.kind == "chat")
            .expect("chat 归档行应存在");
        // 既有归档 57 次 + 超期明细 2 次
        assert_eq!(chat_arch.count, Some(59));
        assert_eq!(chat_arch.total_tokens, Some(9_030));
        assert_eq!(chat_arch.ts, "2026-09-01T00:00:00Z");
        let ocr_arch = out
            .iter()
            .find(|r| r.is_rollup() && r.kind == "ocr")
            .expect("ocr 归档行应存在");
        assert_eq!(ocr_arch.count, Some(1));
        assert!(out.iter().any(|r| r.ts == "2026-10-18T10:00:00Z"));
        assert!(out.iter().any(|r| r.ts == "2026-10-17T10:00:00Z"));

        // 幂等: 二次压实无变化
        assert!(!compact_at(&path, 30));
        assert_eq!(read_from(&path).len(), 4);
    }

    #[test]
    fn compaction_groups_by_kind_and_key_separately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.jsonl");
        let mut keyed = rec("2026-09-01T01:00:00Z", "chat", "dsv4", Some(10));
        keyed.key = Some("sk-a***1111".to_string());
        let mut other_key = rec("2026-09-01T02:00:00Z", "chat", "dsv4", Some(20));
        other_key.key = Some("sk-b***2222".to_string());
        let plain = rec("2026-09-01T03:00:00Z", "embed", "bge-m3", Some(5));
        write_lines(&path, &[keyed, other_key, plain]);

        assert!(compact_at(&path, 30));
        let out = read_from(&path);
        assert_eq!(out.len(), 3, "不同 (调用族, 凭据) 各自成行");
        let chat_a = out
            .iter()
            .find(|r| r.key.as_deref() == Some("sk-a***1111"))
            .unwrap();
        assert_eq!(chat_a.count, Some(1));
        let chat_b = out
            .iter()
            .find(|r| r.key.as_deref() == Some("sk-b***2222"))
            .unwrap();
        assert_eq!(chat_b.count, Some(1));
        assert!(out.iter().any(|r| r.kind == "embed" && r.key.is_none()));
    }

    #[test]
    fn compaction_preserves_malformed_timestamp_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.jsonl");
        let mut broken = rec("garbage", "chat", "dsv4", Some(1));
        broken.count = None;
        write_lines(
            &path,
            &[
                rec("2026-01-01T00:00:00Z", "chat", "dsv4", Some(10)),
                broken,
            ],
        );
        assert!(compact_at(&path, 30));
        let out = read_from(&path);
        assert_eq!(out.len(), 2, "异常时间戳行宁留不删");
        assert!(out.iter().any(|r| r.ts == "garbage"));
    }

    #[test]
    fn compaction_skips_when_lock_held_proceeds_when_stale() {
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.jsonl");
        write_lines(
            &path,
            &[rec("2026-01-01T00:00:00Z", "chat", "dsv4", Some(10))],
        );
        let lock_path = lock_path_for(&path);

        // 活锁占用: 跳过压实
        let _held = LedgerLock::acquire_stale(lock_path.clone(), Duration::from_secs(60));
        assert!(std::fs::metadata(&lock_path).is_ok());
        assert!(!compact_stale(&path, 30, Duration::from_secs(60)));
        assert_eq!(read_from(&path).len(), 1);
        drop(_held);
        assert!(!lock_path.exists(), "Drop 应释放锁");

        // 僵尸锁 (stale_after=0 视为必过期): 删除并完成压实
        std::fs::write(&lock_path, "pid=999999\n").unwrap();
        assert!(compact_stale(&path, 30, Duration::ZERO));
        let out = read_from(&path);
        assert_eq!(out.len(), 1);
        assert!(out[0].is_rollup());
        assert!(!lock_path.exists(), "压实完成后锁应释放");
    }

    #[test]
    fn compaction_noop_without_expired_details() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("usage.jsonl");
        write_lines(
            &path,
            &[
                rec("2026-10-18T10:00:00Z", "chat", "dsv4", Some(10)),
                rollup("2026-09-01T00:00:00Z", "chat", "dsv4", 9_000, 57),
            ],
        );
        let before = std::fs::read_to_string(&path).unwrap();
        assert!(!compact_at(&path, 30), "无超期明细不应触发重写");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }
}
