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
（实例：0.2.0 移除某 PDF 转换入口，同版本的全能转换命令 `markitdown --mode speed`
为严格超集替代。）

### 预发布与 dist-tag

- 预发布 tag：`vX.Y.Z-rc.N`；npm 仅对稳定版打 `latest`，RC 版不进 latest。
- 同版本重复发布会触发 CI 内的 "already published" 跳过逻辑，属预期行为。

## 2. 发布流程

1. `main` 分支 CI 绿（离线 `cargo check` + `cargo test`）。
2. 发布准备提交：`Cargo.toml` 版本号 + CHANGELOG（`Unreleased` 段落固化为版本段）
   + 需要时的 README 更新。
3. 打 tag `vX.Y.Z` 并推送 tag → CI 自动：版本一致性校验、六平台二进制上传
   Release、安装脚本与 tests_data.zip 数据集上传、npm 包族 provenance 发布
   （依赖 `NPM_TOKEN` secret）。
4. 执行发布后验证清单（§3）。

## 3. 发布条件清单

状态标注（2026-10-18）：✅ 完成｜🔄 进行中｜⬜ 未开始

| # | 条件 | 状态 | 说明 |
|---|---|---|---|
| C1 | LICENSE 文件与全部声明一致 | ✅ | 此前仓库无 LICENSE 文件而 README/Cargo/npm 均声明 Apache-2.0，已补齐文本 |
| C2 | npm 包族版本一致性 | ✅ | `build.sh` 现同步改写 `optionalDependencies` 版本钉（已用测试版本实测） |
| C3 | CHANGELOG 建立 | ✅ | 见根目录 `CHANGELOG.md` |
| C4 | 版本与发布制度文档 | ✅ | 即本文档 |
| C5 | markitdown 阶段②~⑤落地并真实网关验证 | ✅ | 五阶段全部落地（2026-10-18，提交链 df95251→…→81331ba，含图像/HTML 通道 631c0f3），真实网关验证通过；契约见 `tests/docs/QYZS-MARKITDOWN.md` |
| C6 | 骨架审美重构完成且测试全绿 | ✅ | dispatch 按域拆分/thiserror/models 数据驱动（81331ba）；CI 离线 83 单元 + 9 集成全绿零警告 |
| C7 | 安全门：历史泄露的 API Key 在平台侧吊销并复验失效 | ⬜ | **阻塞级**。旧 key 实测仍有效（2026-10-17 复验 200），须运营方吊销后用同款探针复验返回鉴权失败 |
| C8 | `NPM_TOKEN` secret 配置 + npm provenance 流水线端到端实跑 | ⬜ | 0.1.0 从未打 tag，发布链路一次都没跑过 |
| C9 | 六平台二进制首次真实发布并抽查安装 | ⬜ | install.sh / install.ps1 / npm 三种方式各至少一台真实机器 |
| C10 | `qyzs upgrade` 自更新链路实测 | ⬜ | 依赖 C9 的已发布版本 |
| C11 | CI 版本一致性守卫 | ✅ | tag 触发时校验 tag == Cargo.toml 版本，不符即失败；数据集 tests_data.zip 已纳入 Release 资产打包（`qyzs dataset` 默认 URL 指向 v0.2.0，首个 tag 发布后生效） |
| C12 | 命令参考与 i18n 覆盖 | ✅ | README 子命令速览覆盖全 18 子命令；markitdown/usage 运行时消息全量双语目录化（`msg!`/`msgf!`）。注：帮助文本与错误详情以中文为主，系 README「核心特性」明示的设计边界，不计入本项 |
| C13 | 1.0.0 门槛 | ⬜ | CLI 契约稳定承诺 + 连续两个版本的发布实践 + 安全审查通过（依赖 C7-C10） |

## 4. 版本历史锚点

- **0.1.0**（2026-10-17，提交 3cb3e1c）：初始全量 CLI——17 个子命令、能力边界
  探测固化（capability_matrix.toml）、客户端护栏（embeddings 分块/256 批拆批、
  chat 16 图分批）。注：仓库 git 历史曾被并行进程重建为单初始提交，0.1.0
  **从未打 tag 发布**。
- **0.2.0**（开发完成，未打 tag）：markitdown 全能文档转 Markdown 流水线——
  五阶段全部落地并真实网关验证（自动识别格式 + pdf/docx/xls/xlsx/pptx/vsdx/
  image/html 全通道 + 三档流水线 speed/balanced/rigor + 图像/HTML 通道）；
  同版本新增 `usage` 本地用量统计子命令；移除被吸收的独立 PDF 转换入口
  （迁移豁免见 §1）。发布待 C7~C10 条件清零。
