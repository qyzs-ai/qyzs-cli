//! 全局统一错误与退出码定义


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

#[derive(Debug, thiserror::Error)]
pub enum QyzsError {
    #[error("用法错误: {0}")]
    Usage(String),
    #[error("认证凭据错误: {0}")]
    Auth(String),
    #[error("网络连接失败: {0}")]
    Network(String),
    #[error("请求超时: {0}")]
    Timeout(String),
    #[error("服务端异常: {0}")]
    Server(String),
    #[error("IO 操作失败: {0}")]
    Io(String),
    #[error("PDF 处理异常: {0}")]
    Pdf(String),
    #[error("配置异常: {0}")]
    Config(String),
}

impl QyzsError {
    /// 在保留错误变体与退出码的前提下附加一条可操作提示 (换行追加)
    pub fn with_hint(self, hint: &str) -> Self {
        fn attach(mut msg: String, hint: &str) -> String {
            msg.push('\n');
            msg.push_str(hint);
            msg
        }
        match self {
            Self::Usage(m) => Self::Usage(attach(m, hint)),
            Self::Auth(m) => Self::Auth(attach(m, hint)),
            Self::Network(m) => Self::Network(attach(m, hint)),
            Self::Timeout(m) => Self::Timeout(attach(m, hint)),
            Self::Server(m) => Self::Server(attach(m, hint)),
            Self::Io(m) => Self::Io(attach(m, hint)),
            Self::Pdf(m) => Self::Pdf(attach(m, hint)),
            Self::Config(m) => Self::Config(attach(m, hint)),
        }
    }

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
