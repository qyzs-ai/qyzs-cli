# Changelog

本项目所有对外可见的变更记录于此。格式遵循
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [SemVer 2.0.0](https://semver.org/)；
版本制度、发布流程与发布条件见 [docs/RELEASE.md](docs/RELEASE.md)。

## [Unreleased]

### Added

- **`scan` 体裁分类规则修正（语料实测驱动）**：用 273 份带目录标签的语料做离线审计，
  原规则总体准确率仅 **65%**，修正后达 **93%**，叠加模型仲裁达 **97%**。四处根因：
  - **附件误判（13 份公文被抢成标准）**：`行业标准`/`GB/T ` 等是主题词不是体裁标记，
    公文「引用」GB/T 22239 极其常见。现拆为强/弱标记：强标记（`中华人民共和国国家标准`/
    `ICS`/`DB53/T`）直接定案；弱标记须配完整标准编号形（`GB/T \d{3,5}`）且只给弱置信。
  - **法律漏判（17/20 部法律落 generic）**：`附则` 字面匹配失配法律排版的「附　则」（全角
    空格）；且条款骨架要求另含一个公文标记词，而法律永远不含「条例」二字。现附则容空白
    （`附\s{0,3}则`），三条骨架（第一条/第二条/第三条）齐备即免标记词。
  - **红头漏判**：文种词尾此前只认「文件/通知」，补 函/意见/决定/公告/批复/纪要。
  - **骨架蹭票**：`范围` 改为行首编号形（`1 范围` / `2 规范性引用文件`），杜绝正文
    「一、适用范围」命中。
- **`scan` 弱置信命中送模型仲裁**：此前模型兜底仅在规则判出 Generic 时触发——只能救
  假阴性，**永远救不了假阳性**（被误判为标准的公文根本到不了模型）。现按置信度分派：
  强信号规则定案；弱信号（弱标记+编号、单层骨架）送模型仲裁，模型改判则采纳。实测 13
  份弱命中中文档模型中 12 份纠正为 official。判定路径在 INFO 行明示
  （`[规则(强信号)]` / `[模型仲裁(规则弱命中X,改判 Y)]` / `[规则(弱命中,模型确认)]`），可审计。
- **`scan --classify-model <MODEL>` 体裁识别的模型通道**：规则判据（文号/国标骨架/
  双主体/Heading 样式）确定性高但覆盖有限，现补上模型兜底。两种模式：
  - **默认（规则优先）**：规则判不出体裁（Generic）时调用模型兜底（dsv4），实测
    接口规范类 docx 由规则的 generic 修正为 standard；
  - **指定模型（模型优先）**：`--classify-model qwen3.6-27b` 等显式指定后改为模型
    先行判定，规则仅在模型失败时兜底。
  提示词含体裁特征描述（contract/official/standard/generic），抽样正文前 3000 字符；
  回复解析宽松（大小写不敏感、容忍推理模型的思考文本、按语义优先级而非出现顺序）；
  模型失败回退规则结果并告警，不阻断转换。判定路径在 INFO 行明示
  （`[规则]` / `[模型(<model>)]` / `[规则(模型失败回退)]`），可审计。
- **`scan` 排版/样式信息提取（合同、公文、标准规范重点优化）**：新增 `style` 模块，
  从两条权威来源取样式，用于补足「编号层级」覆盖不到的场景：
  - **DOCX**：解析 `word/styles.xml` 的 Heading/标题 样式与段落 `w:pStyle`，得到
    文档真实的标题层级。中文文档常用数字 styleId（如 `4`/`5`），必须经名称解析
    才不会误判；无命名样式时回退 `w:outlineLvl`。实测接口规范类 docx 还原出
    **797 个标题、6 级层级**（1 / 2 / 2.1 / 2.1.1…）。
  - **PDF**：解析页面内容流，跟踪 `Tf`（字体/字号）与 `Tj/TJ`（文本），由字体名与
    FontDescriptor `Flags` 的 ForceBold 位判定粗体。字号/字重在文字编码损坏
    （非标准 CMap）时依然有效——这是几何/样式信息相对文本编码的天然优势。
  样式只做**精确文本匹配后的层级校正**，不凭空新增标题。
- **`scan` 结构缺失时从编号补建标题**：合同等文档的章/条与正文同字号、无内置
  样式，抽取通道不产出任何标题，此时「重定级」无从下手。现按「编号行数 > 现有
  标题数 × 2」判定结构大部分缺失，再从 `第X编/分编/章/节/条` 补建标题
  （`（一）`/`1.` 属条款内列举，不升级）。结构已完整时不触发，避免把正文条文
  全部打散。
- **`scan` 标题层级增设「文档标题占位」**：首个标题若不是编号单元且其后存在编号
  单元，结构整体下移一级。否则会出现「标题与第一章同为一级」的并列误读。
- **`scan` 新增合同/协议体裁**：判据为双主体（甲方/乙方/发包人/承包人…）+ 约定
  措辞或签署区之一；仅出现「合同」字样（如《合同管理办法》）不判为合同。
  合同与公文办法体裁共用自适应编号定级（章 > 条 > （一））。
- **`scan` 法规类识别**：`国务院令`/`条例`/`第一条+第二条+附则` 骨架判为公文
  体裁，从而走条文体裁层级重建（实测《招标投标法实施条例》还原 令 > 章 > 条）。
- **`scan` 标题文本规整**：抽取通道把原文粗体转成 `**...**` 后落到标题里成为冗余
  噪声（`# 2 **接口列表**`），现统一移除标题内的 `**`/`__` 标记；单 `*` 不处理
  （可能是正文符号）。
- **`scan` 文档类型识别与体裁特化排版**：转换后先判定文档类型（公文 / 标准规范 /
  表格 / 演示文稿 / 图示 / 图像 / 通用），再按体裁走不同层级重建规则。纯规则判定
  （文号 `〔YYYY〕N号`、国标标识 `GB/T`+`ICS`+「范围/规范性引用文件/术语和定义」骨架、
  格式直判），确定性且无模型开销；`--no-classify` 可关闭。
- **`scan` 编号层级自适应重建**：实测发现抽取通道会把层级压平（34 页国标 126 个
  标题全部落在两级、144 页法典 81 个标题 80 个在一级），而这类文档**层级不在字号里**
  （国标正文与标题同为 9.7pt，法典全篇 9.0pt 单一字号），只在编号文本中。现按文中
  **实际出现的**编号体裁（编/分编/章/节/条、一、/（一）/1./（1））压紧排名定级，
  而非固定映射——同一个「（一）」在通知体裁是二级、在「办法」体裁位于章条之下
  却是更深级，固定表必然出错。实测法典由 2 级恢复到 4 级、公文办法体裁
  章>条>（一）层次正确。**不做**按编号数值重排段落顺序（跨页/跨节边界判定不可靠，
  误判代价高于收益）。
- **`scan` 图片附件落盘（assets）**：OOXML 容器精确抽取 `word|ppt|xl|visio/media`
  原图字节；PDF 经 lopdf 抽取内嵌图并按「像素/页面点」比值剔除整页扫描底图
  （300dpi A4 扫描 ≈4.17，页内插图远低于 2.5 阈值）。输出到 `assets/<源文件名>/`，
  Markdown 使用相对路径引用，可整体搬移；`--assets-dir` 自定义位置、`--no-assets` 关闭。
  引用重写采取**严格计数匹配**：引用数与附件数相等时才按顺序映射，不等则一律标记
  `qyzs:missing-asset` 并附「图片附件」索引章节——顺序硬配会把 A 图放到 B 的位置，
  静默内容错配比缺失更糟。
- **`scan` 照片跳过**：图片输入先由视觉模型（qwen3-VL）判定是文档类还是真实照片/插画，
  照片直接跳过、不产出空文件、退出码 0，批量处理时继续后续文件（新增
  `QyzsError::Skipped` 语义，区别于失败）。判定失败不阻断，按文档继续并告警。
- `scan` 图片引用「未落盘」告警改为存在性校验：assets 刚落盘的引用不再误报。
- `scan --report` 阶段耗时新增 `classify` / `layout` / `assets` 三项。
- `status --no-probe`：跳过远程三网关探活，仅输出本地环境、配置路径与凭据权限。
  默认行为（`qyzs status` 执行全链路诊断）不变，为纯增量开关。跳过探针时不输出
  全局健康结论（跳过 ≠ 正常）；`--json` 模式的 `probes` 字段为 `null`，脚本可据此
  区分「未探测」与「探测失败」。
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
- `usage` 统计维度扩展：调用族（`--kind` 过滤 + [按调用族] 段）、凭据
  （每行记录掩码指纹，`--key` 过滤 + 多凭据时自动展示 [按凭据] 段）、
  小时窗口（`--hourly` UTC 小时桶替代按日）；归档历史单列标注/`archived`
  口径，不与小时桶混淆。
- `usage` 历史压实：明细保留 30 天（UTC 日界），超期明细在 `usage` 调用时
  时机性压实为 日×模型×调用族×凭据 归档行（`count` 字段承载聚合请求数），
  与既有归档行同组合并；create_new 锁 + 60s 僵尸锁清除，活锁占用跳过本次；
  同目录 tmp+rename 原子重写，幂等。
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
- **markitdown rigor 档扫描件多模型交叉验证（三通道证据）**：此前 rigor 的第二通道是
  PDF 文本层，扫描件文本层为空、交叉验证从未真正运行，只能诚实降级为 balanced。
  现新增图像证据通道（`pdf_ocr` 模块）：提取扫描件页面内嵌整页图（典型扫描件每页一张
  DCTDecode JPEG，lopdf 直接取原始流字节，纯 Rust 零栅格化、零新增依赖），逐页
  （4 路并发、单页单通道失败跳过并告警）送双模型转录——paddleocrvl 文字逐行转录 +
  qwen3-VL-30B-A3B 图像理解（逐字转录、表格逐格、印章/文号/页码要素标注、版面阅读
  顺序，8192 token 输出预算），按页合并为第二通道证据，与主通道 MinerU 构成三通道
  交叉验证。产物持久化为 `secondary-evidence` 段（`--resume` 可续跑），证据通道全部
  不可用时仍诚实降级并在报告写明原因。仲裁与润色模型经 `-m` 指定（准确率优先可取
  `-m qwen3.6-27b` 深度推理）。实测 N104（14 页扫描件）三配置对比：OCR 单证据
  dsv4 发现 7 冲突段/83s；双证据 dsv4 降为 5 段/96s；双证据 + qwen3.6-27b 4 段/424s
  （仲裁 139s，且将分类表格重建为 14 行 Markdown）。qwen3.6-27b 的思维链会偶发泄漏
  进全局审查建议文本，提示词约束之外现做特征检出：含思维链噪声的建议整体丢弃并
  如实注明「建议该步骤使用非推理模型」，最终产物实测无污染；默认仲裁模型仍为 dsv4。
- markitdown 过程警告全面持久化：润色质检回退、仲裁修正/确认/不可决、全局审查
  建议、OCR 通道逐页告警此前只打印到 stderr，`--report` 报告的 warnings 读自
  manifest 而看不到这些过程信息（二轮实测发现降级告警缺失、三轮实测发现仲裁
  结果在报告中不可见）。现统一落盘，报告可完整追溯每一段的验证与仲裁结论。
- markitdown 产物图片引用落盘缺口的显式披露：MinerU 的 Markdown 正文内联
  `![](images/<sha256>.jpg)` 形式的图片引用，但当前网关只返回正文、不提供图片
  字节（实测 `/images`、`/zip`、`/result` 均 404），这些引用在产物中是死链。
  此前既不告警也不计数，使用方无从得知。现于转换结束时统计并告警未落盘图片
  引用数，`--report` 报告新增 `image_refs` 与 `unresolved_image_refs` 字段。
  正文内容本身保持原样（图片引用是 MinerU 的事实，抹掉会丢失「此处有图」信息）。

### Changed

- **（破坏性）删除 `ocr` 子命令，功能并入 `scan`**：两个入口都做「图像→文字」，
  但 `ocr` 只出纯文本、`scan` 出结构化 Markdown 且支持多文件/体裁识别/图片资产，
  长期并存会让用户困惑。迁移：
  - `qyzs ocr img.png` → `qyzs scan img.png --image-handling ocr --no-front-matter`
  - `qyzs ocr img.png -p "自定义引导词"` → `qyzs scan img.png --image-prompt "自定义引导词"`
  - `qyzs ocr img.png --max-dim 1600` → `qyzs scan img.png --max-dim 1600`
  `scan` 相对 `ocr` 的增量：多文件批量、照片自动跳过、`--image-handling`
  （auto/ocr/describe/both/skip）、VL 图像理解、Markdown 结构化输出、`--report` 报告。
  内部 `client.ocr` 能力保留（scan 的图像通道与扫描件第二通道证据仍使用）。

### Changed

- **（破坏性）`parse` 与 `markitdown` 子命令合并为 `scan`**：两个入口职责重叠
  （都做文档→Markdown，一个裸调 MinerU、一个跑多通道流水线），且 `markitdown`
  名不副实。现统一为 `scan`，参数面沿用原 `markitdown`（`-o/--mode/--type/
  --model/--report/--work-dir/--segment-chars/--resume/--no-clean/
  --no-front-matter/--dry-run`），并新增 `--assets-dir` / `--no-assets` /
  `--no-classify`。迁移：`qyzs parse f.pdf` → `qyzs scan f.pdf -o f.md`；
  `qyzs markitdown f.pdf` → `qyzs scan f.pdf`。内部模块 `pipeline::markitdown`
  同步更名为 `pipeline::scan`（`MarkitdownPipeline` → `ScanPipeline`）。
- **退出码对齐 sysexits.h（行为变更，脚本需注意）**：`Network`/`Server`/`Config`
  三类错误此前统一返回 `FAILURE=1`，外部脚本无法区分网络不可达、服务端故障与本地
  配置错误。现分别映射到 BSD `sysexits.h` 标准码：`Network → 69 (EX_UNAVAILABLE)`、
  `Server → 70 (EX_SOFTWARE)`、`Config → 78 (EX_CONFIG)`。Usage/Auth/Timeout/Io/Pdf
  的 2/3/4/5 保持不变。完整退出码表写入 README「退出码约定」小节（此前从未文档化）。
- 全局参数取值校验收紧（与语言参数同一策略：CLI 显式非法值报 USAGE_ERR 并列出
  合法值，环境变量/配置文件层保持 best-effort 回退）：`--color` 非法值此前静默
  回退 auto，现报错；`--format` 非法值此前静默回退 text，现报错；`--model-lang`
  非法值此前原样透传（等价 auto 不注入），现报错，合法值集对齐 chat 注入逻辑
  为 `auto|cn|zh|zh-cn|en`（`zh` 为消费端本已支持而文档缺失的值，help/README 同步）；
  `--base-url` 非 http(s) 前缀此前拖到请求时才失败，现启动即报错。
  markitdown 子命令参数（`--mode`/`--type`/`--image-handling`/`--min-confidence`）
  经审计本已有显式校验，未改动。校验策略写入 `src/cli/args.rs` 模块头注释。
- `config set` 写入路径取值校验：枚举类配置项（`ui.language` 含 `auto`、
  `ui.color`、`ui.format`、`model.language`、`network.base_url`）与 CLI 参数
  共用同一套校验器与合法值集，非法值拒绝写入并报 USAGE_ERR；读取时保持
  best-effort，存量脏配置不阻断命令。校验器下沉至 `config/resolver.rs`
  （`validate_base_url`/`parse_model_language`/`parse_color_choice`）供 CLI
  与配置两路复用；timeout 整数校验与 `model.default_*` 模型名（无枚举）不变。
- `config set` 支持 per-model 超参表写入：`config set model.params.<模型名>.<参数名> <数值>`
  （如 `model.params.dsv4.temperature 0.7`），此前只能手编 config.toml。模型名按
  消费端主查找形态归一化（`qwen3.6-27b` → `qwen3_6_27b`），参数值仅接受数值
  （i64 优先其次 f64，与 temperature/top_p/max_tokens/top_n/max_dim 消费点一致）；
  供 `chat --temperature/--top-p` 等未显式传参时级联取用。
- `--model-lang` help 与 README 注明「仅 chat 生效」：该参数当前唯一消费点是
  chat 的 system prompt 语言注入，对其余子命令为静默 no-op（markitdown 润色
  阶段语言由流水线内部 prompt 控制，不受此参数影响）。
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

- **scan 多格式图像支持**：此前 `image` crate 只启用 png/jpeg 解码，其余格式
  （gif/bmp/tiff/webp）无法缩放 —— 大图绕过 800px 预处理直接送网关，正是 max_dim
  要防的越界场景；且 data URL 的 MIME 映射把 gif/tiff 误标为 image/png，网关会按
  PNG 解码而失败。现启用 bmp/gif/tiff/webp 解码并修正 MIME 映射，6 种格式实测
  全部转换成功（关键文字命中 8~9 处，表格与版面描述均正确）。

### Fixed

- **scan 与 SSOT 文档的一致性**（2026-10-21 全量审计）：
  - 运行时消息全部改走 i18n 目录（新增 8 个 MsgId：体裁判定/模型失败/弱命中仲裁/
    图像判定/样式校正/资产落盘/附件索引说明），此前新增能力用裸中文 `eprintln!`
    输出，违反「运行时消息全量走目录」契约，英文界面会中英混排。
  - `tests/docs/QYZS-SCAN.md`（原 QYZS-MARKITDOWN.md）标题与内容同步：新增
    §3.2「体裁识别与排版样式转换」（识别顺序、置信度分派、仲裁语义、编号层级
    自适应重建、样式来源、资产抽取与严格计数匹配、明确不做的重排），§3 档位表
    补扫描件三通道，§4 补图片先判文档/照片。
  - `model.default_parse` 配置项此前因 `parse` 子命令删除而成为孤儿（只可读写、
    不影响任何行为）。现 MinerU 端点路径取该配置（`ai-gateway/<model>/file_parse`），
    配置重新生效并可切换解析模型版本；`get_default_model` 接受 "scan" 与 "parse" 两个键。
  - 附件索引章节的说明文字改走 i18n；测试中的旧命令名措辞同步。

### Fixed

- `usage` 过滤值两端空白容错：`--key` / `--model` / `--kind` 此前做精确字符串比对，
  用户从 `[按凭据]` 表格列复制掩码指纹时会连带左对齐填充的尾随空格，导致静默
  匹配不到、输出「无数据」而退出码仍为 0（比报错更难排查）。现统一 `trim()`
  后比对，JSON 的 `filter` 段回显同步改为 trim 后的实际生效值。
- 破坏性操作前警告纳入 i18n 目录：`auth logout` / `config reset` / `usage --clear`
  三条 `⚠` 警告此前硬编码中文，英文界面（`--lang en` / `QYZS_LANG=en`）下会中英
  混排。现走 `MsgId` 目录并补齐中英文案。
- `usage_ledger::tests::filter_by_days_model_kind_key` 修复日历漂移：测试此前
  硬编码 `2026-10-18` 作为「今天」，过了该日 `--days` 窗口断言必然失败。改用
  相对当前时间生成的 `ts_of(days_ago, hour)` 辅助函数，该测试不再随日期失效。

- MinerU 任务型响应解析：空白页文档 `md_content` 为空时不再泄漏原始 JSON 到
  输出；网关返回 pending/running 异步状态时显式报错而非静默；整体抽取结果
  为空时 markitdown 报错，不产出空文件假成功。
- LLM 响应取文：新增 `ChatResponse::primary_text`（content 空回退 reasoning），
  修复 qwen3-VL 非流式偶发空响应（实测该模型把完整回答放入 reasoning 字段）。
- markitdown 转换场景客户端超时下限 300s（实测扫描件解析长尾超过默认 60s）。
- PDF 切片性能病态（`markitdown` 与 `pdf-split --chunk-size` 共用路径）：此前
  `PdfSplitter::chunk` 对每个切片都重新解析全文，并调用 lopdf 的 `delete_pages`
  逐页删页；而 `delete_object` 内部每次都跑一遍全文档 `traverse_objects`，该遍历
  又以 `Vec::contains` 线性查找已访问对象，整体退化为 O(页数 × 引用数 × 对象数)。
  实测 277 页 / 58866 对象 / 215983 引用的文档，单次遍历即耗时 31s，单个切片
  600s 仍未完成，文档整体无法转换。现改为「按需保留」策略：用 HashSet 可达性
  遍历算出目标页真正依赖的对象子集（O(引用数)），裁剪原页树 `/Kids` 并重算
  `/Count`，切片文档只含目标页及其资源；同批切片复用一次解析结果。实测该文档
  切片由 >600s 未完成降至 0.5s，产物体积由 156MB 降至 16MB，切片内容与原文
  逐页一致。
- PDF 切片页序错位与内容越界：旧实现裁剪页树后未按裁剪结果重算 `/Count`，
  且未跳过页对象的 `/Parent` 与页树节点的 `/Kids` 边，可达性遍历会经页树绕回
  全文页面。现修正 `/Count` 计算顺序、显式接管页树边，并新建最小 Catalog
  （不沿用原 Catalog 的 `/StructTreeRoot`、`/Outlines` 等指向全文的引用枢纽）。
  另修正新建对象 id 与被复制对象撞号导致根页树被覆盖的问题。
- markitdown `rigor` 档破坏表格结构：第二通道是扁平纯文本，结构上无法表达
  表格，表格段落与之做向量比对必然低相似度而被判为冲突，仲裁环节让模型按
  扁平证据重写，表格被压回流水句（实测 balanced 档 50 行表格 → rigor 档 0 行）。
  现将表格块整体排除在交叉验证与仲裁之外（原样保留），并在仲裁落盘前增加
  结构护栏：修正结果丢失表格行数时回退保留原文。实测 rigor 档表格行数恢复为 50。
  另：第二通道过薄触发诚实降级时的告警此前只打印到 stderr、未持久化到 manifest，
  `--report` 报告里看不到降级原因；现已落盘。
- `verify` 同源互验指标失效：此前用「唯一字符集合」的 Jaccard，对中文公文几乎
  无鉴别力——实测同源文档 49%~87%、完全无关的两份中文文档也达 49%~58%，全部
  越过 15% 阈值判为「通过」，闸门无法识别错配。现改为字符 n-gram 集合的
  Jaccard（同时编码字符内容与相邻顺序）：实测无关文档对由 49% 降至 2.01%
  并被正确拒绝，同源文档仍正常通过。
- markitdown `--report` 报告字段缺失：CLI 帮助声明输出「各通道置信度/冲突清单/
  轮次耗时」，实际报告只有一条不透明的 `convert` 阶段记录。现补充阶段耗时明细
  （extract / polish / cross-verify / arbitrate / global-review）、冲突段落数、
  生效的相似度阈值，以及图片引用计数。
- markitdown `--work-dir` 死参数：参数已解析进选项结构但全仓库无人读取，工作区
  恒定落在 `~/.cache/qyzs/markitdown/<哈希>/`。现已接到工作区开立入口，指定目录
  时工作区（含 manifest/切片/段产物）实际落在该目录下。
- `parse --is-ocr` 无法置 false：clap 将其推断为 SetTrue 标志，`--is-ocr=false`
  报 `unexpected value 'false'`，与其帮助文本「纯文本 PDF 可显式置 false 跳过」
  矛盾。现改为可取值的布尔参数（`--is-ocr false`），并保留裸标志形式向后兼容。
- `status` 的 MinerU 探针假阴性：探测路径 `ai-gateway/mineru3.2.3` 会 301 到
  不带端口的绝对地址，跟随重定向后连到不可达端口而挂起超时，报 `ok:false /
  status 0`，而该专线解析实际全部成功。现改探测实际解析端点（GET 返回 405，
  与另两条专线判定口径一致）并禁止跟随重定向；实测由 `ok:false` 转为 `ok:true`。
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
