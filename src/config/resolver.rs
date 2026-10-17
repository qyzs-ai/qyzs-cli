//! 四层级联配置解析器 (Flags > Env > Files > Defaults)

use super::auth::Credentials;
use super::profile::Config;
use crate::error::{QyzsError, QyzsResult};
use crate::term::{ColorChoice, Language, OutputFormat};

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub api_key: String,
    pub api_key_source: String,
    pub base_url: String,
    pub timeout_seconds: u64,
    pub ui_language: Language,
    pub model_language: String,
    pub color_choice: ColorChoice,
    pub output_format: OutputFormat,
    pub config: Config,
}

/// 凭证环境变量名（统一命名规范：QYZS_ 前缀 + 业务语义）
const API_KEY_ENV_VAR: &str = "QYZS_APIKEY";

/// 规整环境变量值：去除首尾空白，空白视为未设置
fn normalize_env_value(raw: Option<String>) -> Option<String> {
    raw.map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

impl ResolvedConfig {
    pub fn resolve(
        cli_key: Option<&str>,
        cli_base_url: Option<&str>,
        cli_timeout: Option<u64>,
        cli_lang: Option<&str>,
        cli_model_lang: Option<&str>,
        cli_color: Option<&str>,
        cli_format: Option<&str>,
    ) -> QyzsResult<Self> {
        let config = Config::load().unwrap_or_default();
        let creds = Credentials::load().unwrap_or_default();

        // 1. 解析 API Key
        let (api_key, api_key_source) = if let Some(k) = cli_key.filter(|s| !s.is_empty()) {
            (k.to_string(), "CLI Argument".to_string())
        } else if let Some(k) = normalize_env_value(std::env::var(API_KEY_ENV_VAR).ok()) {
            (k, format!("Environment Variable ({})", API_KEY_ENV_VAR))
        } else if let Some(c) = creds.filter(|c| !c.api_key.is_empty()) {
            (c.api_key, format!("Credentials File ({})", Credentials::default_path().display()))
        } else {
            (String::new(), "None".to_string())
        };

        // 2. 解析 Base URL
        let base_url = if let Some(url) = cli_base_url.filter(|s| !s.is_empty()) {
            url.to_string()
        } else if let Ok(url) = std::env::var("QYZS_BASE_URL") {
            let trimmed = url.trim().to_string();
            if !trimmed.is_empty() {
                trimmed
            } else if !config.network.base_url.is_empty() {
                config.network.base_url.clone()
            } else {
                "http://59.219.113.13:9200".to_string()
            }
        } else if !config.network.base_url.is_empty() {
            config.network.base_url.clone()
        } else {
            "http://59.219.113.13:9200".to_string()
        };

        // 3. 解析超时
        let timeout_seconds = if let Some(t) = cli_timeout {
            t
        } else if let Some(t) = std::env::var("QYZS_TIMEOUT").ok().and_then(|s| s.parse::<u64>().ok()) {
            t
        } else {
            config.network.timeout
        };

        // 4. 解析界面语言
        let ui_language = if let Some(lang) = cli_lang {
            match lang.to_lowercase().as_str() {
                "zh" | "cn" | "zh-cn" => Language::Zh,
                _ => Language::En,
            }
        } else if let Ok(lang) = std::env::var("QYZS_LANG") {
            let lower = lang.to_lowercase();
            if lower.contains("zh") || lower.contains("cn") {
                Language::Zh
            } else {
                Language::En
            }
        } else if config.ui.language != "auto" {
            match config.ui.language.to_lowercase().as_str() {
                "zh" | "cn" | "zh-cn" => Language::Zh,
                _ => Language::En,
            }
        } else {
            crate::term::i18n::detect_environment_language()
        };

        // 5. 解析模型语言
        let model_language = if let Some(ml) = cli_model_lang {
            ml.to_lowercase()
        } else if let Ok(ml) = std::env::var("QYZS_MODEL_LANG") {
            ml.to_lowercase()
        } else {
            config.model.language.to_lowercase()
        };

        // 6. 解析色彩偏好
        let color_choice = if let Some(c) = cli_color {
            match c.to_lowercase().as_str() {
                "always" => ColorChoice::Always,
                "never" => ColorChoice::Never,
                _ => ColorChoice::Auto,
            }
        } else {
            match config.ui.color.to_lowercase().as_str() {
                "always" => ColorChoice::Always,
                "never" => ColorChoice::Never,
                _ => ColorChoice::Auto,
            }
        };

        // 7. 解析输出格式
        let output_format = if let Some(f) = cli_format {
            f.parse().unwrap_or(OutputFormat::Text)
        } else {
            config.ui.format.parse().unwrap_or(OutputFormat::Text)
        };

        Ok(Self {
            api_key,
            api_key_source,
            base_url,
            timeout_seconds,
            ui_language,
            model_language,
            color_choice,
            output_format,
            config,
        })
    }

    /// 要求 API Key 必须存在，否则返回 AUTH 错误
    pub fn ensure_auth(&self) -> QyzsResult<&str> {
        if self.api_key.is_empty() {
            Err(QyzsError::Auth(crate::msg!(AuthMissingKey).to_string()))
        } else {
            Ok(&self.api_key)
        }
    }

    /// 根据子命令获取默认模型（基于实测评分最优矩阵）
    pub fn get_default_model(&self, subcommand: &str) -> &str {
        match subcommand {
            "chat" => &self.config.model.default_chat,
            "ocr" => &self.config.model.default_ocr,
            "parse" => &self.config.model.default_parse,
            "embed" => &self.config.model.default_embed,
            "rerank" => &self.config.model.default_rerank,
            "asr" => &self.config.model.default_asr,
            "sam" => &self.config.model.default_sam,
            _ => "dsv4",
        }
    }

    /// 获取针对特定模型的专属超参 (如 temperature, max_tokens 等)
    pub fn get_model_param_f64(&self, model: &str, param: &str) -> Option<f64> {
        let normalized = model.replace('.', "_").replace('-', "_");
        self.config
            .model
            .params
            .get(&normalized)
            .or_else(|| self.config.model.params.get(model))
            .and_then(|table| table.get(param))
            .and_then(|v| v.as_float())
    }

    pub fn get_model_param_i64(&self, model: &str, param: &str) -> Option<i64> {
        let normalized = model.replace('.', "_").replace('-', "_");
        self.config
            .model
            .params
            .get(&normalized)
            .or_else(|| self.config.model.params.get(model))
            .and_then(|table| table.get(param))
            .and_then(|v| v.as_integer())
    }

    pub fn get_model_param_bool(&self, model: &str, param: &str) -> Option<bool> {
        let normalized = model.replace('.', "_").replace('-', "_");
        self.config
            .model
            .params
            .get(&normalized)
            .or_else(|| self.config.model.params.get(model))
            .and_then(|table| table.get(param))
            .and_then(|v| v.as_bool())
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_env_value;

    #[test]
    fn env_value_is_trimmed() {
        assert_eq!(
            normalize_env_value(Some("  sk-test-key  ".to_string())),
            Some("sk-test-key".to_string())
        );
    }

    #[test]
    fn env_value_blank_is_treated_as_unset() {
        assert_eq!(normalize_env_value(Some("   ".to_string())), None);
        assert_eq!(normalize_env_value(Some(String::new())), None);
        assert_eq!(normalize_env_value(None), None);
    }
}
