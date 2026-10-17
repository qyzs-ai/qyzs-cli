//! 跨平台自更新与二进制原子就地替换 (安装即更新)

use super::github::GitHubClient;
use crate::error::{QyzsError, QyzsResult};
use std::io::Cursor;
use std::path::Path;

pub struct UpdateManager;

impl UpdateManager {
    /// 检查是否通过 npm 全局包运行
    pub fn is_running_under_npm() -> bool {
        if let Ok(exe) = std::env::current_exe() {
            let path_str = exe.to_string_lossy().to_lowercase();
            if path_str.contains("node_modules") || path_str.contains(".npm") || path_str.ends_with(".js") {
                return true;
            }
        }
        false
    }

    /// 执行在线自更新 (从 GitHub Release 下载匹配平台的最新包)
    pub async fn self_update(repo: &str) -> QyzsResult<bool> {
        if Self::is_running_under_npm() {
            return Err(QyzsError::Usage(crate::msg!(SelfUpdateNpmWarning).to_string()));
        }

        let release = GitHubClient::get_latest_release(repo).await?;
        let current_version = env!("CARGO_PKG_VERSION");
        let remote_tag = release.tag_name.trim_start_matches('v');

        if remote_tag == current_version {
            return Ok(false); // 已经是最新版
        }

        let (target, _) = GitHubClient::current_platform_target();
        let asset = GitHubClient::find_matching_asset(&release, target).ok_or_else(|| {
            QyzsError::Server(format!("未在 Release 中找到适配平台 '{}' 的构建产物", target))
        })?;

        // 流式下载资产包
        let client = reqwest::Client::new();
        let bytes = client
            .get(&asset.browser_download_url)
            .send()
            .await
            .map_err(|e| QyzsError::Network(format!("下载更新包失败: {}", e)))?
            .bytes()
            .await
            .map_err(|e| QyzsError::Network(format!("读取更新包数据流失败: {}", e)))?;

        // 解压并执行就地替换
        Self::apply_update_bytes(&bytes, &asset.name)?;

        Ok(true)
    }

    /// 若本地已有安装包，则直接安装/更新 (支持 .zip 与 .tar.gz)
    pub fn install_from_local_package(package_path: &str) -> QyzsResult<()> {
        let p = Path::new(package_path);
        if !p.exists() {
            return Err(QyzsError::Io(format!("指定安装包文件不存在: {}", package_path)));
        }

        let bytes = std::fs::read(p)
            .map_err(|e| QyzsError::Io(format!("读取本地安装包失败 {}: {}", package_path, e)))?;

        let file_name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("package.zip");

        Self::apply_update_bytes(&bytes, file_name)?;
        Ok(())
    }

    /// 从内存压缩包中解压出 qyzs 可执行文件并原子替换当前自身
    fn apply_update_bytes(bytes: &[u8], filename: &str) -> QyzsResult<()> {
        let current_exe = std::env::current_exe()
            .map_err(|e| QyzsError::Io(format!("无法获取当前程序运行路径: {}", e)))?;

        let binary_bytes = if filename.ends_with(".zip") {
            Self::extract_binary_from_zip(bytes)?
        } else {
            // 对于单二进制文件或 gzip，直接提取
            bytes.to_vec()
        };

        Self::atomic_replace_binary(&current_exe, &binary_bytes)?;
        Ok(())
    }

    /// 从 ZIP 压缩包提取 qyzs/qyzs.exe 二进制
    pub fn extract_binary_from_zip(bytes: &[u8]) -> QyzsResult<Vec<u8>> {
        let reader = Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(reader)
            .map_err(|e| QyzsError::Io(format!("解压 ZIP 失败: {}", e)))?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)
                .map_err(|e| QyzsError::Io(format!("读取 ZIP 内文件失败: {}", e)))?;
            let name_cow = file.name()
                .map_err(|e| QyzsError::Io(format!("获取 ZIP 条目名称失败: {}", e)))?;
            let name = name_cow.as_ref();

            if name == "qyzs" || name == "qyzs.exe" || name.ends_with("/qyzs") || name.ends_with("/qyzs.exe") {
                use std::io::Read;
                let mut content = Vec::new();
                file.read_to_end(&mut content)
                    .map_err(|e| QyzsError::Io(format!("解压二进制内容失败: {}", e)))?;
                return Ok(content);
            }
        }

        Err(QyzsError::Io("ZIP 压缩包中未找到 qyzs 二进制文件".to_string()))
    }

    /// 跨平台就地原子替换二进制
    pub fn atomic_replace_binary(target_path: &Path, new_binary_bytes: &[u8]) -> QyzsResult<()> {
        let dir = target_path
            .parent()
            .ok_or_else(|| QyzsError::Io("目标路径无有效父目录".to_string()))?;

        let tmp_file = dir.join(format!(".qyzs_update_{}.tmp", std::process::id()));

        // 1. 写入临时文件
        std::fs::write(&tmp_file, new_binary_bytes)
            .map_err(|e| QyzsError::Io(format!("写入临时更新文件失败: {}", e)))?;

        // 2. 设置 Unix 执行权限
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&tmp_file)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&tmp_file, perms)?;
        }

        // 3. 跨平台替换处理
        #[cfg(windows)]
        {
            // Windows 下运行中的可执行文件禁止直接覆盖写入，但允许重命名！
            let old_file = dir.join(format!(".qyzs_old_{}.bak", std::process::id()));
            if target_path.exists() {
                let _ = std::fs::rename(target_path, &old_file);
            }
            std::fs::rename(&tmp_file, target_path)
                .map_err(|e| QyzsError::Io(format!("Windows 替换二进制失败: {}", e)))?;
            let _ = std::fs::remove_file(old_file);
        }

        #[cfg(not(windows))]
        {
            // Unix (Linux/macOS) 下 std::fs::rename 是原子替换 (Atomic Rename)
            std::fs::rename(&tmp_file, target_path)
                .map_err(|e| QyzsError::Io(format!("原子替换二进制文件失败: {}", e)))?;
        }

        Ok(())
    }
}
