//! GitHub 基准测试数据集下载、校验与解压管理

use crate::error::{QyzsError, QyzsResult};
use std::io::Cursor;
use std::path::Path;

pub struct DatasetManager;

impl DatasetManager {
    /// 从远程 GitHub 仓库下载测试数据集 ZIP 并解压到目标目录
    pub async fn download_and_extract(download_url: &str, dest_dir: &str) -> QyzsResult<usize> {
        let client = reqwest::Client::new();
        let resp = client
            .get(download_url)
            .send()
            .await
            .map_err(|e| QyzsError::Network(format!("下载数据集失败: {}", e)))?;

        if !resp.status().is_success() {
            return Err(QyzsError::Server(format!(
                "下载数据集返回 HTTP {}",
                resp.status()
            )));
        }

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| QyzsError::Network(format!("接收数据集数据流中断: {}", e)))?;

        Self::extract_zip_to_dir(&bytes, dest_dir)
    }

    /// 解压 ZIP 数据流到指定目录
    ///
    /// 契约: **归档内容原样展开到目标目录**, 不剥离任何层级——归档里有几层就落
    /// 几层。因此发布的数据集包须把 `tests/data/` 的**内容**（而非 `data/` 目录
    /// 本身）打包为归档根, 否则解到 `--dest tests/data` 会多出一层 `data/`。
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

#[cfg(test)]
mod tests {
    use super::DatasetManager;
    use std::io::Cursor;

    /// 构造一个 ZIP: entries 为 (条目名, 内容)
    fn make_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(Cursor::new(&mut buf));
            for (name, content) in entries {
                zw.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                std::io::Write::write_all(&mut zw, content.as_bytes()).unwrap();
            }
            zw.finish().unwrap();
        }
        buf
    }

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("qyzs-dataset-test-{tag}"));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn archive_contents_land_directly_in_dest() {
        // 发布的数据集包形态: 归档根即 tests/data/ 的内容 (chat/ ocr/ …)
        // 解到 <dest> 后应为 <dest>/<类目>/<文件>
        let zip = make_zip(&[
            ("chat/short_prompts.json", "{}"),
            ("ocr/paired_alignment_spec.json", "{}"),
        ]);
        let dest = tmpdir("root");
        let n = DatasetManager::extract_zip_to_dir(&zip, dest.to_str().unwrap()).unwrap();
        assert_eq!(n, 2);
        assert!(
            dest.join("chat/short_prompts.json").exists(),
            "归档内容应直接落在目标目录下, 实际落点: {:?}",
            list(&dest)
        );
        assert!(dest.join("ocr/paired_alignment_spec.json").exists());
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn archive_structure_is_preserved_as_is() {
        // 契约: 归档里有几层就落几层, 不剥离任何层级
        // (含"只有一个顶层目录"的情形——那层目录可能是有意义的)
        let zip = make_zip(&[("sample/test.txt", "hello")]);
        let dest = tmpdir("preserve");
        let n = DatasetManager::extract_zip_to_dir(&zip, dest.to_str().unwrap()).unwrap();
        assert_eq!(n, 1);
        assert!(
            dest.join("sample/test.txt").exists(),
            "顶层目录应保留, 实际落点: {:?}",
            list(&dest)
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("sample/test.txt")).unwrap(),
            "hello"
        );
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn flat_archive_is_kept_as_is() {
        let zip = make_zip(&[("a.json", "{}"), ("b.json", "{}")]);
        let dest = tmpdir("flat");
        let n = DatasetManager::extract_zip_to_dir(&zip, dest.to_str().unwrap()).unwrap();
        assert_eq!(n, 2);
        assert!(dest.join("a.json").exists());
        assert!(dest.join("b.json").exists());
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn nested_paths_keep_their_sub_structure() {
        let zip = make_zip(&[("x/y/z.json", "{}")]);
        let dest = tmpdir("nested");
        DatasetManager::extract_zip_to_dir(&zip, dest.to_str().unwrap()).unwrap();
        assert!(dest.join("x/y/z.json").exists());
        let _ = std::fs::remove_dir_all(&dest);
    }

    fn list(dir: &std::path::Path) -> Vec<String> {
        let mut v = Vec::new();
        fn walk(d: &std::path::Path, base: &std::path::Path, v: &mut Vec<String>) {
            if let Ok(rd) = std::fs::read_dir(d) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, base, v);
                    } else {
                        v.push(
                            p.strip_prefix(base)
                                .unwrap_or(&p)
                                .to_string_lossy()
                                .to_string(),
                        );
                    }
                }
            }
        }
        walk(dir, dir, &mut v);
        v.sort();
        v
    }
}
