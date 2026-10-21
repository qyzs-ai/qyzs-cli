//! 业务工作流聚合流水线门面

pub mod ocr_verify;
pub mod dataset;
pub mod markscan;

pub use ocr_verify::OcrVerifier;
pub use dataset::DatasetManager;
pub use markscan::MarkScanPipeline;
