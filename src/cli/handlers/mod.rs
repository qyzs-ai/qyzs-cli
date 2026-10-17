//! 子命令处理器 (按域分组的业务逻辑)
//!
//! dispatch.rs 只负责参数解析→处理器映射; 各 handler 承载实际业务。
//! `Ctx` 携带已解析配置、终端打印器与网关客户端, 避免重复构造。

pub mod documents;
pub mod inference;
pub mod system;

use crate::client::QyzsClient;
use crate::config::ResolvedConfig;
use crate::term::Printer;

/// 子命令执行上下文
pub struct Ctx<'a> {
    pub config: ResolvedConfig,
    pub printer: &'a Printer,
    pub client: &'a QyzsClient,
}

pub use inference::ChatArgs;
