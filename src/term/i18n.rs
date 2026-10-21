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

/// 解析 locale/语言标签: 取首个语言子标签精确匹配 (POSIX/BCP-47 均可, 如
/// zh_CN.UTF-8 / en-US / chinese), zh 系判中文, en 系判英文, 无法识别返回 None
/// (由调用方决定兜底)。注意不能用子串包含判断——"french" 含 "en" 会被误判。
pub fn language_from_tag(tag: &str) -> Option<Language> {
    let lower = tag.trim().to_lowercase();
    let primary = lower.split(['_', '-', '.']).next()?;
    match primary {
        "zh" | "cn" | "chinese" => Some(Language::Zh),
        "en" | "english" => Some(Language::En),
        _ => None,
    }
}

/// 自动探测界面语言，优先级: QYZS_LANG > 标准 locale 变量 > [Windows/macOS] 系统语言 API > 英文兜底
pub fn detect_environment_language() -> Language {
    // 1. 项目环境变量与 POSIX 国际化变量 (LC_ALL > LC_MESSAGES > LANGUAGE > LANG)
    for var in ["QYZS_LANG", "LC_ALL", "LC_MESSAGES", "LANGUAGE", "LANG"] {
        if let Ok(val) = std::env::var(var) {
            if let Some(lang) = language_from_tag(&val) {
                return lang;
            }
        }
    }

    // 2. Windows 无 POSIX locale 变量, 经 Win32 API 读取系统 UI 语言
    #[cfg(windows)]
    if let Some(lang) = windows_ui_language() {
        return lang;
    }

    // 3. macOS 图形会话 (launchd 直启等) 同样常无 locale 环境变量, 经
    //    CoreFoundation 读取系统区域标识
    #[cfg(target_os = "macos")]
    if let Some(lang) = macos_system_language() {
        return lang;
    }

    // 4. 所有无法识别的环境一律回退到英文 (English)
    Language::En
}

#[cfg(windows)]
fn windows_ui_language() -> Option<Language> {
    // kernel32 随 Rust std 默认链接, 手写 FFI 避免引入 windows-sys 依赖
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    // 主语言标识为低 10 位; 0x0004 = Chinese
    let primary = unsafe { GetUserDefaultUILanguage() } & 0x03FF;
    Some(if primary == 0x0004 {
        Language::Zh
    } else {
        Language::En
    })
}

#[cfg(target_os = "macos")]
fn macos_system_language() -> Option<Language> {
    // CoreFoundation 为 macOS 系统自带框架, 手写 FFI 零第三方依赖;
    // allocator 传 NULL 即 kCFAllocatorDefault (CF 惯例)
    use std::ffi::{c_long, c_void};

    type CFStringRef = *mut c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFLocaleCopyCurrent(allocator: *mut c_void) -> CFStringRef;
        fn CFLocaleCopyIdentifier(locale: CFStringRef) -> CFStringRef;
        fn CFStringGetCString(
            string: CFStringRef,
            buffer: *mut u8,
            buffer_size: c_long,
            encoding: u32,
        ) -> bool;
        fn CFRelease(cf: *mut c_void);
    }

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    unsafe {
        let locale = CFLocaleCopyCurrent(std::ptr::null_mut());
        if locale.is_null() {
            return None;
        }
        let mut result = None;
        let identifier = CFLocaleCopyIdentifier(locale);
        if !identifier.is_null() {
            let mut buf = [0u8; 128];
            if CFStringGetCString(
                identifier,
                buf.as_mut_ptr(),
                buf.len() as c_long,
                K_CF_STRING_ENCODING_UTF8,
            ) {
                let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                result = std::str::from_utf8(&buf[..end]).ok().and_then(language_from_tag);
            }
            CFRelease(identifier);
        }
        CFRelease(locale);
        result
    }
}

/// 纯 Rust 编译期静态消息枚举
#[derive(Debug, Clone, Copy)]
pub enum MsgId {
    AppDescription,
    AllSystemsOperational,
    StatusDegraded,
    ConfigSaved,
    ConfigReset,
    WarnConfigResetDestructive,
    WarnAuthLogoutDestructive,
    WarnUsageClearDestructive,
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
    // ── scan ──
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
    // ── scan 体裁识别 / 排版样式 / 图片资产 ──
    MdDocKindDetected,
    MdClassifyModelFailed,
    MdClassifyFallbackFailed,
    MdWeakArbitrateFailed,
    MdImageKindFailed,
    MdStyleAdjusted,
    MdAssetsExtracted,
    MdAssetIndexNote,
    MdDecideRuleStrong,
    MdDecideWeakConfirmed,
    MdDecideRuleModelFailed,
    MdDecideWeakArbitrateFailed,
    MdDecideModelFallback,
    MdDecideModelFallbackFailed,
    MdDecideModelFirst,
    MdDecideWeakArbitrated,
    MdUnresolvedImageRefs,
    MdLabelMode,
    MdLabelChannel,
    MdLabelStages,
    MdLabelPdfPages,
    // ── usage ──
    UsageHeader,
    UsageLedgerLine,
    UsageRange,
    UsageTotalLine,
    UsageByKind,
    UsageByModel,
    UsageByKey,
    UsageByDay,
    UsageByHour,
    UsageModelRow,
    UsageDayRow,
    UsageArchivedNote,
    UsageNoData,
    UsageCleared,
}

impl MsgId {
    pub fn get(self) -> &'static str {
        self.get_for(current_language())
    }

    /// 取指定语言的文案 (get 的显式语言版本; 便于按语言单测)
    pub fn get_for(self, lang: Language) -> &'static str {
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
            (MsgId::WarnConfigResetDestructive, Language::Zh) =>
                "⚠ 即将把配置文件恢复为初始默认值, 全部自定义项 (含 per-model 超参表) 将丢失",
            (MsgId::WarnConfigResetDestructive, Language::En) =>
                "⚠ About to reset the config file to defaults; all custom entries (incl. per-model param table) will be lost",
            (MsgId::WarnAuthLogoutDestructive, Language::Zh) =>
                "⚠ 即将清除本地保存的 API Key 凭据文件, 操作不可撤销",
            (MsgId::WarnAuthLogoutDestructive, Language::En) =>
                "⚠ About to purge the locally saved API Key credentials file; this cannot be undone",

            (MsgId::AuthLoginSuccess, Language::Zh) => "凭据已安全保存 (权限 0600)",
            (MsgId::AuthLoginSuccess, Language::En) => "Credentials securely saved (mode 0600)",

            (MsgId::AuthLogoutSuccess, Language::Zh) => "本地凭据已清除",
            (MsgId::AuthLogoutSuccess, Language::En) => "Local credentials purged",

            (MsgId::AuthMissingKey, Language::Zh) => "未检测到 API Key，请执行 'qyzs auth login' 或设置 QYZS_APIKEY",
            (MsgId::AuthMissingKey, Language::En) => "API Key not found, run 'qyzs auth login' or set QYZS_APIKEY",

            (MsgId::AuthFilePermWarning, Language::Zh) => "凭据文件权限宽于 0600，建议执行 chmod 600 加固",
            (MsgId::AuthFilePermWarning, Language::En) => "Credentials file permission is wider than 0600, chmod 600 recommended",

            (MsgId::ChatExcludesPaddleOcr, Language::Zh) => "模型 'paddleocrvl' 是专属 OCR 视觉引擎，不支持对话交互。请使用 'qyzs scan <图片> --image-handling ocr'",
            (MsgId::ChatExcludesPaddleOcr, Language::En) => "Model 'paddleocrvl' is OCR-only and cannot chat. Please use 'qyzs scan <image> --image-handling ocr'",

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

            (MsgId::MdPlanHeader, Language::Zh) => "[scan] 流水线规划预览 (--dry-run)",
            (MsgId::MdPlanHeader, Language::En) => "[scan] Pipeline plan preview (--dry-run)",
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

            (MsgId::MdDocKindDetected, Language::Zh) => "文档类型: {0} ({1}) [{2}]",
            (MsgId::MdDocKindDetected, Language::En) => "Document kind: {0} ({1}) [{2}]",
            (MsgId::MdClassifyModelFailed, Language::Zh) => "体裁模型判定失败 ({0}), 回退规则",
            (MsgId::MdClassifyModelFailed, Language::En) => "Genre model failed ({0}); falling back to rules",
            (MsgId::MdClassifyFallbackFailed, Language::Zh) => "体裁模型兜底失败 ({0}), 维持通用文档",
            (MsgId::MdClassifyFallbackFailed, Language::En) => "Genre model fallback failed ({0}); keeping generic",
            (MsgId::MdWeakArbitrateFailed, Language::Zh) => "弱命中仲裁失败 ({0}), 保留规则判定",
            (MsgId::MdWeakArbitrateFailed, Language::En) => "Weak-match arbitration failed ({0}); keeping rule decision",
            (MsgId::MdImageKindFailed, Language::Zh) => "图像类型判定失败 ({0}), 按文档继续处理",
            (MsgId::MdImageKindFailed, Language::En) => "Image kind detection failed ({0}); continuing as document",
            (MsgId::MdStyleAdjusted, Language::Zh) => "样式校正: {0} 个标题按样式信息重定级",
            (MsgId::MdStyleAdjusted, Language::En) => "Style correction: {0} heading(s) re-levelled by style info",
            (MsgId::MdAssetsExtracted, Language::Zh) => "图片附件: 落盘 {0} 个 → {1}; 引用替换 {2} 处{3}",
            (MsgId::MdAssetsExtracted, Language::En) => "Image assets: {0} written to {1}; {2} reference(s) rewritten{3}",
            (MsgId::MdAssetIndexNote, Language::Zh) => "正文中的原始图片引用无法与抽取结果逐一对应，故在此按顺序列出，供人工核对位置。",
            (MsgId::MdAssetIndexNote, Language::En) => "Original image references cannot be matched one-to-one with the extracted assets; listed in order for manual cross-check.",

            (MsgId::MdUnresolvedImageRefs, Language::Zh) => "产物含 {0} 处未落盘图片引用 (网关未提供图片字节, 链接不可达)",
            (MsgId::MdUnresolvedImageRefs, Language::En) => "{0} unresolved image reference(s) in output (gateway provides no image bytes; links are dead)",
            (MsgId::MdDecideModelFirst, Language::Zh) => "模型({0})",
            (MsgId::MdDecideModelFirst, Language::En) => "model ({0})",
            (MsgId::MdDecideWeakArbitrated, Language::Zh) => "模型仲裁(规则弱命中 {0}, 改判 {1})",
            (MsgId::MdDecideWeakArbitrated, Language::En) => "model arbitration (rule weak match {0}, changed to {1})",
            (MsgId::MdDecideRuleStrong, Language::Zh) => "规则(强信号)",
            (MsgId::MdDecideRuleStrong, Language::En) => "rule (strong signal)",
            (MsgId::MdDecideWeakConfirmed, Language::Zh) => "规则(弱命中, 模型确认)",
            (MsgId::MdDecideWeakConfirmed, Language::En) => "rule (weak match, model confirmed)",
            (MsgId::MdDecideRuleModelFailed, Language::Zh) => "规则({0}, 模型失败回退)",
            (MsgId::MdDecideRuleModelFailed, Language::En) => "rule ({0}, model failed)",
            (MsgId::MdDecideWeakArbitrateFailed, Language::Zh) => "规则(弱命中, 仲裁失败保留)",
            (MsgId::MdDecideWeakArbitrateFailed, Language::En) => "rule (weak match, arbitration failed)",
            (MsgId::MdDecideModelFallback, Language::Zh) => "模型(规则判不出兜底)",
            (MsgId::MdDecideModelFallback, Language::En) => "model (fallback for inconclusive rules)",
            (MsgId::MdDecideModelFallbackFailed, Language::Zh) => "规则(判不出, 兜底失败)",
            (MsgId::MdDecideModelFallbackFailed, Language::En) => "rule (inconclusive, fallback failed)",

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
            (MsgId::UsageByKind, Language::Zh) => "[按调用族]",
            (MsgId::UsageByKind, Language::En) => "[By Call Family]",
            (MsgId::UsageByModel, Language::Zh) => "[按模型]",
            (MsgId::UsageByModel, Language::En) => "[By Model]",
            (MsgId::UsageByKey, Language::Zh) => "[按凭据]",
            (MsgId::UsageByKey, Language::En) => "[By Credential]",
            (MsgId::UsageByDay, Language::Zh) => "[按日]",
            (MsgId::UsageByDay, Language::En) => "[By Day]",
            (MsgId::UsageByHour, Language::Zh) => "[按小时]",
            (MsgId::UsageByHour, Language::En) => "[By Hour]",
            (MsgId::UsageModelRow, Language::Zh) => "  {0}  {1} 次请求 | 输入 {2} | 输出 {3} | 总计 {4} tk",
            (MsgId::UsageModelRow, Language::En) => "  {0}  {1} request(s) | prompt {2} | completion {3} | overall {4} tk",
            (MsgId::UsageDayRow, Language::Zh) => "  {0}  {1} 次请求 | 总计 {2} tk",
            (MsgId::UsageDayRow, Language::En) => "  {0}  {1} request(s) | overall {2} tk",
            (MsgId::UsageArchivedNote, Language::Zh) => "  另有归档历史 {0} 次请求 | 总计 {1} tk (超 {2} 天明细已按日折叠, 仅按日/模型/调用族粒度可查)",
            (MsgId::UsageArchivedNote, Language::En) => "  Plus archived history: {0} request(s) | overall {1} tk (details older than {2} days folded by day; day/model/family granularity only)",
            (MsgId::UsageNoData, Language::Zh) => "本地用量台账暂无匹配记录 (发起一次 chat/embed 等网关调用后此处将显示统计)",
            (MsgId::UsageNoData, Language::En) => "No matching usage records yet (run a chat/embed gateway call to populate statistics)",
            (MsgId::UsageCleared, Language::Zh) => "已清空本地用量台账 (删除 {0} 条记录)",
            (MsgId::UsageCleared, Language::En) => "Usage ledger cleared ({0} record(s) removed)",
            (MsgId::WarnUsageClearDestructive, Language::Zh) =>
                "⚠ 即将清空本地用量台账 (全部历史请求与 Token 记录), 操作不可撤销",
            (MsgId::WarnUsageClearDestructive, Language::En) =>
                "⚠ About to clear the local usage ledger (all historical request and token records); this cannot be undone",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_tag_parsing() {
        assert_eq!(language_from_tag("zh"), Some(Language::Zh));
        assert_eq!(language_from_tag("zh_CN.UTF-8"), Some(Language::Zh));
        assert_eq!(language_from_tag("zh-cn"), Some(Language::Zh));
        assert_eq!(language_from_tag("Chinese"), Some(Language::Zh));
        assert_eq!(language_from_tag("en"), Some(Language::En));
        assert_eq!(language_from_tag("en_US.UTF-8"), Some(Language::En));
        assert_eq!(language_from_tag("fr_FR.UTF-8"), None);
        assert_eq!(language_from_tag("french"), None);
        assert_eq!(language_from_tag("C"), None);
        assert_eq!(language_from_tag(""), None);
    }

    /// 破坏性操作警告: 必须中英双语齐备 (否则英文界面下中英混排) 且带 ⚠ 告警标记
    #[test]
    fn destructive_warnings_are_bilingual() {
        // 语言是进程级全局, 跑完还原, 免得并行测试里留下英文状态
        let saved = current_language();
        for id in [
            MsgId::WarnConfigResetDestructive,
            MsgId::WarnAuthLogoutDestructive,
            MsgId::WarnUsageClearDestructive,
        ] {
            set_language(Language::Zh);
            let zh = id.get().to_string();
            set_language(Language::En);
            let en = id.get().to_string();

            assert!(!zh.is_empty(), "{:?} 中文案为空", id);
            assert!(!en.is_empty(), "{:?} 英文案为空", id);
            assert_ne!(zh, en, "{:?} 中英文案相同, 疑似漏译", id);
            assert!(zh.starts_with('⚠'), "{:?} 中文案缺 ⚠ 标记", id);
            assert!(en.starts_with('⚠'), "{:?} 英文案缺 ⚠ 标记", id);
            // 占位符数量须一致, 否则 msgf! 填充会错位
            assert_eq!(
                zh.matches('{').count(),
                en.matches('{').count(),
                "{:?} 中英文案占位符数量不一致",
                id
            );
        }
        set_language(saved);
    }
}
