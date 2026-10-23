//! 业务工作流聚合流水线门面

pub mod dataset;
pub mod markscan;
pub mod ocr_verify;

pub use dataset::DatasetManager;
pub use markscan::MarkScanPipeline;
pub use ocr_verify::OcrVerifier;
