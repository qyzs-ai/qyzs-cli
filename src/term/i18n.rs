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
    // ── markitdown ──
    MdPlanHeader,
    MdInputLocal,
    MdInputUrl,
    MdDetectContent,
    MdDetectExtension,
    MdDetectManual,
    MdUnsupportedStatus,
    MdReason,
    MdPolishModel,
    MdOutputTarget,
    MdDefaultBalanced,
    MdSliceProgress,
    MdSaved,
    MdPolishFallback,
    MdCrossVerifySummary,
    MdSecondaryTooThin,
    MdArbitrateConfirmed,
    MdArbitrateCorrected,
    MdArbitrateUnverified,
    MdGlobalReview,
    MdLabelInput,
    MdLabelFormat,
    MdLabelMode,
    MdLabelChannel,
    MdLabelStages,
    MdLabelPdfPages,
    // ── usage ──
    UsageHeader,
    UsageLedgerLine,
    UsageRange,
    UsageTotalLine,
    UsageByModel,
    UsageByDay,
    UsageModelRow,
    UsageDayRow,
    UsageNoData,
    UsageCleared,
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

            (MsgId::MdPlanHeader, Language::Zh) => "[markitdown] 流水线规划预览 (--dry-run)",
            (MsgId::MdPlanHeader, Language::En) => "[markitdown] Pipeline plan preview (--dry-run)",
            (MsgId::MdInputLocal, Language::Zh) => "本地文件",
            (MsgId::MdInputLocal, Language::En) => "local file",
            (MsgId::MdInputUrl, Language::Zh) => "网络下载",
            (MsgId::MdInputUrl, Language::En) => "downloaded from URL",
            (MsgId::MdDetectContent, Language::Zh) => "内容嗅探",
            (MsgId::MdDetectContent, Language::En) => "content sniffing",
            (MsgId::MdDetectExtension, Language::Zh) => "扩展名兜底",
            (MsgId::MdDetectExtension, Language::En) => "extension fallback",
            (MsgId::MdDetectManual, Language::Zh) => "手动指定",
            (MsgId::MdDetectManual, Language::En) => "manually specified",
            (MsgId::MdUnsupportedStatus, Language::Zh) => "  状态:        ✗ 该格式暂不支持转换",
            (MsgId::MdUnsupportedStatus, Language::En) => "  Status:       ✗ conversion not supported for this format",
            (MsgId::MdReason, Language::Zh) => "  原因:        ",
            (MsgId::MdReason, Language::En) => "  Reason:       ",
            (MsgId::MdPolishModel, Language::Zh) => "  润色模型:    ",
            (MsgId::MdPolishModel, Language::En) => "  Polish model: ",
            (MsgId::MdOutputTarget, Language::Zh) => "  输出目标:    ",
            (MsgId::MdOutputTarget, Language::En) => "  Output:       ",
            (MsgId::MdDefaultBalanced, Language::Zh) => "默认 balanced 档 (单通道 + dsv4 单轮排版润色 + 轻量质检)",
            (MsgId::MdDefaultBalanced, Language::En) => "Default balanced mode (single channel + dsv4 single-pass polish + light check)",
            (MsgId::MdSliceProgress, Language::Zh) => "── 片段进度 {0}/{1} ──",
            (MsgId::MdSliceProgress, Language::En) => "── segment {0}/{1} ──",
            (MsgId::MdSaved, Language::Zh) => "Markdown 结果已保存至: {0}",
            (MsgId::MdSaved, Language::En) => "Markdown result saved to: {0}",
            (MsgId::MdPolishFallback, Language::Zh) => "润色质检回退",
            (MsgId::MdPolishFallback, Language::En) => "polish quality-check fallback",
            (MsgId::MdCrossVerifySummary, Language::Zh) => "交叉验证: {0} 段参与, {1} 段低于阈值 {2}",
            (MsgId::MdCrossVerifySummary, Language::En) => "Cross-verify: {0} paragraph(s) checked, {1} below threshold {2}",
            (MsgId::MdSecondaryTooThin, Language::Zh) => "第二通道文本层过薄 ({0} 字符 vs 主通道 {1} 字符), 交叉验证不可用, rigor 降级为 balanced 行为",
            (MsgId::MdSecondaryTooThin, Language::En) => "Secondary channel too thin ({0} chars vs {1} in primary); cross-verify unavailable, rigor falls back to balanced",
            (MsgId::MdArbitrateConfirmed, Language::Zh) => "段落与证据一致 (相似度 {0}), 保留原文",
            (MsgId::MdArbitrateConfirmed, Language::En) => "Paragraph matches evidence (similarity {0}), kept as-is",
            (MsgId::MdArbitrateCorrected, Language::Zh) => "段落已按第二通道证据修正 (相似度 {0})",
            (MsgId::MdArbitrateCorrected, Language::En) => "Paragraph corrected against secondary evidence (similarity {0})",
            (MsgId::MdArbitrateUnverified, Language::Zh) => "段落仲裁不可决, 已标记 qyzs:unverified (相似度 {0})",
            (MsgId::MdArbitrateUnverified, Language::En) => "Arbitration unresolved, paragraph marked qyzs:unverified (similarity {0})",
            (MsgId::MdGlobalReview, Language::Zh) => "全局审查建议",
            (MsgId::MdGlobalReview, Language::En) => "Global review suggestions",

            (MsgId::MdLabelInput, Language::Zh) => "  输入:        ",
            (MsgId::MdLabelInput, Language::En) => "  Input:        ",
            (MsgId::MdLabelFormat, Language::Zh) => "  识别格式:    ",
            (MsgId::MdLabelFormat, Language::En) => "  Format:       ",
            (MsgId::MdLabelMode, Language::Zh) => "  处理档位:    ",
            (MsgId::MdLabelMode, Language::En) => "  Mode:         ",
            (MsgId::MdLabelChannel, Language::Zh) => "  规划通道:    ",
            (MsgId::MdLabelChannel, Language::En) => "  Channel:      ",
            (MsgId::MdLabelStages, Language::Zh) => "  执行阶段:    ",
            (MsgId::MdLabelStages, Language::En) => "  Stages:       ",
            (MsgId::MdLabelPdfPages, Language::Zh) => "  PDF 页数:    {0} 页 → {1} 批切片 (每批 ≤{2} 页)",
            (MsgId::MdLabelPdfPages, Language::En) => "  PDF pages:    {0} → {1} slice batch(es) (≤{2} pages each)",

            (MsgId::UsageHeader, Language::Zh) => "本地网关用量统计",
            (MsgId::UsageHeader, Language::En) => "Local Gateway Usage Statistics",
            (MsgId::UsageLedgerLine, Language::Zh) => "  台账文件:     {0} (共 {1} 条, 匹配 {2} 条)",
            (MsgId::UsageLedgerLine, Language::En) => "  Ledger file:  {0} ({1} record(s), {2} matched)",
            (MsgId::UsageRange, Language::Zh) => "  统计范围:     {0} ~ {1}",
            (MsgId::UsageRange, Language::En) => "  Range:        {0} ~ {1}",
            (MsgId::UsageTotalLine, Language::Zh) => "  合计:         {0} 次请求 | 输入 {1} tk | 输出 {2} tk | 总计 {3} tk (其中 {4} 次含 Token 数据)",
            (MsgId::UsageTotalLine, Language::En) => "  Total:        {0} request(s) | prompt {1} tk | completion {2} tk | overall {3} tk ({4} with token data)",
            (MsgId::UsageByModel, Language::Zh) => "[按模型]",
            (MsgId::UsageByModel, Language::En) => "[By Model]",
            (MsgId::UsageByDay, Language::Zh) => "[按日]",
            (MsgId::UsageByDay, Language::En) => "[By Day]",
            (MsgId::UsageModelRow, Language::Zh) => "  {0}  {1} 次请求 | 输入 {2} | 输出 {3} | 总计 {4} tk",
            (MsgId::UsageModelRow, Language::En) => "  {0}  {1} request(s) | prompt {2} | completion {3} | overall {4} tk",
            (MsgId::UsageDayRow, Language::Zh) => "  {0}  {1} 次请求 | 总计 {2} tk",
            (MsgId::UsageDayRow, Language::En) => "  {0}  {1} request(s) | overall {2} tk",
            (MsgId::UsageNoData, Language::Zh) => "本地用量台账暂无匹配记录 (发起一次 chat/embed 等网关调用后此处将显示统计)",
            (MsgId::UsageNoData, Language::En) => "No matching usage records yet (run a chat/embed gateway call to populate statistics)",
            (MsgId::UsageCleared, Language::Zh) => "已清空本地用量台账 (删除 {0} 条记录)",
            (MsgId::UsageCleared, Language::En) => "Usage ledger cleared ({0} record(s) removed)",
        }
    }
}

#[macro_export]
macro_rules! msg {
    ($id:ident) => {
        $crate::term::i18n::MsgId::$id.get()
    };
}

/// 用序号占位符 ({0},{1},...) 填充目录消息。
/// format! 不接受非字面量格式串, 参数化目录消息统一走本函数。
pub fn fill(template: &str, params: &[String]) -> String {
    let mut out = template.to_string();
    for (i, p) in params.iter().enumerate() {
        out = out.replace(&format!("{{{}}}", i), p);
    }
    out
}

#[macro_export]
macro_rules! msgf {
    ($id:ident, $($param:expr),*) => {
        $crate::term::i18n::fill($crate::term::i18n::MsgId::$id.get(), &[$($param.to_string()),*])
    };
}
