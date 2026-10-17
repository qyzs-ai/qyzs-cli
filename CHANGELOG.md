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
- 阶段②转换执行（speed 档）：PDF 本地 15 页/批切片 + MinerU 逐片同步抽取聚合；
  docx 整文件解析；规则清洗行为契约（只做结构归一不改写内容，见
  `tests/docs/QYZS-MARKITDOWN.md` §3.1）；过程数据工作区生命周期（内容指纹
  工作目录、并发锁、manifest 段×阶段状态机、段完成即流式清理切片二进制、成功
  清理/失败保留、`--resume` 幂等续跑、7 天残留时机性清扫）。
- `docs/RELEASE.md`：版本号制度、发布流程与发布条件清单（SSOT）。
- `CHANGELOG.md`（本文件）。
- `LICENSE`：补齐 Apache-2.0 许可证文本（此前 README/Cargo/npm 均声明但文件缺失）。

### Changed

- 破坏性：原"超长 PDF 切片转 Markdown"独立子命令移除，能力同版本被
  `markitdown --mode speed` 吸收（迁移豁免见 docs/RELEASE.md §1）。
  迁移映射: `qyzs <旧命令> --file X -o Y --batch-pages 15` →
  `qyzs markitdown X -o Y`（15 页/批切片为 markitdown 内置默认行为）；
  小文档原始直通继续使用 `qyzs parse`；页面级拆分继续使用 `qyzs pdf-split`。
  argv[0] 双形式调用 (`qyzs-<command>` 软链) 对被移除命令名不再可用。

### Changed

- 骨架重构：dispatch 分发层拆分为 `handlers/{inference,documents,system}`
  按域处理器（分发层只留"解析→映射→调用"）；`QyzsError` 迁移 thiserror
  声明式派生；`qyzs models` 矩阵改为 `capability_matrix.toml` 数据驱动渲染
  （新增 `[models.matrix]` 结构段，原有实测注释数值未动），并修正繁体混用；
  OCR 同源互验模块更名 `OcrVerifier`（与向量交叉验证同名异义消除）。
  CLI 行为契约（--help/退出码/输出语义）经回归验证保持不变。

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
