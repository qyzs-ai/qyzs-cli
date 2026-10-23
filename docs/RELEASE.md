# 版本与发布制度

本文档是 qyzs CLI 版本编号、发布流程与发布条件的唯一事实源（SSOT）。
发布相关变更（流程、门槛）必须先改本文档再执行。

## 1. 版本号制度

- 严格遵循 [SemVer 2.0.0](https://semver.org/)：`MAJOR.MINOR.PATCH`。
- **发布唯一触发器是 git tag `vX.Y.Z`**：CI 从 tag 取版本装配 npm 包族并发布
  （见 `.github/workflows/ci.yml`）。`Cargo.toml` 的 `version` 必须在打 tag 前的
  发布准备提交中与 tag 对齐——tag 是对外的最终真值。
- npm 包版本由 tag 经 `npm/build.sh` 装配生成，顶层包与六个平台子包及
  `optionalDependencies` 版本钉自动同步（模板 `npm/cli/package.json` 中的版本号
  仅为占位，不参与发布）。

### 1.0.0 之前（当前阶段）

| 变更类型 | 版本动作 | 说明 |
|---|---|---|
| 新功能 / 破坏性变更（含移除子命令、改参数语义） | MINOR +1 | 0.x 阶段 SemVer 允许 MINOR 携带破坏性变更 |
| 缺陷修复、文档、无接口变化的内部改进 | PATCH +1 | |

### 1.0.0 之后

- 破坏性变更必须 MAJOR +1。
- 移除子命令/参数须提前**一个 MINOR 版本**在 `--help` 与 CHANGELOG 中标注废弃，
  在下一个 MAJOR 中移除。

### 破坏性变更的迁移豁免

若被移除入口的**替代能力在同一版本内提供**（如某独立命令被新的流水线命令吸收），
可免除废弃过渡期，但 CHANGELOG 必须给出迁移映射表。
（实例：0.2.0 移除某 PDF 转换入口，同版本的全能转换命令 `markscan --mode speed`
为严格超集替代。）

### 预发布与 dist-tag

- 预发布 tag：`vX.Y.Z-rc.N`；npm 仅对稳定版打 `latest`，RC 版不进 latest。
- 同版本重复发布会触发 CI 内的 "already published" 跳过逻辑，属预期行为。

## 2. 发布流程

1. `main` 分支 CI 全绿：`rust-check`（`cargo fmt --check` + `cargo check` +
   `cargo clippy --deny warnings` + `cargo test`）、`secret-scan`（提交历史密钥扫描，
   需 `fetch-depth: 0`）、`cross-target-check`（Windows/macOS/Linux-musl 目标类型检查），
   三个 job 全绿。
2. 发布准备提交：`Cargo.toml` 版本号 + CHANGELOG（`Unreleased` 段落固化为版本段）
   + 需要时的 README 更新。
3. 打 tag `vX.Y.Z` 并推送 tag → CI 自动：版本一致性校验 → **先创建 Release 并上传
   安装脚本与 `tests_data.zip`** → **六平台二进制追加进该 Release** → npm 包族
   provenance 发布（依赖 `NPM_TOKEN` secret）。
   顺序不可调换：`upload-rust-binary-action` 要求 Release **已存在**才能追加资产，
   建 Release 的动作必须在前。
4. 执行发布后验证清单（§3）。

## 3. 发布条件清单

状态标注（2026-10-23）：✅ 完成｜🔄 进行中｜⬜ 未开始

| # | 条件 | 状态 | 说明 |
|---|---|---|---|
| C1 | LICENSE 文件与全部声明一致 | ✅ | 此前仓库无 LICENSE 文件而 README/Cargo/npm 均声明 Apache-2.0，已补齐文本 |
| C2 | npm 包族版本一致性 | ✅ | `build.sh` 现同步改写 `optionalDependencies` 版本钉（已用测试版本实测） |
| C3 | CHANGELOG 建立 | ✅ | 见根目录 `CHANGELOG.md` |
| C4 | 版本与发布制度文档 | ✅ | 即本文档 |
| C5 | markscan 阶段②~⑤落地并真实网关验证 | ✅ | 五阶段全部落地（2026-10-18，提交链 df95251→…→81331ba，含图像/HTML 通道 631c0f3），真实网关验证通过；契约见 `tests/docs/QYZS-MARKSCAN.md` |
| C6 | 骨架审美重构完成且测试全绿 | ✅ | dispatch 按域拆分/thiserror/models 数据驱动（81331ba）；CI 离线 284 项全绿零警告（240 单元 + 43 集成/回归 + 1 网关离线守卫；另 3 项真实网关调用用例按 `#[ignore]` 门控跳过） |
| C7 | 提交历史中无 API Key 记录 | ✅ | **责任边界**：本项目负责「凭据不进入 git 历史」，历史中若曾出现则须重写历史；已在 key 平台侧吊销由运营方负责，不计入本项目发布门槛。核查与门禁：`tests/scripts/check_no_secrets.sh` 扫描**全对象库**（含可达提交与悬空/不可达对象——`git add` 后又删除的凭据同样会留在对象库里）与已跟踪文件，CI 以 `secret-scan` job 逐次拦截（需 `fetch-depth: 0`）。当前实测：`.env` 从未进入提交，真实 Key 在对象库中零命中 |
| C8 | `NPM_TOKEN` secret 配置 + npm provenance 流水线端到端实跑 | ⬜ | 0.1.0 从未打 tag，发布链路一次都没跑过。**本地已就绪**：`tests/scripts/verify_release_pipeline.py` 验证包族装配、顶层与六个平台包版本钉同步、启动器平台调度三方一致（19/19 通过）。仍需 `NPM_TOKEN` 与真实 `npm publish` |
| C9 | 六平台二进制首次真实发布并抽查安装 | ⬜ | install.sh / install.ps1 / npm 三种方式各至少一台真实机器。**本地已就绪**：模拟 Release 全链路验证 25/25 通过——install.sh 查版本→下载→解包→安装→可执行；install.ps1 经 pwsh 在 Linux 上验证 URL 构造→下载→Expand-Archive→安装→PATH 注入（Windows 专属的重命名替换仍须真机）；`upgrade --package` 本地安装包路径；`qyzs dataset` 解压落点；npm 启动器实际调度到正确平台包。CI 以 `release-pipeline-verify` job 逐次拦截 |
| C10 | `qyzs upgrade` 自更新链路实测 | ⬜ | 依赖 C9 的已发布版本。**本地已就绪**：模拟 Release 验证 `upgrade --check` 版本比较、`upgrade` 下载→解包→原子替换自身（替换后二进制可执行、权限 0755 保持）。另修复一处阻塞缺陷——自更新此前只解 `.zip`，Linux/macOS 的 `.tar.gz` 资产会被原样写入导致二进制损坏 |
| C11 | CI 版本一致性守卫 | ✅ | tag 触发时校验 tag == Cargo.toml 版本，不符即失败；数据集 tests_data.zip 已纳入 Release 资产打包。`Cargo.toml` 版本与 `qyzs dataset` 默认 URL 现均指向 v0.2.0，首个 tag 推送后该 URL 生效 |
| C12 | 命令参考与 i18n 覆盖 | ✅ | README 子命令速览覆盖全 16 子命令；markscan/usage 运行时消息全量双语目录化（`msg!`/`msgf!`）。注：帮助文本与错误详情以中文为主，系 README「核心特性」明示的设计边界，不计入本项 |
| C13 | 1.0.0 门槛 | ⬜ | CLI 契约稳定承诺 + 连续两个版本的发布实践 + 安全审查通过（依赖 C8~C10；C7 已按「历史无凭据」口径达成） |

## 4. 版本历史锚点

- **0.1.0**（2026-10-17，提交 3cb3e1c）：初始全量 CLI——17 个子命令、能力边界
  探测固化（capability_matrix.toml）、客户端护栏（embeddings 分块/256 批拆批、
  chat 16 图分批）。注：仓库 git 历史曾被并行进程重建为单初始提交，0.1.0
  **从未打 tag 发布**。
- **0.2.0**（2026-10-23 发布准备完成，待打 tag）：markscan 全能文档转 Markdown
  流水线——五阶段全部落地并真实网关验证（自动识别格式 + pdf/docx/xls/xlsx/pptx/
  vsdx/csv/json/xml/zip/image/html 全通道 + **两档**流水线 rigor（默认）/speed
  （旧值 balanced 兼容接受并映射到 rigor）+ 图像/HTML 通道）；
  同版本新增 `usage` 本地用量统计子命令；移除被吸收的独立 PDF 转换入口，
  以及 `ocr`/`parse`/`markitdown` 三个已被 markscan 吸收的子命令
    （迁移豁免见 §1）。发布待 C8~C10 条件清零（C7「提交历史无凭据」已达成）。
