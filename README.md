# QYZS CLI (庆阳智算中心统一命令行工具)

[![CI](https://github.com/qyzs-ai/qyzs-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/qyzs-ai/qyzs-cli/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

自然资源部庆阳智算中心（国家级智算节点）官方命令行客户端（`qyzs`）。
基于 **纯 Rust** 打造，具备**零宿主动态库依赖（Zero Dynamic Dependency）**、**极小构建体积**、**跨平台/国产化架构原生支持**与**单二进制双形式兼容调用**。

---

## 核心特性

- **零外部动态依赖**：内置 Mozilla 根证书与 `rustls`，无需宿主系统 OpenSSL 或 C 动态库，拷到任意机器即装即用。
- **全平台多架构矩阵**：
  - **Linux**: `amd64` (x86_64), `arm64` (aarch64), **`loong64` (龙芯 loongarch64 全国产化支持)**；
  - **macOS**: `amd64` (Intel), `arm64` (Apple Silicon M系列)；
  - **Windows**: `amd64` (`x86_64-pc-windows-msvc`)。
- **命令行双形式兼容**：原生支持标准子命令 `qyzs <command>`，且内置 `argv[0]` 自动重写，无缝兼容 `qyzs-<command>`（如软链接调用）。
- **实测评分最优默认模型**：未指定 `--model` 时，自动装载经基准压测评定最高分的模型（对话默认 `dsv4` 1.0s、实测 51.8 万 Tokens 上下文 100% 召回；多图默认 `qwen3-VL` 16图并发；公文OCR默认 `paddleocrvl` 带800px防崩溃缩放；文档结构化默认 `mineru3.2.3`）。
- **双语环境自适应 (i18n)**：运行时状态/进度提示自动适配中英文（默认跟随系统语言：POSIX locale 变量优先，Windows 读取系统 UI 语言 API、macOS 读取 CoreFoundation 系统区域，均无法识别回退英文；`--lang zh|en` 显式切换，`QYZS_LANG` 环境变量同效），上游模型返回内容原样透传；帮助文本与错误详情以中文为主。
- **四层级联配置优先级**：`CLI 参数` > `环境变量 (QYZS_APIKEY / QYZS_BASE_URL / QYZS_TIMEOUT / QYZS_LANG / QYZS_MODEL_LANG)` > `配置文件 (~/.config/qyzs/)` > `代码基准真值`；敏感 API Key 严格采用 XDG 0600 安全隔离。注：`--color` 与 `--format` 仅有 CLI 参数与配置文件两层，无环境变量层。

---

## 安装方式

### 1. 跨平台一键脚本（推荐，安装即更新）

```bash
# Linux / macOS —— 装最新版
curl -fsSL https://raw.githubusercontent.com/qyzs-ai/qyzs-cli/main/install.sh | bash

# 装指定版本 (Release 里的 install.sh 已钉定到该 tag)
curl -fsSL https://github.com/qyzs-ai/qyzs-cli/releases/download/v0.2.0/install.sh | bash
```

```powershell
# Windows PowerShell —— 装最新版
irm https://raw.githubusercontent.com/qyzs-ai/qyzs-cli/main/install.ps1 | iex

# 装指定版本
irm https://github.com/qyzs-ai/qyzs-cli/releases/download/v0.2.0/install.ps1 | iex
```

> **版本钉定**：仓库 `main` 分支上的脚本装「当前最新」；Release 资产里的脚本钉定到
> 该 tag（发布时由 CI 注入版本号），不会出现「下载 v0.2.0 的脚本却装上 v0.3.0」。
> 也可用 `QYZS_VERSION=v0.2.0` 显式指定版本。
>
> **完整性校验**：Release 提供 `SHA256SUMS`，安装脚本下载后自动比对，不匹配即中止
> 安装（不留下任何产物）。手动安装可用 `sha256sum -c SHA256SUMS` 自查。

### 2. npm 全局安装

```bash
npm install -g @qyzs-ai/cli
```

### 3. 包管理器安装（各平台主流路径）

```powershell
# Windows —— winget
winget install QYZS-AI.qyzs
```

```bash
# macOS —— Homebrew
brew install qyzs        # 需先添加对应 tap
```

> 打包清单源在 [`packaging/`](packaging/)（winget / Homebrew），每次发版由
> `packaging/generate_manifests.py` 从 Release 的 `SHA256SUMS` 自动重算哈希，
> 避免手抄出错。

### 4. Cargo 源码构建

```bash
cargo install --path .
```

### 4. 镜像源 / 内网自建（可选）

安装脚本与自更新模块的下载地址可通过环境变量覆盖，便于走国内镜像源或内网自建
GitHub 兼容服务（**这四个变量不属于下面的「四层级联配置优先级」**——它们只作用于
安装与升级链路的地址，不参与 CLI 配置解析）：

| 变量 | 作用 | 默认值 |
|---|---|---|
| `QYZS_REPO` | 仓库标识 | `qyzs-ai/qyzs-cli` |
| `QYZS_GITHUB_URL` | Release 资产下载前缀 | `https://github.com/<repo>` |
| `QYZS_RELEASE_API_URL` | 最新版本查询接口（`qyzs upgrade` 亦生效） | GitHub Release API |
| `QYZS_FALLBACK_TAG` | 版本查询失败时的兜底 tag | `v0.2.0` |

```bash
# 示例: 走镜像源安装
QYZS_GITHUB_URL=https://mirror.example.com/qyzs-ai/qyzs-cli \
QYZS_RELEASE_API_URL=https://mirror.example.com/api/releases/latest \
bash install.sh
```

---

## 常用子命令速览

### 全局参数与短语法规范

所有全局参数置于子命令**前后均可**。短形式规范：`-h/-V` 为保留字；同义参数跨子命令统一字母（`--model` = `-m`、`--output` = `-o`）；不同含义允许复用字母（如 `-d` 在 `rerank` 为 documents、在 `dataset` 为 dest，`-p` 在 `pdf-split` 为 pages）；规范全文见 `src/cli/args.rs` 模块头注释。

| 短 | 长 | 说明 |
|---|---|---|
| `-b` | `--base-url` | 覆盖网关 Base URL |
| `-t` | `--timeout` | 请求超时秒数 |
| `-l` | `--lang` | 界面语言 `zh` \| `en`（默认跟随系统语言，无法识别回退 `en`） |
| `-c` | `--color` | 终端色彩 `auto` \| `always` \| `never` |
| `-f` | `--format` | 输出格式 `text` \| `json` \| `raw` |
| — | `--api-key` | 覆盖 API Key（无短形式：`-k` 已被 `auth login --key` 占用） |
| — | `--model-lang` | 模型输出语言约束 `auto` \| `cn` \| `zh` \| `zh-cn` \| `en`（仅 chat 生效；无短形式：`-m` 已被 `--model` 约定占用） |

### 退出码约定

脚本化调用可直接按退出码分流，无需解析 stderr 文案：

| 码 | 含义 | 典型触发场景 |
|---|---|---|
| `0` | 成功 | — |
| `1` | 通用失败 | 未归类的内部错误 |
| `2` | 用法错误 | 参数非法、枚举值不在白名单、数值越界、互斥参数同时出现 |
| `3` | 认证失败 | 缺少/无效 API Key |
| `4` | 请求超时 | 超过 `--timeout` 阈值 |
| `5` | IO 失败 | 本地文件读写失败、PDF 处理异常 |
| `69` | 服务不可达（`EX_UNAVAILABLE`） | 网络无法连接网关、握手失败 |
| `70` | 内部/服务端错误（`EX_SOFTWARE`） | 网关返回 5xx、响应体解析失败 |
| `78` | 配置错误（`EX_CONFIG`） | `config.toml` 格式错误、配置键非法 |

69 / 70 / 78 取自 BSD `sysexits.h`，便于脚本区分「网络问题」「服务端问题」「本地配置问题」；1~5 为本项目自定义语义码。

### 1. 运行诊断与模型可用矩阵

```bash
# 全链路运行诊断（本地环境 + 凭据 0600 安全审计 + 远程三网关延时探针）
qyzs status
qyzs status --json

# 仅查本地环境与凭据：跳过远程探针（离线秒回，JSON 的 probes 段为 null）
qyzs status --no-probe
qyzs status --no-probe --json

# 查看智算中心在线可用模型矩阵与实测延时
qyzs models
```

`--no-probe` 只输出本地环境、配置路径与凭据权限，跳过三个网关的网络往返（每端点 5 秒超时），并**不输出全局健康结论**——跳过探针不等于全链路正常。JSON 模式下 `probes` 字段为 `null`，便于脚本区分「未探测」与「探测失败」。

### 2. 本地用量统计 (`usage`)

```bash
# 本地网关用量台账: 请求次数与 Token 消耗 (chat/ocr/asr/embed/rerank 自动记录)
qyzs usage

# 只看最近 7 天 / 只看指定模型 / 指定调用族
qyzs usage --days 7
qyzs usage --model dsv4
qyzs usage --kind embed        # chat | ocr | asr | embed | rerank

# 按小时分桶 (UTC) / 指定凭据 (掩码指纹见 [按凭据] 段, 多凭据使用时自动展示)
qyzs usage --hourly
qyzs usage --key 'sk-***8816'

# 机器可读输出 / 清空台账
qyzs usage --json
qyzs usage --clear
```

> 台账为本地 JSONL（`$XDG_STATE_HOME/qyzs/usage.jsonl`，回退 `~/.local/state/qyzs/`），
> 仅统计本机发起的调用；流式 chat 经 `stream_options.include_usage` 获取 Token 数据
> （2026-10 实测网关支持）。MinerU/SAM 为任务型接口，无 Token 数据，不记录。
> 明细保留 30 天（UTC 日界），超期明细在 `usage` 调用时时机性压实为
> 日 × 模型 × 调用族 × 凭据 归档行（`--hourly` 模式以标注行单列；
> `--json` 以 `archived` 口径单列），压实幂等且带锁防并发。

### 3. 凭据与配置管理

```bash
# 方式一：环境变量（推荐，优先级：CLI 参数 > QYZS_APIKEY > 已保存凭据）
export QYZS_APIKEY=<your-api-key>

# 方式二：安全登录并保存 API Key (自动应用 0600 权限)
qyzs auth login --key "$QYZS_APIKEY"

# 查看当前生效 Key 掩码与安全性
qyzs auth status

# 读取或设置持久化配置
qyzs config get network.base_url
qyzs config set model.language zh-cn
qyzs config path
```

### 4. 对话与多模态推理 (`chat`)

```bash
# 纯文本对话 (默认调用 dsv4，开启流式打字机输出)
qyzs chat "请阐述《测试文献A》的关键要求。"

# 深度思维链推理 (调用 qwen3.6-27b，展示实时思考过程)
qyzs chat -m qwen3.6-27b --reasoning "核算以下建设用地指标与争议仲裁推演..."

# 多图并发视觉理解 (自动切换至 qwen3-VL-30B-A3B; 超过 16 张自动分批, 每批 ≤16)
qyzs chat -i p1.png -i p2.png -i p3.png "对比数张公文扫描件，指出印章盖章单位与签字页位置。"

# 采样与流式控制 (--stream/--no-stream 互斥; 默认流式)
qyzs chat --no-stream "生成一段结构化摘要。"
qyzs chat --temperature 0.2 --top-p 0.9 --max-tokens 2048 "写一份项目验收意见。"
```

### 5. 图像与文档转换 (`markscan`)

文档与图像的转换统一由 `markscan` 承担（原 `ocr` / `parse` 子命令已并入，详见第 7 节）：

```bash
# 图像转文字 (png/jpg/gif/bmp/tiff/webp; 内置 800px 等比缩放, 规避网关 2048 崩溃)
qyzs markscan 图.png -o 图.md --image-handling ocr

# 文档结构化转换 (类型识别 + 多模型抽取 + 交叉验证)
qyzs markscan document.pdf -o document.md
```

### 6. 语音识别与遥感变化检测 (`asr` & `sam`)

```bash
# 语音识别转文字 (默认 Qwen3-ASR-1.7B, 16kHz)
qyzs asr meeting.wav

# 遥感双时相大模型变化检测 (默认 acid-sam3-full, 支持 AOI 多边形与敏感度阈值)
qyzs sam --before before_2023.tif --after after_2024.tif --aoi aoi.geojson --threshold 0.5
```

### 7. 智能文档转换与 PDF 拆分 (`markscan` & `pdf-split`)

`markscan` 是唯一的文档转 Markdown 入口（取代此前的 `parse` 与 `markitdown`）。它对每个
输入先判定文档类型，再按类型特征走向应通道并做交叉验证：

```bash
# 先预览: 自动识别格式与流水线规划 (离线, 零模型调用)
qyzs markscan tender_doc.pdf --dry-run

# 常规转换: 本地/URL 自动识别格式 (pdf/docx/xls/xlsx/ppt/pptx/vsd/vsdx/图片/html),
# 超长 PDF 每 15 页切片聚合; 图片附件写入与 .md 同级的 assets/ 并在正文引用
qyzs markscan tender_doc.pdf -o tender.md

# 默认即对抗式全流程: 多通道抽取 + 交叉验证逐段仲裁 + 全局审查 (准确率优先)
# (--report 输出质量报告 JSON; --mode speed 可切批量快速通道)
qyzs markscan report.docx --report report.json

# 分类与排版: 识别公文/合同/标准规范/表格/演示, 按各自体裁重建层级
#   - 标准规范: 编>分编>章>节>条 编号层级重建 (这类文档字号统一, 层级只在编号里)
#   - 公文/法规: 编>章>节>条 与 一、>（一）>1.>（1） 双体系按文中实际出现的体裁自适应定级
#   - 合同/协议: 章/条 + （一）/1., 结构缺失时从编号补建标题
#   - DOCX: 直接读 word/styles.xml 的 Heading/标题 样式, 还原真实层级
#   - PDF: 另用字号/粗体识别文档标题与大字号标题 (编码损坏时样式仍有效)
qyzs markscan 国标.pdf -o 国标.md
qyzs markscan 合同.docx -o 合同.md

# 图片输入: 先判定是文档还是照片; 真实照片/插画按策略跳过 (不产出空文件)
qyzs markscan 扫描件.jpg -o 扫描件.md

# 体裁识别: 规则强信号直接定案; 弱信号/判不出时自动送模型仲裁; 也可指定模型先行
qyzs markscan 材料.pdf -o 材料.md                       # 强信号定案, 弱命中→dsv4 仲裁
qyzs markscan 材料.pdf -o 材料.md --classify-model qwen3.6-27b   # 模型优先

# 附件控制: 自定义目录 / 关闭抽取 / 关闭类型识别
qyzs markscan report.docx -o out/report.md --assets-dir out/imgs
qyzs markscan report.docx -o report.md --no-assets --no-classify

# 图像输入 (png/jpg/gif/bmp/tiff/webp 六种格式, 统一 800px 缩放预处理)
#   --image-handling: auto(默认, 文字稀少时补 VL 描述) | ocr | describe | both | skip
qyzs markscan 图.png -o 图.md --image-handling both
qyzs markscan 图.png -o 图.md --image-prompt "只输出图中的中文表格，逐格转录"
qyzs markscan 图.png -o 图.md --max-dim 1600        # 覆盖长边像素上限 (1~8192)

# 格式手动覆盖 (自动识别失败时): pdf|docx|xls|xlsx|ppt|pptx|vsd|vsdx|csv|json|xml|zip|image|html
qyzs markscan 无扩展名文件 --type pdf -o out.md

# 批量与目录输入 (目录默认只遍历顶层, --recursive 递归子目录)
qyzs markscan 资料目录/ -o 输出目录/
qyzs markscan 资料目录/ -o 输出目录/ --recursive

# 过程与工作区
qyzs markscan big.pdf -o big.md --work-dir ./wk     # 指定工作区根目录 (默认 ~/.cache/qyzs/markscan)
qyzs markscan big.pdf -o big.md --resume            # 断点续跑 (输入指纹一致才复用)
qyzs markscan big.pdf -o big.md --no-clean          # 成功后保留全部中间产物
qyzs markscan big.pdf -o big.md --segment-chars 12000   # 分段目标字符数 (1~100000)
qyzs markscan big.pdf -o big.md --min-confidence 0.8     # rigor 档交叉验证阈值 (0.0~1.0)
qyzs markscan big.pdf -o big.md --no-front-matter  # 不加 YAML front matter

# 提取指定页码范围另存为新 PDF (纯 Rust, 零系统 C 依赖)
qyzs pdf-split big.pdf --pages 1-3,5 --output split.pdf

# 按每 N 页分片整个文档到目录 (与 markscan 的长文档切片同路径)
qyzs pdf-split big.pdf --chunk-size 15 --out-dir ./slices
```

### 8. 语义表征与重排 (`embed` & `rerank`)

```bash
# 向量表征 (默认 bge-m3 1024 维)
qyzs embed "自然资源资产清查" "国土空间生态修复"

# 语义重排序精排 (默认 qwen3-reranker-8b, Top1置信度>0.90)
qyzs rerank -q "土地利用现状分类" -d "自然资源部规程..." -d "无关参考文本..." --top-n 3
```

### 9. 同源互验与运维工具 (`verify` / `dataset` / `upgrade` / `completions`)

```bash
# OCR 与 MinerU 解析结果同源互验仲裁 (字符 n-gram 相似度对账, 可识别错配)
qyzs verify --ocr ocr_result.txt --markdown parsed.md

# 从 GitHub 下载官方基准测试数据集 (默认解压到 tests/data; --url 换源)
qyzs dataset
qyzs dataset --url https://github.com/qyzs-ai/qyzs-cli/releases/download/v0.2.0/tests_data.zip

# 版本自更新 (GitHub Release 拉取最新二进制; --check 仅检查不更新)
qyzs upgrade
qyzs upgrade --check
qyzs upgrade --repo my-org/my-fork --package my-pkg

# 模型矩阵查询 (矩阵数据来自 capability_matrix.toml, 非在线探测)
qyzs models                # 仅在线可用模型 (默认即 --online-only)
qyzs models --all-models   # 追加列出实测禁止调度的已下线项

# 生成 Shell 补全脚本 (bash/zsh/fish/powershell)
qyzs completions bash
```

---

## 本地开发环境

在 Linux 开发机上复现六平台构建矩阵（交叉编译工具链、各目标的可行性边界、
质量门禁与实测脚本）：[docs/DEVENV.md](docs/DEVENV.md)

---

## 版本与发布

- 版本号制度、发布流程与发布条件清单：[docs/RELEASE.md](docs/RELEASE.md)
- 变更记录：[CHANGELOG.md](CHANGELOG.md)

---

## 许可证

本项目采用 [Apache-2.0](LICENSE) 许可证开源。
