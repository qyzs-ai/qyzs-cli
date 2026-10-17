//! markitdown 过程数据工作区生命周期
//!
//! 布局: `<缓存基目录>/markitdown/<输入 sha256 前 16 位>/`
//! ├── manifest.json   输入指纹 + 段×阶段状态机 (幂等续跑依据)
//! ├── lock            并发互斥锁 (create_new, 携带 PID)
//! ├── slices/         网关切片二进制 (段完成后立即删除 — 流式清理)
//! └── segments/       段级文本产物 (成功后随目录整体清理)
//!
//! 生命周期: 成功删除整个目录 (--no-clean 保留)；失败保留现场供 --resume；
//! 启动时机性清扫 mtime 超过 7 天且无锁文件的残留目录。

use crate::error::{QyzsError, QyzsResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read as _, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 残留工作目录清扫阈值
const SWEEP_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

/// 段内单个阶段的状态记录 (预留以支撑后续多轮次阶段)
pub type StageMap = std::collections::BTreeMap<String, bool>;

/// manifest.json 持久化结构
#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    /// 输入文件完整 sha256 (十六进制)；续跑时校验一致才复用
    pub input_sha256: String,
    pub format: String,
    pub mode: String,
    /// manifest 创建时间 (Unix 秒)
    pub created_at: u64,
    /// 段 id → 阶段名 → 是否完成
    pub segments: std::collections::BTreeMap<String, StageMap>,
}

pub struct Workspace {
    pub root: PathBuf,
    pub manifest: Manifest,
    lock_path: PathBuf,
    resumed: bool,
}

impl Workspace {
    /// 打开 (或创建) 输入对应的工作区；返回 (workspace, 是否为续跑)。
    /// `resume=false` 时既有现场一律作废 (默认重跑即重新处理)；仅显式续跑复用。
    pub fn open(
        input_path: &Path,
        format_key: &str,
        mode_key: &str,
        resume: bool,
    ) -> QyzsResult<(Self, bool)> {
        let cache_root = cache_base()?.join("markitdown");
        Self::open_in(cache_root, input_path, format_key, mode_key, resume)
    }

    /// 显式指定缓存根的打开入口 (测试隔离用)
    pub fn open_in(
        cache_root: PathBuf,
        input_path: &Path,
        format_key: &str,
        mode_key: &str,
        resume: bool,
    ) -> QyzsResult<(Self, bool)> {
        let input_sha256 = sha256_file(input_path)?;
        Self::sweep_stale(&cache_root, SWEEP_AGE);

        let root = cache_root.join(&input_sha256[..16]);
        let lock_path = root.join("lock");
        let manifest_path = root.join("manifest.json");

        std::fs::create_dir_all(&root)
            .map_err(|e| QyzsError::Io(format!("无法创建工作目录 {}: {}", root.display(), e)))?;

        // 续跑判定: 显式续跑 + 既有 manifest + 输入指纹一致
        let mut resumed = false;
        let manifest = if manifest_path.exists() && resume {
            match std::fs::read_to_string(&manifest_path)
                .ok()
                .and_then(|s| serde_json::from_str::<Manifest>(&s).ok())
                .filter(|m| m.input_sha256 == input_sha256)
            {
                Some(m) => {
                    resumed = true;
                    m
                }
                None => {
                    // 输入已变化或 manifest 损坏 → 全新开始
                    std::fs::remove_dir_all(&root).map_err(|e| {
                        QyzsError::Io(format!("无法重置工作目录 {}: {}", root.display(), e))
                    })?;
                    std::fs::create_dir_all(&root)?;
                    Self::fresh_manifest(&input_sha256, format_key, mode_key)
                }
            }
        } else {
            Self::fresh_manifest(&input_sha256, format_key, mode_key)
        };

        // 并发互斥: create_new 失败即视为另一进程在处理相同输入
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    QyzsError::Usage(format!(
                        "相同输入的转换似乎正在另一进程进行 (锁文件: {})；确认无并发后可删除该锁文件",
                        lock_path.display()
                    ))
                } else {
                    QyzsError::Io(format!("无法创建锁文件 {}: {}", lock_path.display(), e))
                }
            })?;
        let mut lock = lock;
        let _ = writeln!(lock, "pid={}", std::process::id());

        for sub in ["slices", "segments"] {
            std::fs::create_dir_all(root.join(sub))?;
        }

        let ws = Self {
            root,
            manifest,
            lock_path,
            resumed,
        };
        ws.persist_manifest()?;
        Ok((ws, resumed))
    }

    fn fresh_manifest(sha: &str, format_key: &str, mode_key: &str) -> Manifest {
        Manifest {
            input_sha256: sha.to_string(),
            format: format_key.to_string(),
            mode: mode_key.to_string(),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            segments: Default::default(),
        }
    }

    /// 时机性清扫: mtime 超过 max_age 且无锁文件的残留工作目录
    fn sweep_stale(cache_root: &Path, max_age: Duration) {
        let Ok(entries) = std::fs::read_dir(cache_root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() || path.join("lock").exists() {
                continue;
            }
            let stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age > max_age);
            if stale {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }

    pub fn is_resumed(&self) -> bool {
        self.resumed
    }

    /// 查询段×阶段是否已完成 (幂等续跑)
    pub fn stage_done(&self, segment: &str, stage: &str) -> bool {
        self.manifest
            .segments
            .get(segment)
            .and_then(|m| m.get(stage))
            .copied()
            .unwrap_or(false)
    }

    /// 标记段×阶段完成: 持久化 manifest + 流式清理该段二进制产物
    pub fn complete_stage(
        &mut self,
        segment: &str,
        stage: &str,
        binaries: &[PathBuf],
    ) -> QyzsResult<()> {
        self.manifest
            .segments
            .entry(segment.to_string())
            .or_default()
            .insert(stage.to_string(), true);
        self.persist_manifest()?;
        for path in binaries {
            if path.exists() {
                std::fs::remove_file(path).map_err(|e| {
                    QyzsError::Io(format!("清理中间产物 {} 失败: {}", path.display(), e))
                })?;
            }
        }
        Ok(())
    }

    /// 段级文本产物落盘 (供 --resume 复用)
    pub fn write_segment_text(&self, segment: &str, content: &str) -> QyzsResult<PathBuf> {
        let path = self.root.join("segments").join(format!("{}.md", segment));
        std::fs::write(&path, content)?;
        Ok(path)
    }

    /// 读取已完成的段级文本产物
    pub fn read_segment_text(&self, segment: &str) -> QyzsResult<String> {
        let path = self.root.join("segments").join(format!("{}.md", segment));
        std::fs::read_to_string(&path)
            .map_err(|e| QyzsError::Io(format!("读取段产物 {} 失败: {}", path.display(), e)))
    }

    pub fn slices_dir(&self) -> PathBuf {
        self.root.join("slices")
    }

    fn persist_manifest(&self) -> QyzsResult<()> {
        let json = serde_json::to_string_pretty(&self.manifest)
            .map_err(|e| QyzsError::Server(format!("manifest 序列化失败: {}", e)))?;
        std::fs::write(self.root.join("manifest.json"), json)?;
        Ok(())
    }

    /// 成功收尾: 释放锁并删除整个工作目录 (--no-clean 时保留产物)
    pub fn finish_success(mut self, no_clean: bool) -> QyzsResult<()> {
        self.release_lock();
        if no_clean {
            return Ok(());
        }
        if self.root.exists() {
            std::fs::remove_dir_all(&self.root).map_err(|e| {
                QyzsError::Io(format!("清理工作目录 {} 失败: {}", self.root.display(), e))
            })?;
        }
        Ok(())
    }

    /// 失败收尾: 释放锁、保留现场 (供 --resume)
    pub fn finish_failure(&mut self) {
        self.release_lock();
    }

    fn release_lock(&mut self) {
        if self.lock_path.exists() {
            let _ = std::fs::remove_file(&self.lock_path);
        }
        self.resumed = false;
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // 兜底: 无论何种路径退出都释放锁；产物目录的清理由显式收尾方法负责
        self.release_lock();
    }
}

/// 缓存基目录 (与配置目录同一平台约定: XDG > HOME > APPDATA > 当前目录)
fn cache_base() -> QyzsResult<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(base.join("qyzs"))
}

/// 流式计算文件 sha256 (支持大文件，不整读进内存)
pub fn sha256_file(path: &Path) -> QyzsResult<String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| QyzsError::Io(format!("无法打开输入文件 {}: {}", path.display(), e)))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(content).unwrap();
        p
    }

    #[test]
    fn test_open_fresh_and_resume() {
        let cache = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir_in(cache.path()).unwrap();
        let input = sample_file(work.path(), "input.bin", b"hello qyzs");
        let ws_cache = cache.path().join("wsroot");

        let (mut ws, resumed) =
            Workspace::open_in(ws_cache.clone(), &input, "pdf", "speed", false).unwrap();
        assert!(!resumed);
        assert!(ws.root.join("manifest.json").exists());
        assert!(ws.slices_dir().exists());
        assert!(!ws.stage_done("slice-01", "extract"));

        ws.complete_stage("slice-01", "extract", &[]).unwrap();
        assert!(ws.stage_done("slice-01", "extract"));
        let root = ws.root.clone();
        ws.finish_success(false).unwrap();
        assert!(!root.exists(), "成功后工作目录应被删除");

        // 二次运行: 全新开始 (上次已清理)
        let (mut ws, resumed) =
            Workspace::open_in(ws_cache.clone(), &input, "pdf", "speed", false).unwrap();
        assert!(!resumed);

        // 模拟中断: 标记一个段完成后不清理直接 drop (仅释放锁)
        ws.complete_stage("slice-01", "extract", &[]).unwrap();
        drop(ws);

        // 显式续跑: 复用已完成段
        let (ws, resumed) =
            Workspace::open_in(ws_cache.clone(), &input, "pdf", "speed", true).unwrap();
        assert!(resumed, "显式续跑且指纹一致时应复用现场");
        assert!(ws.stage_done("slice-01", "extract"));
        drop(ws);

        // 不带续跑标志: 现场作废重新处理 (默认重跑即重新处理)
        let (ws, resumed) =
            Workspace::open_in(ws_cache, &input, "pdf", "speed", false).unwrap();
        assert!(!resumed, "未显式续跑时既有现场应作废");
        assert!(!ws.stage_done("slice-01", "extract"));
        ws.finish_success(false).unwrap();
    }

    #[test]
    fn test_no_clean_preserves_workspace() {
        let cache = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir_in(cache.path()).unwrap();
        let input = sample_file(work.path(), "input.bin", b"keep me");
        let ws_cache = cache.path().join("wsroot");

        let ws = Workspace::open_in(ws_cache, &input, "pdf", "speed", false).unwrap().0;
        let root = ws.root.clone();
        ws.finish_success(true).unwrap();
        assert!(root.exists(), "--no-clean 应保留工作目录");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_hash_mismatch_resets() {
        let cache = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir_in(cache.path()).unwrap();
        let input = sample_file(work.path(), "input.bin", b"v1");
        let ws_cache = cache.path().join("wsroot");

        let mut ws = Workspace::open_in(ws_cache.clone(), &input, "pdf", "speed", false).unwrap().0;
        ws.complete_stage("doc", "extract", &[]).unwrap();
        drop(ws);

        // 输入内容变化 → 自动全新开始，旧状态作废 (即使显式要求续跑)
        sample_file(work.path(), "input.bin", b"v2-changed");
        let (ws, resumed) = Workspace::open_in(ws_cache, &input, "pdf", "speed", true).unwrap();
        assert!(!resumed, "指纹不一致时即使显式续跑也应全新开始");
        assert!(!ws.stage_done("doc", "extract"));
        ws.finish_success(false).unwrap();
    }

    #[test]
    fn test_lock_conflict() {
        let cache = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir_in(cache.path()).unwrap();
        let input = sample_file(work.path(), "input.bin", b"locked");
        let ws_cache = cache.path().join("wsroot");

        let ws = Workspace::open_in(ws_cache.clone(), &input, "pdf", "speed", false).unwrap().0;
        let second = Workspace::open_in(ws_cache.clone(), &input, "pdf", "speed", false);
        assert!(
            matches!(second, Err(QyzsError::Usage(_))),
            "并发打开相同输入应报 Usage 错"
        );
        ws.finish_success(false).unwrap();

        // 锁释放后可再次打开
        assert!(Workspace::open_in(ws_cache, &input, "pdf", "speed", false).is_ok());
    }

    #[test]
    fn test_streaming_cleanup_and_segment_text() {
        let cache = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir_in(cache.path()).unwrap();
        let input = sample_file(work.path(), "input.bin", b"slices test");
        let ws_cache = cache.path().join("wsroot");

        let mut ws = Workspace::open_in(ws_cache, &input, "pdf", "speed", false).unwrap().0;
        let slice = ws.slices_dir().join("part-01.pdf");
        std::fs::write(&slice, b"fake pdf bytes").unwrap();

        let seg_path = ws
            .write_segment_text("part-01", "extracted markdown")
            .unwrap();
        assert!(seg_path.exists());
        assert_eq!(ws.read_segment_text("part-01").unwrap(), "extracted markdown");

        ws.complete_stage("part-01", "extract", &[slice.clone()])
            .unwrap();
        assert!(!slice.exists(), "段完成后切片二进制应被流式清理");
        ws.finish_success(false).unwrap();
    }

    #[test]
    fn test_sweep_stale_and_lock_protection() {
        let cache = tempfile::tempdir().unwrap();
        let ws_cache = cache.path().join("wsroot");

        // 直测 sweep_stale (max_age=0: 一切无锁目录均视为过期) → 验证清扫与锁保护两条分支
        std::fs::create_dir_all(ws_cache.join("stale_target")).unwrap();
        std::fs::create_dir_all(ws_cache.join("locked_target")).unwrap();
        std::fs::write(ws_cache.join("locked_target/lock"), b"pid=1").unwrap();

        Workspace::sweep_stale(&ws_cache, Duration::ZERO);

        assert!(!ws_cache.join("stale_target").exists(), "无锁过期目录应被清扫");
        assert!(ws_cache.join("locked_target").exists(), "持锁目录不应被清扫");

        // open_in 路径走 7 天阈值: 近期创建的目录不应被误扫
        let work = tempfile::tempdir_in(cache.path()).unwrap();
        let input = sample_file(work.path(), "input.bin", b"sweep trigger");
        let (ws, _) =
            Workspace::open_in(ws_cache.clone(), &input, "pdf", "speed", false).unwrap();
        assert!(ws_cache.join("locked_target").exists(), "近期目录不应被 open_in 清扫");
        ws.finish_success(false).unwrap();
    }

    #[test]
    fn test_sha256_streaming() {
        let cache = tempfile::tempdir().unwrap();
        // 200KB > 内部缓冲 64KB, 验证多轮读取正确
        let content: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
        let input = sample_file(cache.path(), "big.bin", &content);

        let hex = sha256_file(&input).unwrap();
        assert_eq!(hex.len(), 64);

        // 相同内容 → 相同指纹；不同内容 → 不同指纹
        let input2 = sample_file(cache.path(), "big2.bin", &content);
        assert_eq!(hex, sha256_file(&input2).unwrap());
        let input3 = sample_file(cache.path(), "big3.bin", b"different");
        assert_ne!(hex, sha256_file(&input3).unwrap());
    }
}
