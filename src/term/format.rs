//! 输出格式枚举

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
    Raw,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            "raw" => Ok(Self::Raw),
            _ => Err(format!("未知输出格式: '{}', 仅支持 text, json, raw", s)),
        }
    }
}
