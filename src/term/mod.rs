//! 终端输出交互模块

pub mod color;
pub mod format;
pub mod i18n;
pub mod printer;

pub use color::ColorChoice;
pub use format::OutputFormat;
pub use i18n::{current_language, detect_environment_language, set_language, Language};
pub use printer::Printer;
