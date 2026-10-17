//! 遥感双时相变化检测与 SAM 分割 (/api/aiCenterChat/v3/form/acid-sam3-full)

use super::QyzsClient;
use crate::error::{QyzsError, QyzsResult};
use reqwest::multipart::{Form, Part};
use std::path::Path;

impl QyzsClient {
    /// 执行遥感双时相变化检测
    pub async fn run_remote_sensing(
        &self,
        image_before: &str,
        image_after: &str,
        aoi_geojson: Option<&str>,
        threshold: Option<f64>,
    ) -> QyzsResult<String> {
        let key = self.config.ensure_auth()?;

        let p1 = Path::new(image_before);
        let p2 = Path::new(image_after);
        if !p1.exists() || !p2.exists() {
            return Err(QyzsError::Io(format!("遥感影像文件不存在: {} 或 {}", image_before, image_after)));
        }

        let b1 = std::fs::read(p1).map_err(|e| QyzsError::Io(format!("读取前时相影像失败: {}", e)))?;
        let b2 = std::fs::read(p2).map_err(|e| QyzsError::Io(format!("读取后时相影像失败: {}", e)))?;

        let part1 = Part::bytes(b1)
            .file_name(p1.file_name().and_then(|n| n.to_str()).unwrap_or("before.tif").to_string())
            .mime_str("application/octet-stream")
            .map_err(|e| QyzsError::Usage(e.to_string()))?;

        let part2 = Part::bytes(b2)
            .file_name(p2.file_name().and_then(|n| n.to_str()).unwrap_or("after.tif").to_string())
            .mime_str("application/octet-stream")
            .map_err(|e| QyzsError::Usage(e.to_string()))?;

        let mut form = Form::new()
            .part("image_before", part1)
            .part("image_after", part2);

        if let Some(aoi) = aoi_geojson {
            form = form.text("aoi", aoi.to_string());
        }

        if let Some(th) = threshold {
            form = form.text("threshold", th.to_string());
        }

        let resp = self
            .http
            .post("api/aiCenterChat/v3/form/acid-sam3-full", Some(key))
            .multipart(form)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(QyzsError::Server(format!("遥感流水线网关返回 HTTP {}: {}", status, err_text)));
        }

        let res_text = resp.text().await.map_err(|e| QyzsError::Server(e.to_string()))?;
        Ok(res_text)
    }
}
