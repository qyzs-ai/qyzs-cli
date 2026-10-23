//! GitHub Release API 交互与资产匹配

use crate::error::{QyzsError, QyzsResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub html_url: String,
    pub published_at: Option<String>,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

pub struct GitHubClient;

impl GitHubClient {
    /// 获取 qyzs-ai/qyzs-cli 仓库最新 Release
    ///
    /// 接口地址可用环境变量 `QYZS_RELEASE_API_URL` 覆盖 (默认 GitHub Release API),
    /// 便于走镜像源或本地离线验证发布链路。
    pub async fn get_latest_release(repo: &str) -> QyzsResult<GitHubRelease> {
        let url = match std::env::var("QYZS_RELEASE_API_URL") {
            Ok(u) if !u.trim().is_empty() => u,
            _ => format!("https://api.github.com/repos/{}/releases/latest", repo),
        };
        let client = reqwest::Client::builder()
            .user_agent("qyzs-cli-updater")
            .build()
            .map_err(|e| QyzsError::Network(format!("创建 GitHub 客户端失败: {}", e)))?;

        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| QyzsError::Network(format!("连接 GitHub Release API 失败: {}", e)))?;

        if !resp.status().is_success() {
            return Err(QyzsError::Server(format!(
                "GitHub API 返回 HTTP {}",
                resp.status()
            )));
        }

        let release: GitHubRelease = resp
            .json()
            .await
            .map_err(|e| QyzsError::Server(format!("解析 GitHub Release 失败: {}", e)))?;

        Ok(release)
    }

    /// 根据当前操作系统与 CPU 架构匹配资产关键字
    pub fn current_platform_target() -> (&'static str, &'static str) {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        // 必须与 CI 构建矩阵一致（.github/workflows/ci.yml matrix）；
        // 对不上时资产名不存在，`qyzs upgrade` 会 404
        let target = match (os, arch) {
            ("linux", "x86_64") => "x86_64-unknown-linux-musl",
            ("linux", "aarch64") => "aarch64-unknown-linux-musl",
            ("linux", "loongarch64") => "loongarch64-unknown-linux-gnu",
            ("macos", "x86_64") => "x86_64-apple-darwin",
            ("macos", "aarch64") => "aarch64-apple-darwin",
            ("windows", "x86_64") => "x86_64-pc-windows-msvc",
            _ => "unknown",
        };

        let ext = if os == "windows" { ".zip" } else { ".tar.gz" };
        (target, ext)
    }

    /// 从 Release 资产列表中查找匹配当前平台的下载包
    pub fn find_matching_asset<'a>(
        release: &'a GitHubRelease,
        target: &str,
    ) -> Option<&'a ReleaseAsset> {
        if let Some(asset) = release.assets.iter().find(|a| a.name.contains(target)) {
            return Some(asset);
        }

        // 宽松匹配：操作系统与架构
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        release.assets.iter().find(|a| {
            let n = a.name.to_lowercase();
            let matches_os = n.contains(os) || (os == "macos" && n.contains("darwin"));
            let matches_arch = n.contains(arch)
                || (arch == "x86_64" && (n.contains("amd64") || n.contains("x64")))
                || (arch == "aarch64" && n.contains("arm64"))
                || (arch == "loongarch64" && n.contains("loong64"));
            matches_os && matches_arch
        })
    }
}
