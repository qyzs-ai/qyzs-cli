//! 命令行参数与子命令定义 (clap 4.x derive)
//!
//! # 短语法规范 (本模块是参数面的代码 SSOT)
//!
//! 1. 保留字: `-h` (帮助) 与 `-V` (版本) 由 clap 占用, 不得挪用。
//! 2. 全局参数短形式对全部子命令生效, 字母须避开所有子命令级已用短形式。
//!    当前全局短形式: `-b` base-url / `-t` timeout / `-l` language / `-c` color /
//!    `-f` format。`--api-key` 与 `--model-lang` 无短形式: `-k` 已被 `auth login
//!    --key` 占用, `-m` 已被子命令级 `--model` 约定占用, 且两者均低频。
//! 3. 子命令级同义参数跨命令统一字母: `--model` 统一 `-m`, `--output` 统一 `-o`;
//!    不同含义允许复用同一字母 (作用域隔离, clap 惯例), 如 `-p` 在 ocr 为 prompt、
//!    在 pdf-split 为 pages。
//! 4. 短形式只增不改不删 (0.x 阶段兼容承诺); 高频参数应有短形式, 低频参数可只留长形式。

use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Debug, Parser)]
#[command(
    name = "qyzs",
    author = "Qingyang AI Computing Center Team <dev@qyzs.ai>",
    version,
    about = "自然资源部庆阳智算中心大模型统一命令行工具",
    long_about = "自然资源部庆阳智算中心统一命令行工具 (qyzs)，提供全量模型调用、公文与大文档结构化解析、同源互验与跨平台自更新能力。",
    after_help = "常用流程:\n  qyzs status                          # 运行环境、凭据与三网关自检\n  qyzs auth login                      # 安全保存 API Key (0600)\n  qyzs chat \"请综述这份材料...\"         # 对话推理 (流式打字机)\n  qyzs models                          # 查看实测可用模型矩阵\n  qyzs markitdown 文档.pdf --dry-run   # 全能转 Markdown: 预览识别与流水线规划\n  qyzs markitdown 文档.pdf -o out.md   # 全能转 Markdown: 执行转换"
)]
pub struct Cli {
    // --api-key 无短形式: -k 已被 auth login --key 占用 (见模块头短语法规范)
    #[arg(long, global = true, help = "覆盖 API Key 认证凭证")]
    pub api_key: Option<String>,

    #[arg(short = 'b', long, global = true, help = "覆盖庆阳智算中心网关 Base URL")]
    pub base_url: Option<String>,

    #[arg(short = 't', long, global = true, help = "请求超时秒数")]
    pub timeout: Option<u64>,

    // 默认取系统界面语言 (QYZS_LANG / locale / Windows 系统语言), 无法识别回退英文; --lang 为兼容别名
    #[arg(short = 'l', long = "language", alias = "lang", global = true, help = "界面语言: zh (中文) | en (英文) (默认: 系统语言, 无法识别回退 en)")]
    pub language: Option<String>,

    // --model-lang 无短形式: -m 已被子命令级 --model 约定占用 (见模块头短语法规范)
    #[arg(long, global = true, help = "模型输出语言约束: auto | cn | zh-cn | en")]
    pub model_lang: Option<String>,

    #[arg(short = 'c', long, global = true, help = "终端色彩输出: auto | always | never")]
    pub color: Option<String>,

    #[arg(short = 'f', long, global = true, help = "输出格式: text | json | raw")]
    pub format: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    #[command(about = "纯文本与多模态对话推理 (默认: dsv4 / 多图: qwen3-VL)")]
    Chat {
        #[arg(help = "提问提示词内容")]
        prompt: String,

        #[arg(short, long, help = "指定调用模型 (chat 严格排除 paddleocrvl)")]
        model: Option<String>,

        #[arg(short, long, help = "System 角色系统提示词")]
        system: Option<String>,

        #[arg(long, help = "采样温度 (0.0~2.0)")]
        temperature: Option<f64>,

        #[arg(long, help = "最大生成 Token 数量")]
        max_tokens: Option<u32>,

        #[arg(long, help = "Top-P 核采样概率")]
        top_p: Option<f64>,

        #[arg(long, default_value_t = true, help = "启用打字机流式输出 (默认开启)")]
        stream: bool,

        #[arg(long, help = "禁用流式输出")]
        no_stream: bool,

        #[arg(long, default_value_t = true, help = "是否展示 qwen3.6-27b 的思维链思考过程")]
        reasoning: bool,

        #[arg(short, long, help = "附带图像路径 (支持同时传入 1~16 张图片)")]
        image: Vec<String>,
    },

    #[command(about = "公文 OCR 与单页版面文字提取 (默认: paddleocrvl，自动800px预处理)")]
    Ocr {
        #[arg(help = "待识别图片路径")]
        image: String,

        #[arg(short, long, help = "自定义 OCR 引导提示词")]
        prompt: Option<String>,

        #[arg(long, help = "限制长边最大像素尺寸 (默认 800px 防越界)")]
        max_dim: Option<u32>,
    },

    #[command(about = "PDF/Word 文档结构化解析转 Markdown (默认: mineru3.2.3 专线)")]
    Parse {
        #[arg(help = "待解析文档文件路径 (.pdf, .doc, .docx)")]
        file: String,

        #[arg(long, default_value_t = true, help = "是否开启内部 OCR 模式")]
        is_ocr: bool,
    },

    #[command(about = "语义向量表征生成 (默认: bge-m3 1024维 / 备选: qwen3-embadding 4096维)")]
    Embed {
        #[arg(help = "待生成向量的文本内容")]
        text: Vec<String>,

        #[arg(short, long, help = "指定向量模型")]
        model: Option<String>,
    },

    #[command(about = "语义深度重排序 (默认: qwen3-reranker-8b, Top1置信度>0.90)")]
    Rerank {
        #[arg(short, long, help = "查询 Query")]
        query: String,

        #[arg(short, long, help = "候选文本 (支持多次指定 -d doc1 -d doc2)")]
        document: Vec<String>,

        #[arg(long, help = "返回置信度最高的前 N 项", short = 'n')]
        top_n: Option<usize>,

        #[arg(short, long, help = "指定重排模型")]
        model: Option<String>,
    },

    #[command(about = "语音识别转文字 (默认: Qwen3-ASR-1.7B, 16kHz)")]
    Asr {
        #[arg(help = "音频文件路径 (.wav, .mp3, etc.)")]
        audio: String,

        #[arg(short, long, help = "指定 ASR 模型")]
        model: Option<String>,
    },

    #[command(about = "遥感双时相大模型变化检测流水线 (默认: acid-sam3-full)")]
    Sam {
        #[arg(long, help = "前时相遥感影像文件路径")]
        before: String,

        #[arg(long, help = "后时相遥感影像文件路径")]
        after: String,

        #[arg(long, help = "AOI 多边形 GeoJSON 字符串或文件路径")]
        aoi: Option<String>,

        #[arg(long, help = "变化检测判定敏感度阈值 (0.0~1.0)")]
        threshold: Option<f64>,
    },

    #[command(about = "PDF 纯 Rust 页面拆分与分块提取 (基于 lopdf，零系统 C 依赖)")]
    PdfSplit {
        #[arg(help = "待拆分 PDF 原始文件路径")]
        file: String,

        #[arg(short, long, help = "提取指定页码范围 (如 '1-3,5,7-9')")]
        pages: Option<String>,

        #[arg(short, long, help = "拆分后的独立 PDF 输出路径")]
        output: Option<String>,

        #[arg(long, help = "将整个 PDF 按每 N 页分块切片并保存到目录")]
        chunk_size: Option<usize>,

        #[arg(long, help = "分块输出保存目录")]
        out_dir: Option<String>,
    },

    #[command(about = "OCR 与 MinerU 解析结果同源互验仲裁")]
    Verify {
        #[arg(long, help = "OCR 纯文本或包含识别结果的文件路径")]
        ocr: String,

        #[arg(long, help = "MinerU 结构化 Markdown 文件路径")]
        markdown: String,
    },

    #[command(about = "全能文档转 Markdown 流水线 (自动识别格式 + 多通道抽取 + 交叉验证排版)")]
    Markitdown {
        #[arg(help = "输入: 本地文件路径或 http(s):// 在线地址 (自动识别格式)")]
        input: String,

        #[arg(short, long, help = "输出 Markdown 文件保存路径 (默认输出到 stdout)")]
        output: Option<String>,

        #[arg(long, help = "流水线档位: speed (单通道+规则清洗) | balanced (+LLM润色+自检) | rigor (双通道+交叉验证+仲裁) (默认 balanced)")]
        mode: Option<String>,

        #[arg(long, help = "手动指定输入格式覆盖自动识别 (pdf|docx|xls|xlsx|ppt|pptx|vsd|vsdx|image|html)")]
        r#type: Option<String>,

        #[arg(long, help = "图像内容处理策略: auto | ocr | describe | both | skip (默认 auto)")]
        image_handling: Option<String>,

        #[arg(short = 'm', long, help = "理解/润色阶段 LLM 覆盖 (默认: dsv4)")]
        model: Option<String>,

        #[arg(long, help = "交叉验证一致性阈值 0.0~1.0 (默认 0.70, 实测正确段落对文本层相似度 0.57~0.75; 仅 rigor 档生效)")]
        min_confidence: Option<f64>,

        #[arg(long, help = "输出质量报告 JSON 路径 (检测格式/各通道置信度/冲突清单/轮次耗时)")]
        report: Option<String>,

        #[arg(long, help = "工作目录 (默认 ~/.cache/qyzs/markitdown/<输入内容哈希>/)")]
        work_dir: Option<String>,

        #[arg(long, default_value_t = 24_000, help = "章节分段目标字符数 (0 = 禁用结构分段, 仅按切片边界分段)")]
        segment_chars: usize,

        #[arg(long, default_value_t = false, help = "从上次中断的工作目录断点续跑 (输入内容一致时生效)")]
        resume: bool,

        #[arg(long, default_value_t = false, help = "成功后也保留全部中间产物 (默认成功即流式清理, 失败自动保留)")]
        no_clean: bool,

        #[arg(long, default_value_t = false, help = "仅输出格式识别结果与规划流水线, 不执行转换")]
        dry_run: bool,
    },

    #[command(about = "从 GitHub 官方仓库下载并准备基准测试数据集")]
    Dataset {
        #[arg(long, default_value = "https://github.com/qyzs-ai/qyzs-cli/releases/download/v0.2.0/tests_data.zip", help = "数据集下载地址")]
        url: String,

        #[arg(short, long, default_value = "tests/data", help = "数据集解压保存目录")]
        dest: String,
    },

    #[command(about = "客户端全链路运行环境、凭据与远程三网关探活诊断")]
    Status {
        #[arg(long, help = "以 JSON 格式输出诊断结果")]
        json: bool,
    },

    #[command(about = "配置文件持久化管理 (~/.config/qyzs/config.toml)")]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    #[command(about = "API Key 凭据安全存储与管理 (0600 XDG 规范)")]
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },

    #[command(about = "查看庆阳智算中心真实可用模型矩阵与性能指标")]
    Models {
        #[arg(long, default_value_t = true, help = "仅显示经实测在线 (ONLINE) 的高可用模型")]
        online_only: bool,
    },

    #[command(about = "本地网关用量统计 (请求次数与 Token 消耗台账)")]
    Usage {
        #[arg(long, help = "仅统计最近 N 天 (含今天, 按 UTC 日界)")]
        days: Option<u32>,

        #[arg(long, help = "仅统计指定模型 (如 dsv4, bge-m3)")]
        model: Option<String>,

        #[arg(long, help = "以 JSON 格式输出统计结果")]
        json: bool,

        #[arg(long, help = "清空本地用量台账后退出")]
        clear: bool,
    },

    #[command(about = "版本自更新与在线升级 (支持安装即更新)")]
    Upgrade {
        #[arg(long, default_value = "qyzs-ai/qyzs-cli", help = "GitHub 仓库标识")]
        repo: String,

        #[arg(long, help = "若本地已有安装包，可直接从本地包安装/更新")]
        package: Option<String>,
    },

    #[command(about = "生成 Shell 自动补全脚本 (Bash / Zsh / Fish / PowerShell)")]
    Completions {
        #[arg(help = "目标 Shell 类型")]
        shell: Shell,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    #[command(about = "获取当前配置项内容")]
    Get {
        #[arg(help = "配置项键路径 (如 'network.base_url', 'model.default_chat')")]
        key: String,
    },
    #[command(about = "持久化设置配置项")]
    Set {
        #[arg(help = "配置项键路径")]
        key: String,
        #[arg(help = "配置项取值")]
        value: String,
    },
    #[command(about = "查看配置文件与凭据文件所在绝对路径")]
    Path,
    #[command(about = "重置配置文件为初始默认值")]
    Reset,
}

#[derive(Debug, Subcommand)]
pub enum AuthAction {
    #[command(about = "登录并安全存储 API Key (Unix 下自动设置 0600)")]
    Login {
        #[arg(short, long, help = "API Key (未提供时依次读取环境变量 QYZS_APIKEY 与已保存凭据)")]
        key: Option<String>,
    },
    #[command(about = "查看当前生效 API Key 掩码与凭据文件权限安全性")]
    Status,
    #[command(about = "清除本地保存的凭据文件")]
    Logout,
}
