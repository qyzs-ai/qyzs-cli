//! 自更新与跨平台二进制安装管理门面

pub mod github;
pub mod installer;

pub use github::{GitHubRelease, ReleaseAsset};
pub use installer::UpdateManager;
