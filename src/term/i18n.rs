//! 轻量化纯 Rust 零成本多语言字典与语言探测系统

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Zh,
    En,
}

static CURRENT_LANG: AtomicU8 = AtomicU8::new(0); // 0 = uninit/auto, 1 = Zh, 2 = En

/// 设置当前全局语言
pub fn set_language(lang: Language) {
    let val = match lang {
        Language::Zh => 1,
        Language::En => 2,
    };
    CURRENT_LANG.store(val, Ordering::SeqCst);
}

/// 获取当前语言（带环境自动探测）
pub fn current_language() -> Language {
    match CURRENT_LANG.load(Ordering::SeqCst) {
        1 => Language::Zh,
        2 => Language::En,
        _ => detect_environment_language(),
    }
}

/// 自动探测系统环境语言：包含 zh 判为中文，其余一律回退为英文
pub fn detect_environment_language() -> Language {
    // 1. 优先检查环境变量 QYZS_LANG
    if let Ok(val) = std::env::var("QYZS_LANG") {
        let lower = val.to_lowercase();
        if lower.contains("zh") || lower.contains("cn") || lower.contains("chinese") {
            return Language::Zh;
        } else if lower.contains("en") {
            return Language::En;
        }
    }

    // 2. Linux / macOS 国际化变量探测: LC_ALL -> LC_MESSAGES -> LANG
    let check_vars = ["LC_ALL", "LC_MESSAGES", "LANG"];
    for var in check_vars {
        if let Ok(val) = std::env::var(var) {
            let lower = val.to_lowercase();
            if lower.contains("zh") || lower.contains("chinese") {
                return Language::Zh;
            }
        }
    }

    // 3. 所有非中文一律回退到英文 (English)
    Language::En
}

/// 纯 Rust 编译期静态消息枚举
#[derive(Debug, Clone, Copy)]
pub enum MsgId {
    AppDescription,
    AllSystemsOperational,
    StatusDegraded,
    ConfigSaved,
    ConfigReset,
    AuthLoginSuccess,
    AuthLogoutSuccess,
    AuthMissingKey,
    AuthFilePermWarning,
    ChatExcludesPaddleOcr,
    SelfUpdateChecking,
    SelfUpdateUpToDate,
    SelfUpdateSuccess,
    SelfUpdateNpmWarning,
    DatasetDownloading,
    DatasetExtracting,
    DatasetSuccess,
    PdfSplitSuccess,
    StatusDiagnosticsHeader,
    StatusGatewayHeader,
}

impl MsgId {
    pub fn get(self) -> &'static str {
        let lang = current_language();
        match (self, lang) {
            (MsgId::AppDescription, Language::Zh) => "自然资源部庆阳智算中心大模型统一命令行工具",
            (MsgId::AppDescription, Language::En) => "Ministry of Natural Resources Qingyang AI Computing Center CLI",

            (MsgId::AllSystemsOperational, Language::Zh) => "系统全链路状态正常",
            (MsgId::AllSystemsOperational, Language::En) => "All systems operational",

            (MsgId::StatusDegraded, Language::Zh) => "部分专线服务未响应或网络超时",
            (MsgId::StatusDegraded, Language::En) => "Partial service degraded or timed out",

            (MsgId::ConfigSaved, Language::Zh) => "配置项已成功保存",
            (MsgId::ConfigSaved, Language::En) => "Configuration saved successfully",

            (MsgId::ConfigReset, Language::Zh) => "配置已恢复默认初始值",
            (MsgId::ConfigReset, Language::En) => "Configuration reset to defaults",

            (MsgId::AuthLoginSuccess, Language::Zh) => "凭据已安全保存 (权限 0600)",
            (MsgId::AuthLoginSuccess, Language::En) => "Credentials securely saved (mode 0600)",

            (MsgId::AuthLogoutSuccess, Language::Zh) => "本地凭据已清除",
            (MsgId::AuthLogoutSuccess, Language::En) => "Local credentials purged",

            (MsgId::AuthMissingKey, Language::Zh) => "未检测到 API Key，请执行 'qyzs auth login' 或设置 QYZS_APIKEY",
            (MsgId::AuthMissingKey, Language::En) => "API Key not found, run 'qyzs auth login' or set QYZS_APIKEY",

            (MsgId::AuthFilePermWarning, Language::Zh) => "凭据文件权限宽于 0600，建议执行 chmod 600 加固",
            (MsgId::AuthFilePermWarning, Language::En) => "Credentials file permission is wider than 0600, chmod 600 recommended",

            (MsgId::ChatExcludesPaddleOcr, Language::Zh) => "模型 'paddleocrvl' 是专属 OCR 视觉引擎，不支持对话交互。请使用 'qyzs ocr'",
            (MsgId::ChatExcludesPaddleOcr, Language::En) => "Model 'paddleocrvl' is OCR-only and cannot chat. Please use 'qyzs ocr'",

            (MsgId::SelfUpdateChecking, Language::Zh) => "正在检查 GitHub Release 最新版本...",
            (MsgId::SelfUpdateChecking, Language::En) => "Checking for latest GitHub Release...",

            (MsgId::SelfUpdateUpToDate, Language::Zh) => "当前已是最新版本，无需更新",
            (MsgId::SelfUpdateUpToDate, Language::En) => "Already up-to-date, no update needed",

            (MsgId::SelfUpdateSuccess, Language::Zh) => "已成功就地更新升级至新版本",
            (MsgId::SelfUpdateSuccess, Language::En) => "Successfully updated in-place to new version",

            (MsgId::SelfUpdateNpmWarning, Language::Zh) => "检测到当前通过 npm 安装，请执行 'npm install -g @qyzs-ai/cli@latest' 升级",
            (MsgId::SelfUpdateNpmWarning, Language::En) => "Installed via npm, please upgrade using: npm install -g @qyzs-ai/cli@latest",

            (MsgId::DatasetDownloading, Language::Zh) => "正在从 GitHub 下载测试基准数据集...",
            (MsgId::DatasetDownloading, Language::En) => "Downloading benchmark test dataset from GitHub...",

            (MsgId::DatasetExtracting, Language::Zh) => "正在解压缩测试数据集...",
            (MsgId::DatasetExtracting, Language::En) => "Extracting benchmark dataset...",

            (MsgId::DatasetSuccess, Language::Zh) => "测试数据集准备完成",
            (MsgId::DatasetSuccess, Language::En) => "Benchmark dataset prepared successfully",

            (MsgId::PdfSplitSuccess, Language::Zh) => "PDF 页面拆分完成",
            (MsgId::PdfSplitSuccess, Language::En) => "PDF pages split successfully",

            (MsgId::StatusDiagnosticsHeader, Language::Zh) => "客户端运行环境诊断",
            (MsgId::StatusDiagnosticsHeader, Language::En) => "Client Environment Diagnostics",

            (MsgId::StatusGatewayHeader, Language::Zh) => "远程网关连通性探针",
            (MsgId::StatusGatewayHeader, Language::En) => "Remote Gateway Connectivity Probes",
        }
    }
}

#[macro_export]
macro_rules! msg {
    ($id:ident) => {
        $crate::term::i18n::MsgId::$id.get()
    };
}
