//! GitHub 基准测试数据集下载、校验与解压管理

use crate::error::{QyzsError, QyzsResult};
use std::io::Cursor;
use std::path::Path;

pub struct DatasetManager;

impl DatasetManager {
    /// 从远程 GitHub 仓库下载测试数据集 ZIP 并解压到目标目录
    pub async fn download_and_extract(
        download_url: &str,
        dest_dir: &str,
    ) -> QyzsResult<usize> {
        let client = reqwest::Client::new();
        let resp = client
            .get(download_url)
            .send()
            .await
            .map_err(|e| QyzsError::Network(format!("下载数据集失败: {}", e)))?;

        if !resp.status().is_success() {
            return Err(QyzsError::Server(format!("下载数据集返回 HTTP {}", resp.status())));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| QyzsError::Network(format!("接收数据集数据流中断: {}", e)))?;

        Self::extract_zip_to_dir(&bytes, dest_dir)
    }

    /// 解压 ZIP 数据流到指定目录
    pub fn extract_zip_to_dir(bytes: &[u8], dest_dir: &str) -> QyzsResult<usize> {
        let reader = Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(reader)
            .map_err(|e| QyzsError::Io(format!("解析数据集 ZIP 失败: {}", e)))?;

        let out_dir = Path::new(dest_dir);
        std::fs::create_dir_all(out_dir)?;

        let mut extracted_count = 0;

        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| QyzsError::Io(format!("读取压缩包条目失败: {}", e)))?;

            let outpath = match file.enclosed_name() {
                Some(path) => out_dir.join(path),
                None => continue,
            };

            if file.is_dir() {
                std::fs::create_dir_all(&outpath)?;
            } else {
                if let Some(p) = outpath.parent() {
                    if !p.exists() {
                        std::fs::create_dir_all(p)?;
                    }
                }
                let mut outfile = std::fs::File::create(&outpath)?;
                std::io::copy(&mut file, &mut outfile)?;
                extracted_count += 1;
            }
        }

        Ok(extracted_count)
    }
}
