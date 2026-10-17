//! 通用配置文件管理 (~/.config/qyzs/config.toml)

use crate::error::{QyzsError, QyzsResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub model: ModelConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            network: NetworkConfig::default(),
            ui: UiConfig::default(),
            model: ModelConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub base_url: String,
    pub timeout: u64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            base_url: "http://59.219.113.13:9200".to_string(),
            timeout: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    pub language: String,
    pub color: String,
    pub format: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            language: "auto".to_string(),
            color: "auto".to_string(),
            format: "text".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub language: String,
    pub default_chat: String,
    pub default_ocr: String,
    pub default_parse: String,
    pub default_embed: String,
    pub default_rerank: String,
    pub default_asr: String,
    pub default_sam: String,
    #[serde(default)]
    pub params: HashMap<String, toml::Table>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            language: "auto".to_string(),
            default_chat: "dsv4".to_string(),
            default_ocr: "paddleocrvl".to_string(),
            default_parse: "mineru3.2.3".to_string(),
            default_embed: "bge-m3".to_string(),
            default_rerank: "qwen3-reranker-8b".to_string(),
            default_asr: "Qwen3-ASR-1.7B".to_string(),
            default_sam: "acid-sam3-full".to_string(),
            params: HashMap::new(),
        }
    }
}

impl Config {
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("qyzs").join("config.toml")
    }

    pub fn load() -> QyzsResult<Self> {
        let path = Self::default_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| QyzsError::Io(format!("读取配置文件失败 {}: {}", path.display(), e)))?;
        let config: Config = toml::from_str(&content)
            .map_err(|e| QyzsError::Config(format!("配置文件格式错误: {}", e)))?;
        Ok(config)
    }

    pub fn save(&self) -> QyzsResult<()> {
        let path = Self::default_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| QyzsError::Config(format!("序列化配置失败: {}", e)))?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn reset() -> QyzsResult<Self> {
        let default_config = Self::default();
        default_config.save()?;
        Ok(default_config)
    }
}
