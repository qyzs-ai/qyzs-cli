//! OCR/VL 引擎健康熔断器 (进程级, 按引擎名隔离)
//!
//! 动机 (2026-10-26 批量实测): 网关侧引擎故障 (如 paddleocrvl base_url_missing)
//! 时单次调用固定 ~16.5s 超时, 重试不会好转。扫描件第二通道 10 页 × 4 路并发
//! = 3 波 × 16.5s ≈ 50s 纯等待; 引擎故障是进程外状态, 进程内继续排队等超时
//! 只是浪费。连续失败达阈值后短路该引擎 (直接走另一引擎), 熔断窗口到期后
//! 半开允许一次探测调用, 成功即恢复。正式批处理前的快速预检失败会直接熔断，
//! 避免为同一个已确认故障再累计三次超时。

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// 连续失败达到该次数后熔断
const TRIP_AFTER: u32 = 3;

/// 熔断持续时间 (到期后半开: 放行一次探测调用, 失败则重新熔断)
const OPEN_MILLIS: u64 = 10 * 60 * 1000;

/// 引擎名 (熔断注册表的 key; VL 通道实际模型由客户端按有图自动选路)
pub const PADDLE_OCR: &str = "paddleocrvl";
pub const VL: &str = "qwen3-vl";

#[derive(Default)]
struct Breaker {
    consecutive_failures: u32,
    open_until_ms: u64,
}

fn registry() -> &'static Mutex<HashMap<String, Breaker>> {
    static REG: OnceLock<Mutex<HashMap<String, Breaker>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

fn preflight_registry() -> &'static Mutex<HashSet<String>> {
    static PREFLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    PREFLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn with<R>(engine: &str, f: impl FnOnce(&mut Breaker) -> R) -> R {
    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    f(reg.entry(engine.to_string()).or_default())
}

/// 引擎当前是否允许调用 (熔断窗口内返回 false)
pub fn engine_available(engine: &str) -> bool {
    with(engine, |b| now_ms() >= b.open_until_ms)
}

/// 记录一次真实调用失败; 连续失败达阈值后熔断。
/// (熔断跳过不算失败, 否则会把窗口无限续期)
pub fn record_failure(engine: &str) {
    with(engine, |b| {
        b.consecutive_failures += 1;
        if b.consecutive_failures >= TRIP_AFTER {
            b.open_until_ms = now_ms() + OPEN_MILLIS;
        }
    });
}

/// 运行前快速探测失败时立即熔断，避免正式批处理再累计三次超时。
pub fn trip_now(engine: &str) {
    with(engine, |b| {
        b.consecutive_failures = TRIP_AFTER;
        b.open_until_ms = now_ms() + OPEN_MILLIS;
    });
}

/// 同一进程、同一通道的模型预检只执行一次。
pub fn claim_preflight(channel: &str, engine: &str) -> bool {
    let mut checked = preflight_registry()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    checked.insert(format!("{channel}\0{engine}"))
}

/// 记录一次真实调用成功: 清零计数并解除熔断 (半开探测通过)
pub fn record_success(engine: &str) {
    with(engine, |b| {
        b.consecutive_failures = 0;
        b.open_until_ms = 0;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试专用引擎名, 与真实引擎隔离 (注册表进程级共享, 避免污染其他测试)
    const TEST: &str = "test-engine";

    fn reset(engine: &str) {
        with(engine, |b| *b = Breaker::default());
    }

    #[test]
    fn test_trip_and_recover() {
        reset(TEST);
        assert!(engine_available(TEST));
        record_failure(TEST);
        record_failure(TEST);
        assert!(engine_available(TEST), "两次失败不熔断");
        record_failure(TEST);
        assert!(!engine_available(TEST), "连续三次失败应熔断");
        // 半开: 窗口到期后放行
        with(TEST, |b| b.open_until_ms = 1);
        assert!(engine_available(TEST), "窗口到期应半开放行");
        // 半开探测失败 → 重新熔断 (计数未清零)
        record_failure(TEST);
        assert!(!engine_available(TEST));
        // 探测成功 → 完全恢复
        with(TEST, |b| b.open_until_ms = 1);
        record_success(TEST);
        assert!(engine_available(TEST));
        record_failure(TEST);
        assert!(engine_available(TEST), "成功后计数清零, 单次失败不再熔断");
        reset(TEST);
    }

    #[test]
    fn test_preflight_failure_trips_immediately_and_is_claimed_once() {
        const PREFLIGHT_TEST: &str = "preflight-test-engine";
        reset(PREFLIGHT_TEST);
        assert!(claim_preflight("vision", PREFLIGHT_TEST));
        assert!(!claim_preflight("vision", PREFLIGHT_TEST));
        assert!(claim_preflight("chat", PREFLIGHT_TEST));
        trip_now(PREFLIGHT_TEST);
        assert!(!engine_available(PREFLIGHT_TEST));
        reset(PREFLIGHT_TEST);
    }
}
