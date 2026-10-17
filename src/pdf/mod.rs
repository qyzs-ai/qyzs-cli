//! 纯 Rust PDF 操作门面 (基于 lopdf 0.45，零外部 C 库依赖)

pub mod extract;
pub mod split;

pub use extract::{PdfExtractor, PdfMetadata};
pub use split::PdfSplitter;
