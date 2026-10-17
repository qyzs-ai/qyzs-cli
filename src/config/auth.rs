//! API 认证凭据管理 (XDG 0600 安全存储)

use crate::error::{QyzsError, QyzsResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Credentials {
    pub api_key: String,
}

impl Credentials {
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("qyzs").join("credentials.toml")
    }

    pub fn load() -> QyzsResult<Option<Self>> {
        let path = Self::default_path();
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| QyzsError::Io(format!("无法读取凭据文件 {}: {}", path.display(), e)))?;
        let creds: Credentials = toml::from_str(&content)
            .map_err(|e| QyzsError::Config(format!("凭据文件格式错误: {}", e)))?;
        Ok(Some(creds))
    }

    pub fn save(&self) -> QyzsResult<()> {
        let path = Self::default_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| QyzsError::Config(format!("序列化凭据失败: {}", e)))?;
        std::fs::write(&path, content)?;

        // Unix 环境下强制加固为 0600
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path)?.permissions();
            perms.set_mode(0o600);
            std::fs::set_permissions(&path, perms)?;
        }

        Ok(())
    }

    pub fn purge() -> QyzsResult<bool> {
        let path = Self::default_path();
        if path.exists() {
            std::fs::remove_file(&path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 掩码展示 (sk-abc1***456)
    pub fn mask_key(key: &str) -> String {
        if key.len() <= 10 {
            "***".to_string()
        } else {
            format!("{}***{}", &key[..7], &key[key.len() - 3..])
        }
    }

    /// 审计文件权限 (Unix: 检查是否符合 0600)
    pub fn is_permission_secure(path: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(path) {
                let mode = meta.permissions().mode();
                // 确保组和其他用户无任何读写执行权限 (后6位为0)
                return (mode & 0o077) == 0;
            }
        }
        true
    }
}
