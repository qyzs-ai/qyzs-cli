//! 标准状态输出格式化器

use super::color::{ColorChoice, Styler};

pub struct Printer {
    styler: Styler,
}

impl Printer {
    pub fn new(choice: ColorChoice) -> Self {
        Self {
            styler: Styler::new(choice),
        }
    }

    pub fn ok(&self, msg: &str) {
        println!("{} {}", self.styler.green("[OK]  "), msg);
    }

    pub fn info(&self, msg: &str) {
        println!("{} {}", self.styler.cyan("[INFO]"), msg);
    }

    /// 进度类状态走 stderr，避免污染命令的机器可读 stdout。
    pub fn info_err(&self, msg: &str) {
        eprintln!("{} {}", self.styler.cyan("[INFO]"), msg);
    }

    pub fn warn(&self, msg: &str) {
        eprintln!("{} {}", self.styler.yellow("[WARN]"), msg);
    }

    pub fn err(&self, msg: &str) {
        eprintln!("{} {}", self.styler.red("[ERR] "), msg);
    }

    pub fn dim(&self, msg: &str) {
        println!("{}", self.styler.dim(msg));
    }

    pub fn styler(&self) -> &Styler {
        &self.styler
    }
}
