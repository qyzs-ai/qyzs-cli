//! QYZS CLI & Core Library
//! 自然资源部庆阳智算中心命令行客户端核心库

pub mod cli;
pub mod client;
pub mod config;
pub mod error;
pub mod pdf;
pub mod pipeline;
pub mod term;
pub mod updater;

pub use cli::{Cli, Commands};
pub use client::QyzsClient;
pub use config::{Config, Credentials, ResolvedConfig};
pub use error::{exit_code, QyzsError, QyzsResult};
pub use pdf::{PdfExtractor, PdfMetadata, PdfSplitter};
pub use pipeline::{CrossVerifier, DatasetManager, Pdf2MdPipeline};
pub use updater::UpdateManager;
