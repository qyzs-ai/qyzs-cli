//! markscan 的运行状态输出。
//!
//! 转换产物可能直接写到 stdout，因此所有运行状态都写 stderr。消息只描述阶段、
//! 模型和结果，不包含 API Key、提示词、输入路径或模型响应正文，既便于人观察，
//! 也不污染管道中的 Markdown/JSON。

/// 常规流水线状态。
pub fn info(message: &str) {
    eprintln!("{}", line("[INFO]", message));
}

/// 不阻断任务的异常或降级。
pub fn warn(message: &str) {
    eprintln!("{}", line("[WARN]", message));
}

/// 任务项被主动跳过。
pub fn skip(message: &str) {
    eprintln!("{}", line("[SKIP]", message));
}

/// 与远程 AI 请求相关的状态。
pub fn ai(message: &str) {
    eprintln!("{}", line("[AI]", message));
}

fn line(prefix: &str, message: &str) -> String {
    format!("{prefix} {message}")
}

#[cfg(test)]
mod tests {
    use super::line;

    #[test]
    fn prefixes_are_stable_for_log_filtering() {
        assert_eq!(
            line("[AI]", "OCR: calling qwen36"),
            "[AI] OCR: calling qwen36"
        );
        assert_eq!(line("[WARN]", "fallback"), "[WARN] fallback");
    }
}
