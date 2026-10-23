//! 终端调色盘与 ANSI 色彩判定

use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorChoice {
    pub fn should_color(&self) -> bool {
        match self {
            Self::Never => false,
            Self::Always => true,
            Self::Auto => {
                // 遵循 NO_COLOR 行业标准 (https://no-color.org)
                if std::env::var("NO_COLOR").is_ok() {
                    return false;
                }
                // 检查 stdout 是否为真实 TTY 终端
                std::io::stdout().is_terminal()
            }
        }
    }
}

/// 安全 ANSI 标准色彩包装 (兼容明亮与暗黑终端主题，禁用黑白硬编码)
pub struct Styler {
    enabled: bool,
}

impl Styler {
    pub fn new(choice: ColorChoice) -> Self {
        Self {
            enabled: choice.should_color(),
        }
    }

    pub fn green(&self, text: &str) -> String {
        if self.enabled {
            format!("\x1b[32m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn red(&self, text: &str) -> String {
        if self.enabled {
            format!("\x1b[31m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn yellow(&self, text: &str) -> String {
        if self.enabled {
            format!("\x1b[33m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn cyan(&self, text: &str) -> String {
        if self.enabled {
            format!("\x1b[36m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn bold(&self, text: &str) -> String {
        if self.enabled {
            format!("\x1b[1m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }

    pub fn dim(&self, text: &str) -> String {
        if self.enabled {
            format!("\x1b[2m{}\x1b[0m", text)
        } else {
            text.to_string()
        }
    }
}
