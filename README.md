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
  - **Windows**: `amd64` (`x86_64-pc-windows-gnu`)。
- **命令行双形式兼容**：原生支持标准子命令 `qyzs <command>`，且内置 `argv[0]` 自动重写，无缝兼容 `qyzs-<command>`（如软链接调用）。
- **实测评分最优默认模型**：未指定 `--model` 时，自动装载经基准压测评定最高分的模型（对话默认 `dsv4` 1.0s、实测 51.8 万 Tokens 上下文 100% 召回；多图默认 `qwen3-VL` 16图并发；公文OCR默认 `paddleocrvl` 带800px防崩溃缩放；文档结构化默认 `mineru3.2.3`）。
- **双语环境自适应 (i18n)**：运行时状态/进度提示自动适配中英文（默认跟随系统语言：POSIX locale 变量优先，Windows 读取系统 UI 语言 API、macOS 读取 CoreFoundation 系统区域，均无法识别回退英文；`--lang zh|en` 显式切换，`QYZS_LANG` 环境变量同效），上游模型返回内容原样透传；帮助文本与错误详情以中文为主。
- **四层级联配置优先级**：`CLI 参数` > `环境变量 (QYZS_*)` > `配置文件 (~/.config/qyzs/)` > `代码基准真值`；敏感 API Key 严格采用 XDG 0600 安全隔离。

---

## 安装方式

### 1. 跨平台一键脚本（推荐，安装即更新）

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/qyzs-ai/qyzs-cli/main/scripts/install.sh | bash
```

```powershell
# Windows PowerShell
irm https://raw.githubusercontent.com/qyzs-ai/qyzs-cli/main/scripts/install.ps1 | iex
```

### 2. npm 全局安装

```bash
npm install -g @qyzs-ai/cli
```

### 3. Cargo 源码构建

```bash
cargo install --path .
```

---

## 常用子命令速览

### 全局参数与短语法规范

所有全局参数置于子命令**前后均可**。短形式规范：`-h/-V` 为保留字；同义参数跨子命令统一字母（`--model` = `-m`、`--output` = `-o`）；不同含义允许复用字母（如 `-p` 在 `ocr` 为 prompt、在 `pdf-split` 为 pages）；规范全文见 `src/cli/args.rs` 模块头注释。

| 短 | 长 | 说明 |
|---|---|---|
| `-b` | `--base-url` | 覆盖网关 Base URL |
| `-t` | `--timeout` | 请求超时秒数 |
| `-l` | `--lang` | 界面语言 `zh` \| `en`（默认跟随系统语言，无法识别回退 `en`） |
| `-c` | `--color` | 终端色彩 `auto` \| `always` \| `never` |
| `-f` | `--format` | 输出格式 `text` \| `json` \| `raw` |
| — | `--api-key` | 覆盖 API Key（无短形式：`-k` 已被 `auth login --key` 占用） |
| — | `--model-lang` | 模型输出语言约束 `auto` \| `cn` \| `zh` \| `zh-cn` \| `en`（无短形式：`-m` 已被 `--model` 约定占用） |

### 1. 运行诊断与模型可用矩阵

```bash
# 全链路运行诊断（本地环境 + 凭据 0600 安全审计 + 远程三网关延时探针）
qyzs status
qyzs status --json

# 查看智算中心全量高可靠模型矩阵与实测延时
qyzs models
```

### 2. 本地用量统计 (`usage`)

```bash
# 本地网关用量台账: 请求次数与 Token 消耗 (chat/ocr/asr/embed/rerank 自动记录)
qyzs usage

# 只看最近 7 天 / 只看指定模型
qyzs usage --days 7
qyzs usage --model dsv4

# 机器可读输出 / 清空台账
qyzs usage --json
qyzs usage --clear
```

> 台账为本地 JSONL（`$XDG_STATE_HOME/qyzs/usage.jsonl`，回退 `~/.local/state/qyzs/`），
> 仅统计本机发起的调用；流式 chat 经 `stream_options.include_usage` 获取 Token 数据
> （2026-10 实测网关支持）。MinerU/SAM 为任务型接口，无 Token 数据，不记录。

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

# 多图并发视觉理解 (自动切换至 qwen3-VL-30B-A3B，支持 1~16 张图片)
qyzs chat -i p1.png -i p2.png -i p3.png "对比数张公文扫描件，指出印章盖章单位与签字页位置。"
```

### 5. 公文文字 OCR 与结构化解析 (`ocr` & `parse`)

```bash
# 公文 OCR 版面文字提取 (paddleocrvl，内置长边 <= 800px 自动等比缩放，规避 2048 崩溃)
qyzs ocr scan.png

# 复杂公文结构化解析转 Markdown (MinerU 3.2.3 专线，完整保留标题大纲与复杂表格)
qyzs parse document.pdf
```

### 6. 语音识别与遥感变化检测 (`asr` & `sam`)

```bash
# 语音识别转文字 (默认 Qwen3-ASR-1.7B, 16kHz)
qyzs asr meeting.wav

# 遥感双时相大模型变化检测 (默认 acid-sam3-full, 支持 AOI 多边形与敏感度阈值)
qyzs sam --before before_2023.tif --after after_2024.tif --aoi aoi.geojson --threshold 0.5
```

### 7. 全能文档转 Markdown 与 PDF 拆分 (`markitdown` & `pdf-split`)

```bash
# 先预览: 自动识别格式与流水线规划 (离线, 零模型调用)
qyzs markitdown tender_doc.pdf --dry-run

# 全能转换: 本地/URL 自动识别格式 (pdf/docx/xls/xlsx/pptx/vsdx/图片/html),
# 超长 PDF 每 15 页切片聚合, 规则清洗只做结构归一
qyzs markitdown tender_doc.pdf -o tender.md

# 严谨档: 双通道抽取 + 交叉验证逐段仲裁 + 全局审查 (--report 输出质量报告 JSON)
qyzs markitdown report.docx --mode rigor --report report.json

# 提取指定页码范围另存为新 PDF (纯 Rust, 零系统 C 依赖)
qyzs pdf-split --file big.pdf --pages 1-3,5 --output split.pdf
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
# OCR 与 MinerU 解析结果同源互验仲裁 (Jaccard 相似度对账)
qyzs verify --ocr ocr_result.txt --markdown parsed.md

# 从 GitHub 下载官方基准测试数据集 (默认解压到 tests/data)
qyzs dataset

# 版本自更新 (GitHub Release 拉取最新二进制) / 生成 Shell 补全脚本
qyzs upgrade
qyzs completions bash
```

---

## 版本与发布

- 版本号制度、发布流程与发布条件清单：[docs/RELEASE.md](docs/RELEASE.md)
- 变更记录：[CHANGELOG.md](CHANGELOG.md)

---

## 许可证

本项目采用 [Apache-2.0](LICENSE) 许可证开源。
