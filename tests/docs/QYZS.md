# 自然资源部庆阳智算中心大模型权威接口规范与调用指南（QYZS）

> **文档标识**：QYZS-API-SPEC-2026-V1.0  
> **制定单位**：自然资源部信息中心 / 庆阳智算中心项目研发组  
> **适用范围**：庆阳智算中心全量可用大模型服务规范、真实端点寻址、实测参数及多语言接入参考（SSOT 事实标准）  
> **认证凭证**：`Authorization: Bearer $QYZS_APIKEY`（测试凭证经环境变量 `QYZS_APIKEY` 或仓库根目录 `.env` 文件注入，本文档已脱敏，不记载明文 Key）

---

## 1. 概述与网关架构

庆阳智算中心（国家级智算节点）为自然资源部国土空间基础信息平台提供统一的 AI 智能算力底座。经全量端点探针与经验校准，平台对外提供三组核心网关：

```
                    ┌────────────────────────────────────────────────────────┐
                    │  庆阳智算中心统一网关 (http://59.219.113.13:9200)         │
                    └──────────────────────────┬─────────────────────────────┘
                                               │
         ┌─────────────────────────────────────┼──────────────────────────────────┐
         │                                     │                                  │
         ▼                                     ▼                                  ▼
【OpenAI兼容公共网关】                【MinerU文档解析专线】              【遥感变化检测流水线】
/api/aiCenterChat/v1                  /ai-gateway/mineru3.2.3/file_parse  /api/aiCenterChat/v3/form
├── /chat/completions (文本/多模态/ASR)                                   └── /acid-sam3-full
├── /embeddings (向量表征)
└── /rerank (语义重排)
```

---

## 2. 真实可用模型矩阵与校准真值表

平台网关注册有 31 项模型配置，经全量端点探查，**实际可用高可靠模型清单如下**：

| 模型分类 | 官方注册代码 (`model`) | 状态 | 真实能力与特性 | 核心参数规范 | 推荐使用场景 |
| :--- | :--- | :---: | :--- | :--- | :--- |
| **通用对话推理** | `dsv4` | **ONLINE** | 极速响应（短文本 ~1.0s）；超长上下文实测 **51.8 万 Tokens（约 105 万字符）100% 针孔召回**，128s 返回，梯度内未见天花板 | `temperature`: **[0, 2]**（服务端硬校验，越界返回 400）；`max_tokens`: **1~65536 均接受** | 复杂政策审查、公文起草、知识库问答、长文档摘要 |
| **深度逻辑思考** | `qwen3.6-27b` | **ONLINE** | 思维链推理（输出至 `reasoning` 字段）；上下文实测 **≥13.8 万 Tokens 100% 针孔召回**（梯度内未触顶），但延迟极高（13.8 万 Tokens 约 237s，同规模约为 `dsv4` 的 10 倍延时）。**注意：`max_tokens` 须给足思维链预算（≥2048），过小预算会先耗尽推理 Tokens 导致 `content` 为空** | `temperature`: 推荐 0~2（实测 2.1~8.0 仍被接受，无严格上界校验）；`max_tokens`: **1~65536 均接受** | 业务指标计算、空间几何推演、多轮复杂争议仲裁 |
| **视觉多模态** | `qwen3-VL-30B-A3B` | **ONLINE** | 多图并发：4 图计数准确，8/16 图轻微漏数（-1），**16 图为可靠工作上限**；网关可承载 ≥48 图（13.2 MB，无 413），但 24 图以上输出质量劣化（32 图起输出为空）。纯文本通道上下文**硬上限 131,072 Tokens（服务端报错确认）**，实测 10.2 万 Tokens 通过 | `temperature`: 0~2 推荐；`max_tokens`: 1~65536 均接受 | 规划图审查、架构流程图推演、跨页多图对比核验 |
| **文档OCR** | `paddleocrvl` | **ONLINE** | 毫秒级版面文字转录（~1.9s）。**限制：需前置缩放至 $\le 800\text{px}$** | `temperature`: 0.0, 图像最长边 $\le 800\text{px}$ | 扫描件公文识别、公章文本提取、表格行级转录 |
| **智能文档解析** | `mineru3.2.3` | **ONLINE** | PDF/DOC/DOCX 一键结构化输出 Markdown，保留表格与大纲；同步解析实测 30 页（0.2 MB）7.7s 完成，梯度内无页数断点 | 需使用 `files` 多文件表单；同步通道实测 ≥30 页可用 | 政策汇编解析、招标文档结构化、图文分流提取 |
| **通用向量表征** | `bge-m3` | **ONLINE** | 1024 维多语言高密度向量表示，延时 ~0.27s；**单条输入实测上限 1.3 万~1.6 万字符**（13k 通过、16k 拒绝，约 8192 Token 硬限），**批量 ≥256 条/请求** | `input`: 字符串或数组；长文本需客户端分块 | 规划政策知识库检索、语义去重 |
| **超大向量表征** | `qwen3-embadding-8b` | **ONLINE** | 4096 维超高分辨率语义向量（**注意：代码拼写含 `embadding`**）；**单条输入实测上限 1.2 万~1.3 万字符**（12k 通过、13k 拒绝），**批量 ≥256 条/请求** | `input`: 文本，输出 4096 维浮点数组；长文本需分块 | 复杂专业术语匹配、跨法律跨政策精确对齐 |
| **标准中文向量** | `bge-large-zh-v1.5` | **ONLINE** | 1024 维中文优化向量表示，延时 ~0.28s；**单条输入实测上限约 500~800 字符**（500 通过、800 拒绝，512 Token 级短向量模型），**批量 ≥256 条/请求** | `input`: 短文本（≤约 500 字符），超限返回 500 | 中文政务公文专属索引 |
| **语义重排序** | `bge-reranker-v2-m3` | **ONLINE** | Cross-Encoder 语义深度重排，延时 ~0.30s；**候选集实测 ≥200 篇可用**（10 篇 0.37s → 200 篇 1.49s，Top1 分数稳定） | `query`, `documents`: 数组, `top_n`: 整数 | RAG 检索候选精排第一梯队 |
| **高性能重排** | `qwen3-reranker-8b` | **ONLINE** | 8B 参数大规模重排模型，高置信度打分（Top1 分数 > 0.90）；**候选集实测 ≥200 篇可用**（10 篇 0.42s → 200 篇 3.46s） | 同上；大候选集延迟约为 v2-m3 的 2.3 倍 | 复杂法律法规条款精确排序 |
| **语音识别转录** | `Qwen3-ASR-1.7B` | **ONLINE** | 16kHz 单声道语音直接转写，延时 ~0.89s | `type`: `audio_url`, 接受 Base64 编码 | 会议录音整理、外业现场语音转文字 |
| **遥感要素提取** | `acid-sam3-full` | **ONLINE** | 双时相高分遥感变化检测与大模型分割 | Form 传参，支持大文件切片上传 | 地表覆盖监测、违规用地早期预警 |

> [!NOTE]
> **参数实测口径（2026-10-17）**：上表「核心参数规范」列全部数值来自 `tests/scripts/limit_probe*.py` 多轮梯度探针实测（结果存于 `tests/scripts/limit_summary*.json`），非官方标称值。上下文上限以「大海捞针」（needle-in-a-haystack）针孔原样召回为通过标准；多图并发以样本图循环封装计量网关承载，计数准确性另行评估；向量/重排边界以 500 拒绝为上限信号。

> [!WARNING]
> **官方文档已知陷阱与离线模型预警**：
> 1. `qwen36`：后端服务连接超时（`500 Connection error`），**请勿路由**。
> 2. `deepseek_v3_2` 与 `qwen2.5-72b`：未在推理网关上架，返回 `404 未查询到模型配置`。
> 3. `text-embedding-v4`：官方文档所载模型不存在，返回 500。通用表征请路由至 `bge-m3`。
> 4. `mymodel-paddleocr-udhyfg`（Triton PaddleOCR）：网关路由至离线端点（`400 base_url_missing`）。公文 OCR 请统一使用 `paddleocrvl`。
> 5. `paddleocrvl` 上下文限制：上游 FastDeploy 推理服务设置 `max_model_len = 2048`，输入图像若超过 1000px 易导致视觉 Tokens 超限触发 500。客户端应自动将图像缩放到 800px 以内。

---

## 3. 详细接口调用规范

### 3.1 文本对话与推理接口 (`/chat/completions`)

- **URL**：`POST http://59.219.113.13:9200/api/aiCenterChat/v1/chat/completions`
- **Headers**：`Authorization: Bearer <API_KEY>`, `Content-Type: application/json`

#### 极速高精模式 (`dsv4`)
```json
{
  "model": "dsv4",
  "messages": [
    {"role": "system", "content": "你是由自然资源部庆阳智算中心驱动的公文审查助手。"},
    {"role": "user", "content": "请阐述《测试文献A》的关键要求。"}
  ],
  "temperature": 0.2,
  "max_tokens": 2048
}
```

#### 深度思维链推理模式 (`qwen3.6-27b`)
> [!NOTE]
> `qwen3.6-27b` 在生成过程中会将思考过程输出至 `message.reasoning` 字段中，客户端解析时需优先读取 `content`，若为空则从 `reasoning` 中回退提取。

---

### 3.2 视觉与多图并发接口 (`/chat/completions`)

- **URL**：`POST http://59.219.113.13:9200/api/aiCenterChat/v1/chat/completions`
- **支持模型**：`qwen3-VL-30B-A3B`（推荐多图）、`paddleocrvl`（推荐单页快速OCR）

#### 多图并发核验示例（支持 1~16 张图片）：
```json
{
  "model": "qwen3-VL-30B-A3B",
  "messages": [
    {
      "role": "user",
      "content": [
        {"type": "text", "text": "对比以下数张公文扫描件，指出印章盖章单位与签字页位置。"},
        {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0KGgo..."}},
        {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0KGgo..."}}
      ]
    }
  ],
  "temperature": 0.1,
  "max_tokens": 1000
}
```

---

### 3.3 文档结构化解析接口 (`MinerU 3.2.3`)

- **URL**：`POST http://59.219.113.13:9200/ai-gateway/mineru3.2.3/file_parse`
- **Headers**：`Authorization: Bearer <API_KEY>`
- **Content-Type**：`multipart/form-data`
- **表单字段规范**：
  - `files`：待解析的文件二进制流（**必须使用复数 `files`**，支持 `.pdf`, `.doc`, `.docx`）
  - `output_dir`：固定为 `"output"`
  - `is_ocr`：可选 `"true"` 或 `"false"`（扫描件推荐 `"true"`）

#### 响应结构定义：
```json
{
  "task_id": "cde08ab7-7541-41c7-a604-43c01e668636",
  "status": "completed",
  "results": {
    "SAMPLE-0056": {
      "md_content": "# 财政部关于印发《测试文献B》的通知\n\n..."
    }
  }
}
```

---

### 3.4 向量表征与语义重排接口

#### 向量表征 (`/embeddings`)
- **URL**：`POST http://59.219.113.13:9200/api/aiCenterChat/v1/embeddings`
- **模型选项**：`bge-m3`（1024 维）、`qwen3-embadding-8b`（4096 维）、`bge-large-zh-v1.5`（1024 维）
```json
{
  "model": "bge-m3",
  "input": "自然资源部国土空间规划实施监测网络"
}
```

#### 语义重排 (`/rerank`)
- **URL**：`POST http://59.219.113.13:9200/api/aiCenterChat/v1/rerank`
- **模型选项**：`bge-reranker-v2-m3`（推荐通用）、`qwen3-reranker-8b`（超高精度）
```json
{
  "model": "bge-reranker-v2-m3",
  "query": "政府采购评审专家随机抽取合规规定",
  "documents": [
    "《测试文献A》配套实施方案",
    "《测试文献B》明确规定应当在专家库中随机抽取",
    "自然资源三维立体时空数据库设计方案"
  ],
  "top_n": 2
}
```

---

### 3.5 语音转写接口 (`Qwen3-ASR-1.7B`)

- **URL**：`POST http://59.219.113.13:9200/api/aiCenterChat/v1/chat/completions`
- **音频规范**：16kHz 采样率、单声道（Mono）、16-bit PCM WAV，转为 Base64 编码。
```json
{
  "model": "Qwen3-ASR-1.7B",
  "messages": [
    {
      "role": "user",
      "content": [
        {
          "type": "audio_url",
          "audio_url": {
            "url": "data:audio/wav;base64,UklGRiQAAABXQVZFZm10..."
          }
        }
      ]
    }
  ]
}
```

---

## 4. 客户端接入参考代码

### 4.1 cURL 快速接入

```bash
# 1. 文本对话 (dsv4)
curl -X POST "http://59.219.113.13:9200/api/aiCenterChat/v1/chat/completions" \
  -H "Authorization: Bearer $QYZS_APIKEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "dsv4",
    "messages": [{"role": "user", "content": "你好，请自我介绍"}],
    "temperature": 0.2
  }'

# 2. MinerU 文档解析
curl -X POST "http://59.219.113.13:9200/ai-gateway/mineru3.2.3/file_parse" \
  -H "Authorization: Bearer $QYZS_APIKEY" \
  -F "files=@tests/data/mineru/public_doc/SAMPLE-0106_测试审查表.docx" \
  -F "output_dir=output"
```

### 4.2 Python SDK 封装

```python
import os
import requests

API_KEY = os.getenv("QYZS_APIKEY", "")
BASE_URL = "http://59.219.113.13:9200/api/aiCenterChat/v1"

def qyzs_chat(prompt: str, model: str = "dsv4") -> str:
    headers = {"Authorization": f"Bearer {API_KEY}", "Content-Type": "application/json"}
    payload = {
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0.2
    }
    resp = requests.post(f"{BASE_URL}/chat/completions", headers=headers, json=payload, timeout=30)
    resp.raise_for_status()
    msg = resp.json()["choices"][0]["message"]
    return msg.get("content") or msg.get("reasoning") or ""

def qyzs_parse_document(file_path: str) -> str:
    url = "http://59.219.113.13:9200/ai-gateway/mineru3.2.3/file_parse"
    fname = os.path.splitext(os.path.basename(file_path))[0]
    with open(file_path, "rb") as f:
        files = {"files": (os.path.basename(file_path), f)}
        data = {"output_dir": "output"}
        resp = requests.post(url, headers={"Authorization": f"Bearer {API_KEY}"}, files=files, data=data, timeout=60)
        resp.raise_for_status()
        return resp.json().get("results", {}).get(fname, {}).get("md_content", "")
```

### 4.3 Rust 核心调用模式（`reqwest`）

```rust
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct ChatPayload<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    temperature: f32,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: Option<String>,
    reasoning: Option<String>,
}

pub async fn query_qyzs_dsv4(api_key: &str, prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {}", api_key))?);
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let payload = ChatPayload {
        model: "dsv4",
        messages: vec![ChatMessage { role: "user", content: prompt }],
        temperature: 0.2,
    };

    let resp = client.post("http://59.219.113.13:9200/api/aiCenterChat/v1/chat/completions")
        .headers(headers)
        .json(&payload)
        .send()
        .await?
        .json::<ChatResponse>()
        .await?;

    let msg = &resp.choices[0].message;
    Ok(msg.content.clone().or_else(|| msg.reasoning.clone()).unwrap_or_default())
}
```
