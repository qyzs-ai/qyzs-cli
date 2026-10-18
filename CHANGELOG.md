# Changelog

本项目所有对外可见的变更记录于此。格式遵循
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [SemVer 2.0.0](https://semver.org/)；
版本制度、发布流程与发布条件见 [docs/RELEASE.md](docs/RELEASE.md)。

## [Unreleased]

### Added

- `--lang` 全局参数（短形式 `-l`）：界面语言
  解析链为 CLI 显式值 > `QYZS_LANG` > 配置文件 > 系统语言探测 > 英文兜底。
  系统探测扩展两处：locale 标签改为按子标签精确解析（`zh_CN.UTF-8`/`en-US`，
  修复旧子串匹配把 `french` 误判为英文类的问题，且 `LC_ALL=en_*` 不再穿透到
  下一层变量）；Windows 无 POSIX locale 变量，新增 Win32 `GetUserDefaultUILanguage`
  FFI 读取系统 UI 语言；macOS 图形会话（launchd 直启等）同样常无 locale 变量，
  新增 CoreFoundation `CFLocaleCopyCurrent` FFI 读取系统区域标识（两者均为
  手写系统框架声明，零新增依赖；已对 windows-gnu 与 aarch64/x86_64-apple-darwin
  目标完成类型检查）。**默认语言行为变更**：
  此前一律默认中文，现跟随系统语言。CLI 显式非法值（如 `french`）报
  USAGE_ERR 并提示合法值；环境变量/配置文件层保持 best-effort 回退英文。
- 全局参数短形式补齐与短语法规范文档化：`-b` base-url / `-t` timeout /
  `-l` language / `-c` color / `-f` format；`--api-key` 与 `--model-lang` 无短
  形式（`-k` 已被 `auth login --key`、`-m` 已被子命令级 `--model` 占用）。子命令
  级补齐：markitdown `--model` 增 `-m`（对齐 chat/embed/rerank/asr 约定）、
  rerank `--top-n` 增 `-n`。规范全文落在 `src/cli/args.rs` 模块头注释与 README
  「全局参数与短语法规范」小节。参数面契约集成测试覆盖新短形式与语言链。
- `usage` 本地网关用量统计子命令：client 层在 chat（流式/非流式）/ocr/asr/
  embed/rerank 成功调用点自动追加 JSONL 台账
  （`$XDG_STATE_HOME/qyzs/usage.jsonl`），`qyzs usage` 聚合展示请求数与
  prompt/completion/total Token（按模型/按日分组），支持 `--days N`
  （UTC 日界）、`--model`、`--json`、`--clear`；记录为 best-effort，
  写失败静默不影响主命令。流式 Token 经 `stream_options.include_usage`
  获取（2026-10-18 实测网关支持）；rerank 响应新增 `usage.total_tokens`
  解析（实测网关仅回总量）。MinerU/SAM 为任务型接口无 Token 数据，不记录。
- `markitdown` 全能文档转 Markdown 流水线子命令（阶段①）：本地/URL 输入自动
  格式嗅探（PDF 魔数、OOXML 条目名、OLE2 目录流名、图像魔数、HTML，扩展名兜底）、
  格式通道路由、三档流水线（speed/balanced/rigor）规划、`--dry-run` 预览；
  OLE2 容器配 OOXML 扩展名时给出"疑似加密文档"警告。
- 全格式通道补齐：图像（`--image-handling` auto/ocr/describe/both/skip，
  paddleocrvl OCR + qwen3-VL 描述，auto 按文字产出量自适应）与 HTML/网页
  （htmd/html5ever 纯 Rust 转换，script/style 入口剥离）。
- 阶段②转换执行（speed 档）：PDF 本地 15 页/批切片 + MinerU 逐片同步抽取聚合；
  docx 整文件解析；规则清洗行为契约（只做结构归一不改写内容，见
  `tests/docs/QYZS-MARKITDOWN.md` §3.1）；过程数据工作区生命周期（内容指纹
  工作目录、并发锁、manifest 段×阶段状态机、段完成即流式清理切片二进制、成功
  清理/失败保留、`--resume` 幂等续跑、7 天残留时机性清扫）。
- `docs/RELEASE.md`：版本号制度、发布流程与发布条件清单（SSOT）。
- `CHANGELOG.md`（本文件）。
- `LICENSE`：补齐 Apache-2.0 许可证文本（此前 README/Cargo/npm 均声明但文件缺失）。

### Changed

- 全局参数取值校验收紧（与语言参数同一策略：CLI 显式非法值报 USAGE_ERR 并列出
  合法值，环境变量/配置文件层保持 best-effort 回退）：`--color` 非法值此前静默
  回退 auto，现报错；`--format` 非法值此前静默回退 text，现报错；`--model-lang`
  非法值此前原样透传（等价 auto 不注入），现报错，合法值集对齐 chat 注入逻辑
  为 `auto|cn|zh|zh-cn|en`（`zh` 为消费端本已支持而文档缺失的值，help/README 同步）；
  `--base-url` 非 http(s) 前缀此前拖到请求时才失败，现启动即报错。
  markitdown 子命令参数（`--mode`/`--type`/`--image-handling`/`--min-confidence`）
  经审计本已有显式校验，未改动。校验策略写入 `src/cli/args.rs` 模块头注释。
- README 子命令速览覆盖全 18 个子命令（新增 `asr`/`sam`/`verify`/`dataset`/
  `upgrade`/`completions` 展示），markitdown rigor 档示例同步为已接入状态。
- 破坏性：原"超长 PDF 切片转 Markdown"独立子命令移除，能力同版本被
  `markitdown --mode speed` 吸收（迁移豁免见 docs/RELEASE.md §1）。
  迁移映射: `qyzs <旧命令> --file X -o Y --batch-pages 15` →
  `qyzs markitdown X -o Y`（15 页/批切片为 markitdown 内置默认行为）；
  小文档原始直通继续使用 `qyzs parse`；页面级拆分继续使用 `qyzs pdf-split`。
  argv[0] 双形式调用 (`qyzs-<command>` 软链) 对被移除命令名不再可用。
- markitdown 运行时消息全量 i18n 目录化（新增 `msgf!` 序号占位符填充机制，
  format! 不接受非字面量格式串的替代）；修复管道下游关闭 (`| head`) 时的
  Broken pipe panic（Unix 恢复 SIGPIPE 默认行为）。
- 修复 rigor 仲裁告警模板 `{0:.2}` 占位符与 `msgf!` 纯序号替换机制不匹配
  导致相似度以字面量 `{0:.2}` 显示的缺陷（调用点已先行 `{:.2}` 格式化）。
- CI：tag 触发时校验 tag 与 `Cargo.toml` 版本一致性；Release 资产新增
  tests_data.zip 基准数据集打包（`qyzs dataset` 默认 URL 同步指向 v0.2.0）。
- 新增网关依赖集成测试 `tests/gateway_tests.rs`（`#[ignore]` 门控，CI 自动
  跳过；本地注入凭证后 `cargo test -- --ignored` 放行）。
- 骨架重构：dispatch 分发层拆分为 `handlers/{inference,documents,system}`
  按域处理器（分发层只留"解析→映射→调用"）；`QyzsError` 迁移 thiserror
  声明式派生；`qyzs models` 矩阵改为 `capability_matrix.toml` 数据驱动渲染
  （新增 `[models.matrix]` 结构段，原有实测注释数值未动），并修正繁体混用；
  OCR 同源互验模块更名 `OcrVerifier`（与向量交叉验证同名异义消除）。
  CLI 行为契约（--help/退出码/输出语义）经回归验证保持不变。

### Fixed

- MinerU 任务型响应解析：空白页文档 `md_content` 为空时不再泄漏原始 JSON 到
  输出；网关返回 pending/running 异步状态时显式报错而非静默；整体抽取结果
  为空时 markitdown 报错，不产出空文件假成功。
- LLM 响应取文：新增 `ChatResponse::primary_text`（content 空回退 reasoning），
  修复 qwen3-VL 非流式偶发空响应（实测该模型把完整回答放入 reasoning 字段）。
- markitdown 转换场景客户端超时下限 300s（实测扫描件解析长尾超过默认 60s）。
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
