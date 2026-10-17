//! 命令行交互与路由分发门面

pub mod args;
pub mod completions;
pub mod dispatch;

pub use args::{Cli, Commands};
pub use dispatch::run;
