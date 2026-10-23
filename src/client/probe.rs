//! 网关健康度与多专线网络延时探活 (status 核心依赖)

use super::QyzsClient;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub name: String,
    pub endpoint: String,
    pub ok: bool,
    pub status_code: u16,
    pub latency_ms: u64,
    pub error_msg: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GatewayProbes {
    pub base_url: String,
    pub public_api: ProbeResult,
    pub mineru_api: ProbeResult,
    pub sam_api: ProbeResult,
    pub all_ok: bool,
}

impl QyzsClient {
    /// 执行三网关探活探测 (并发执行，探活超时限制为 5 秒，快速反馈)
    pub async fn run_probes(&self) -> GatewayProbes {
        let key = self.config.api_key.as_str();

        // 并发执行三个端点的探活
        let (p1, p2, p3) = tokio::join!(
            self.probe_endpoint(
                "公共推理网关 (Public API /v1)",
                "api/aiCenterChat/v1/chat/completions",
                key
            ),
            self.probe_endpoint("MinerU 结构化专线", MINERU_PROBE_PATH, key),
            self.probe_endpoint("SAM 遥感流水线专线", "api/aiCenterChat/v3/form", key)
        );

        let all_ok = p1.ok && p2.ok && p3.ok;

        GatewayProbes {
            base_url: self.config.base_url.clone(),
            public_api: p1,
            mineru_api: p2,
            sam_api: p3,
            all_ok,
        }
    }

    async fn probe_endpoint(&self, name: &str, subpath: &str, key: &str) -> ProbeResult {
        let start = Instant::now();
        // 探针专用快速超时客户端 (5 秒) + 禁止跟随重定向
        //
        // 网关对「模型目录」路径会 301 到不带端口的绝对地址, 跟随后连到不可达
        // 端口而挂到超时, 令正常工作的专线被误判为不可用 (实测: MinerU 探针报
        // ok:false / status 0, 而该专线解析实际全部成功)。3xx 即证明路由可达。
        let probe_http = match super::HttpClient::new_no_redirect(self.http.base_url(), 5) {
            Ok(c) => c,
            Err(e) => {
                return ProbeResult {
                    name: name.to_string(),
                    endpoint: subpath.to_string(),
                    ok: false,
                    status_code: 0,
                    latency_ms: start.elapsed().as_millis() as u64,
                    error_msg: Some(e.to_string()),
                }
            }
        };
        let req = probe_http.get(subpath, Some(key));
        let res = req.send().await;
        let latency_ms = start.elapsed().as_millis() as u64;

        match res {
            Ok(resp) => {
                let status = resp.status().as_u16();
                // 即使由于没有 payload 返回 400/405, 或 3xx 重定向, 也说明
                // 网络与网关路由可达且在线
                let ok = status < 500;
                ProbeResult {
                    name: name.to_string(),
                    endpoint: subpath.to_string(),
                    ok,
                    status_code: status,
                    latency_ms,
                    error_msg: if ok {
                        None
                    } else {
                        Some(format!("HTTP 状态码: {}", status))
                    },
                }
            }
            Err(e) => ProbeResult {
                name: name.to_string(),
                endpoint: subpath.to_string(),
                ok: false,
                status_code: 0,
                latency_ms,
                error_msg: Some(e.to_string()),
            },
        }
    }
}

/// MinerU 专线探活路径
///
/// 取实际解析端点而非模型目录: 目录路径会 301 到不带端口的地址, 跟随后超时。
/// 该端点用 GET 探测会返回 405 (方法不允许), 证明路由可达 —— 与另外两条
/// 专线 (405 / 404) 的判定口径一致。
const MINERU_PROBE_PATH: &str = "ai-gateway/mineru3.2.3/file_parse";
