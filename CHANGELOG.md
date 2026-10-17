# Changelog

本项目所有对外可见的变更记录于此。格式遵循
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [SemVer 2.0.0](https://semver.org/)；
版本制度、发布流程与发布条件见 [docs/RELEASE.md](docs/RELEASE.md)。

## [Unreleased]

### Added

- `markitdown` 全能文档转 Markdown 流水线子命令（阶段①）：本地/URL 输入自动
  格式嗅探（PDF 魔数、OOXML 条目名、OLE2 目录流名、图像魔数、HTML，扩展名兜底）、
  格式通道路由、三档流水线（speed/balanced/rigor）规划、`--dry-run` 预览；
  OLE2 容器配 OOXML 扩展名时给出"疑似加密文档"警告。
- `docs/RELEASE.md`：版本号制度、发布流程与发布条件清单（SSOT）。
- `CHANGELOG.md`（本文件）。
- `LICENSE`：补齐 Apache-2.0 许可证文本（此前 README/Cargo/npm 均声明但文件缺失）。

### Fixed

- `npm/build.sh`：发布装配时同步改写 `optionalDependencies` 平台包版本钉，
  修复新版顶层包会拉取旧版平台二进制的版本漂移问题。

## [0.1.0] - 2026-10-17

初始全量发布（单提交 3cb3e1c，未打 tag）：

- 17 个子命令：`chat` / `ocr` / `parse` / `embed` / `rerank` / `asr` / `sam` /
  `pdf-split` / `verify` / `status` / `models` / `config` / `auth` / `upgrade` /
  `completions` / `dataset` / PDF 转 Markdown 流水线入口。
- 能力边界探测固化为 `capability_matrix.toml`（SSOT 校准真值）。
- 客户端护栏：embeddings 长文本自动分块 + 均值池化 + 256 条拆批；chat >16 图
  自动分批（真实网关 6/6 实测通过）。
- 六平台构建矩阵（linux x64/arm64/loong64、macOS x64/arm64、Windows x64）
  与 npm 包族 provenance 发布流水线。

[Unreleased]: https://github.com/qyzs-ai/qyzs-cli/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/qyzs-ai/qyzs-cli/releases/tag/v0.1.0
