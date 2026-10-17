//! 全局统一错误与退出码定义

use std::fmt;

/// 标准退出码定义
pub mod exit_code {
    pub const SUCCESS: i32 = 0;
    pub const FAILURE: i32 = 1;
    pub const USAGE_ERR: i32 = 2;
    pub const AUTH_ERR: i32 = 3;
    pub const TIMEOUT_ERR: i32 = 4;
    pub const IO_ERR: i32 = 5;
}

pub type QyzsResult<T> = Result<T, QyzsError>;

#[derive(Debug)]
pub enum QyzsError {
    Usage(String),
    Auth(String),
    Network(String),
    Timeout(String),
    Server(String),
    Io(String),
    Pdf(String),
    Config(String),
}

impl QyzsError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => exit_code::USAGE_ERR,
            Self::Auth(_) => exit_code::AUTH_ERR,
            Self::Timeout(_) => exit_code::TIMEOUT_ERR,
            Self::Io(_) | Self::Pdf(_) => exit_code::IO_ERR,
            Self::Network(_) | Self::Server(_) | Self::Config(_) => exit_code::FAILURE,
        }
    }
}

impl fmt::Display for QyzsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(msg) => write!(f, "用法错误: {}", msg),
            Self::Auth(msg) => write!(f, "认证凭据错误: {}", msg),
            Self::Network(msg) => write!(f, "网络连接失败: {}", msg),
            Self::Timeout(msg) => write!(f, "请求超时: {}", msg),
            Self::Server(msg) => write!(f, "服务端异常: {}", msg),
            Self::Io(msg) => write!(f, "IO 操作失败: {}", msg),
            Self::Pdf(msg) => write!(f, "PDF 处理异常: {}", msg),
            Self::Config(msg) => write!(f, "配置异常: {}", msg),
        }
    }
}

impl std::error::Error for QyzsError {}

impl From<std::io::Error> for QyzsError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

impl From<serde_json::Error> for QyzsError {
    fn from(err: serde_json::Error) -> Self {
        Self::Server(format!("JSON 解析失败: {}", err))
    }
}

impl From<reqwest::Error> for QyzsError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            Self::Timeout(err.to_string())
        } else if err.is_connect() {
            Self::Network(format!("无法连接到目标服务: {}", err))
        } else {
            Self::Server(err.to_string())
        }
    }
}
