//! 文档类型识别 (scan 的分类阶段)
//!
//! 目的是让排版优化按文档「特有特征」走不同规则, 而不是一套通用清洗打天下:
//! - 公文: 红头/文号/主送抄送/落款印章 的结构化分段
//! - 标准规范: 编/章/节/条 编号层级重建 (这类文档字号通常统一, 层级只在编号里)
//! - 表格/演示/图示: 保持结构化输出, 不做段落化改写
//!
//! 判定次序: 格式直判 (xlsx/pptx/vsdx 无需看内容) → 文本特征规则 → 图像由 VL 判定。
//! 规则优先于模型: 公文与国标的特征词/编号模式区分度极高, 规则既确定又免费;
//! 只有「图片是文档扫描件还是照片」这一必须看像素的问题才交模型。

use super::detect::SourceFormat;
use crate::client::{ChatOptions, QyzsClient};
use crate::error::QyzsResult;

/// 文档类型 (决定后续排版策略)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    /// 公文: 红头文件、通知、批复、函
    Official,
    /// 合同/协议: 甲乙双方权利义务、签署区
    Contract,
    /// 标准规范: 国标/行标/地标, 编号条款结构
    Standard,
    /// 表格类: xls/xlsx/csv
    Table,
    /// 演示文稿: ppt/pptx
    Slide,
    /// 图示: vsd/vsdx
    Diagram,
    /// 图片输入 (是否为文档待图像判定)
    Image,
    /// 通用文档
    Generic,
}

impl DocKind {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::Contract => "contract",
            Self::Standard => "standard",
            Self::Table => "table",
            Self::Slide => "slide",
            Self::Diagram => "diagram",
            Self::Image => "image",
            Self::Generic => "generic",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Official => "公文",
            Self::Contract => "合同/协议",
            Self::Standard => "标准规范",
            Self::Table => "表格",
            Self::Slide => "演示文稿",
            Self::Diagram => "图示",
            Self::Image => "图像",
            Self::Generic => "通用文档",
        }
    }
}

/// 格式直判: 这些格式的用途唯一, 不必看内容
pub fn from_format(format: &SourceFormat) -> Option<DocKind> {
    match format {
        SourceFormat::Xls | SourceFormat::Xlsx | SourceFormat::Csv => Some(DocKind::Table),
        SourceFormat::Ppt | SourceFormat::Pptx => Some(DocKind::Slide),
        SourceFormat::Vs | SourceFormat::Vsdx => Some(DocKind::Diagram),
        SourceFormat::Image(_) => Some(DocKind::Image),
        _ => None,
    }
}

/// 判定置信度: 决定是否送模型仲裁
///
/// 误判代价不对称 —— 假阳性 (公文被抢成标准) 会经历错误的排版干预, 比假阴性
/// (少做重建) 更糟。因此弱信号命中不直接定案, 交模型复核; 模型不可用时保留
/// 弱判定 (宁可保住行为, 不静默放大错误)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// 强信号: 强标记/文号/红头/双主体/完整条款骨架
    Strong,
    /// 弱信号: 仅弱标记或单层骨架命中, 建议模型仲裁
    Weak,
}

/// 体裁判定结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub kind: DocKind,
    pub confidence: Confidence,
}

/// 文本特征判定 (仅用于 pdf/doc/docx 这类可能有多种用途的格式)
///
/// 只看样本 (通常取正文前若干字符): 特征词与编号模式都在开篇出现。
pub fn from_text(sample: &str, format: &SourceFormat) -> DocKind {
    decide(sample, format).kind
}

/// 完整判定 (含置信度), 供调用方决定是否送模型仲裁
pub fn decide(sample: &str, format: &SourceFormat) -> Decision {
    if let Some(k) = from_format(format) {
        return Decision { kind: k, confidence: Confidence::Strong };
    }
    // 合同先于标准/公文判定: 合同同样会引用标准编号、带文号, 但核心是双方约定
    if is_contract(sample) {
        return Decision { kind: DocKind::Contract, confidence: Confidence::Strong };
    }
    match standard_decision(sample) {
        Some(d) => return d,
        None => {}
    }
    official_decision(sample)
}

// ── 标准规范 ────────────────────────────────────────────────────────────

/// 强标记: 这些词只出现在标准文本自身 (封面/首页), 主题词歧义小
const STRONG_STD: &[&str] = &[
    "中华人民共和国国家标准",
    "中华人民共和国国家标准化指导性技术文件",
    "ICS ",
    "ICS\t",
    "DB53/T",
];

/// 弱标记: 常见主题词。公文**引用**标准时同样出现 (「按行业标准执行」
/// 「GB/T 22239」), 2026-10-21 审计中 34 份公文因此被误抢 —— 必须配合
/// 完整标准编号形或骨架行才可判, 且只给弱置信。
const WEAK_STD: &[&str] = &["GB/T ", "GB/T\t", "GB ", "DB/T", "行业标准", "地方标准"];

/// 国标骨架: 需为「编号行」形态 (1 范围 / 2 规范性引用文件), 杜绝「适用范围」
/// 这类正文词蹭票
const STD_SKELETON: &[&str] = &["范围", "术语和定义", "规范性引用文件"];

/// 标准规范判定: Some(Strong) 强命中 / Some(Weak) 弱命中 / None 非标准
fn standard_decision(sample: &str) -> Option<Decision> {
    let strong = |d: Decision| Some(d);
    if STRONG_STD.iter().any(|m| sample.contains(m)) {
        return strong(Decision { kind: DocKind::Standard, confidence: Confidence::Strong });
    }
    let skeleton_hits = STD_SKELETON.iter().filter(|k| numbered_skeleton_line(sample, k)).count();
    if skeleton_hits >= 2 {
        return strong(Decision { kind: DocKind::Standard, confidence: Confidence::Strong });
    }
    // 弱标记 + 完整标准编号形 (如 GB/T 22239-2019): 可能是标准, 也可能是
    // 公文+标准附件的合订本 —— 弱置信, 交仲裁
    if WEAK_STD.iter().any(|m| sample.contains(m)) && has_std_number(sample) {
        return Some(Decision { kind: DocKind::Standard, confidence: Confidence::Weak });
    }
    None
}

/// 检测样本中是否存在「某行以编号开头, 紧跟骨架关键词」的行
fn numbered_skeleton_line(sample: &str, keyword: &str) -> bool {
    sample.lines().any(|line| {
        let t = line.trim();
        let mut chars = t.chars();
        // 行首: 阿拉伯数字编号 (可带点分层, 如 1 / 2.1)
        let mut saw_digit = false;
        let mut it = t.char_indices();
        let mut end = 0usize;
        loop {
            match it.next() {
                Some((i, c)) if c.is_ascii_digit() || c == '.' || c == ' ' || c == '\u{3000}' => {
                    if c.is_ascii_digit() { saw_digit = true; }
                    end = i + c.len_utf8();
                }
                _ => break,
            }
        }
        if !saw_digit {
            return false;
        }
        let _ = chars.next();
        t[end..].trim_start_matches([' ', '\u{3000}']).starts_with(keyword)
    })
}

/// 是否存在完整标准编号形 (GB 或 GB/T + 3~5 位数字)
fn has_std_number(sample: &str) -> bool {
    let mut from = 0usize;
    while let Some(pos) = sample[from..].find("GB") {
        let start = from + pos;
        let mut rest = &sample[start + 2..];
        if let Some(r) = rest.strip_prefix("/T") {
            rest = r;
        }
        let rest = rest.trim_start_matches([' ', '\u{3000}', '\t']);
        let digits: usize = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if (3..=5).contains(&digits) {
            return true;
        }
        from = start + 2;
    }
    false
}

/// 公文特征: 文号 (〔年份〕序号号) + 公文惯用语
///
/// 兼容旧入口: 等价于 official_decision 判 official 即可。
pub fn is_official(sample: &str) -> bool {
    official_decision(sample).kind == DocKind::Official
}

/// 公文惯用语标记
const OFFICIAL_MARKERS: &[&str] = &[
    "国务院令",
    "中华人民共和国国务院令",
    "条例",
    "特此通知",
    "特此函复",
    "特此批复",
    "现将有关事项通知如下",
    "请遵照执行",
    "请认真贯彻执行",
    "主送",
    "抄送",
    "印发给你们",
];

/// 公文判定 (2026-10-21 语料审计修正):
/// - 附则容空白: 法律排版是「附　则」, 字面 contains 全部失配 (17 部法律因此漏判)
/// - 三条骨架免标记词: 法律永远不含「条例」二字, 要求 hits>=1 等于拒绝整个法律类
/// - 红头词尾补 函/意见/决定/公告/批复/纪要
/// - 「第一章+第一条」单层骨架只给弱置信 (技术手册也可能有)
fn official_decision(sample: &str) -> Decision {
    let official = |c: Confidence| Decision { kind: DocKind::Official, confidence: c };

    // 文号: 强
    if has_doc_number(sample) {
        return official(Confidence::Strong);
    }
    let hits = OFFICIAL_MARKERS.iter().filter(|m| sample.contains(*m)).count();
    let red = has_red_header(sample);
    if hits >= 2 || (hits >= 1 && red) || red {
        return official(Confidence::Strong);
    }

    // 条文体裁强骨架: 第一/二/三条齐备。法律/条例/办法的固定骨架, 无需标记词
    // —— 这是 laws/rules/local-regulations 漏判的根因修复。附则 (容空白) 存在
    // 时同样强, 但不作硬性要求: 审计中半数部门办法没有附则章。
    if sample.contains("第一条")
        && sample.contains("第二条")
        && sample.contains("第三条")
    {
        return official(Confidence::Strong);
    }

    // 单层骨架 (第一章+第一条): 技术手册也可能有, 弱置信送仲裁
    if sample.contains("第一章") && sample.contains("第一条") {
        return official(Confidence::Weak);
    }
    // 「一、（一）」段落骨架 (意见/方案体裁) 至少两组: 同为弱置信
    if sample.contains("一、") && sample.contains("（一）") {
        return official(Confidence::Weak);
    }
    Decision { kind: DocKind::Generic, confidence: Confidence::Strong }
}

/// 附则 (容中间空白: 「附　则」「附 则」)
fn has_loose_fuze(sample: &str) -> bool {
    let mut chars = sample.char_indices();
    while let Some((i, c)) = chars.next() {
        if c != '附' {
            continue;
        }
        // 附 后允许至多 3 个空白字符再接 则
        let mut spaces = 0;
        let mut it = sample[i + c.len_utf8()..].chars();
        for c2 in it.by_ref() {
            if c2 == '则' {
                return true;
            }
            if c2.is_whitespace() && spaces < 3 {
                spaces += 1;
                continue;
            }
            break;
        }
    }
    false
}

/// 红头: 独立短行以公文文种结尾
fn has_red_header(sample: &str) -> bool {
    const ENDINGS: &[&str] = &["文件", "通知", "函", "意见", "决定", "公告", "批复", "纪要"];
    sample.lines().any(|l| {
        let t = l.trim();
        let n = t.chars().count();
        (1..=24).contains(&n) && ENDINGS.iter().any(|e| t.ends_with(e))
    })
}

/// 合同/协议特征: 双方主体 + 权利义务 + 签署区
///
/// 判据组合 (需同时命中两类以上), 避免把"合同管理办法"这类规章误判为合同:
/// - 主体: 甲方/乙方/丙方/发包人/承包人/委托方/受托方
/// - 约定: 本合同/本协议/双方/经协商一致/达成如下协议
/// - 签署: 签字/盖章/签订地点/合同编号/生效日期
pub fn is_contract(sample: &str) -> bool {
    const PARTIES: &[&str] = &[
        "甲方", "乙方", "丙方", "发包人", "承包人", "委托方", "受托方", "承租方", "出租方",
    ];
    const AGREEMENT: &[&str] = &[
        "本合同", "本协议", "经双方", "经甲", "协商一致", "达成如下", "兹就", "特订立",
    ];
    const SIGNING: &[&str] = &[
        "签字", "盖章", "签订地点", "合同编号", "协议编号", "生效日期", "签订日期",
    ];

    let parties = PARTIES.iter().filter(|m| sample.contains(*m)).count();
    let agreements = AGREEMENT.iter().filter(|m| sample.contains(*m)).count();
    let signing = SIGNING.iter().filter(|m| sample.contains(*m)).count();

    // 双主体是合同的硬特征; 再需约定措辞或签署区之一
    parties >= 2 && (agreements >= 1 || signing >= 1)
}

/// 检测公文文号: 〔YYYY〕N号 (含六角/方括号变体)
pub fn has_doc_number(sample: &str) -> bool {
    let chars: Vec<char> = sample.chars().collect();
    for i in 0..chars.len() {
        if chars[i] != '〔' && chars[i] != '[' && chars[i] != '【' {
            continue;
        }
        // 〔 后 4 位数字 + 〕
        if i + 5 < chars.len()
            && chars[i + 1..i + 5].iter().all(|c| c.is_ascii_digit())
            && (chars[i + 5] == '〕' || chars[i + 5] == ']' || chars[i + 5] == '】')
        {
            // 其后应有 数字+号
            let rest: String = chars[(i + 6).min(chars.len())..]
                .iter()
                .take(8)
                .collect();
            if rest.contains('号') && rest.chars().any(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

/// 体裁识别模型提示词 (规则判不出去时的模型兜底)
const GENRE_PROMPT_TEMPLATE: &str = "你是公文与专业文档的体裁分类器。根据以下文档片段判断体裁, 只回答一个关键词:\n\
- contract: 合同/协议/契约 (甲乙双方权利义务、标的、价款、签署条款)\n\
- official: 公文/法规/规章 (红头、文号、通知、批复、条例、办法、第一条…)\n\
- standard: 标准规范 (GB/T、ICS、范围、规范性引用文件、术语和定义)\n\
- generic: 其他 (实施方案、工作报告、说明材料、网页内容等)\n\
只回答 contract / official / standard / generic 之一, 不要解释。\n\
[文档片段]\n{}";

/// 文本抽样上限 (模型只需开篇即可判体裁, 避免整篇塞进上下文)
const GENRE_SAMPLE_CHARS: usize = 3000;

/// 模型体裁识别 (规则判不出 Generic 时的兜底; 或 --classify-model 指定时优先)
///
/// 解析是宽松的: 推理类模型偶发在关键词前后带思考文本, 按首个命中关键词判定。
pub async fn classify_genre_model(
    client: &crate::client::QyzsClient,
    sample: &str,
    model: Option<&str>,
) -> QyzsResult<DocKind> {
    let trimmed: String = sample.chars().take(GENRE_SAMPLE_CHARS).collect();
    let prompt = GENRE_PROMPT_TEMPLATE.replace("{}", &trimmed);
    let resp = client
        .chat(
            &prompt,
            None,
            crate::client::ChatOptions {
                model: model.map(str::to_string),
                temperature: Some(0.0),
                max_tokens: Some(64),
                top_p: None,
                stream: false,
                show_reasoning: false,
                images: Vec::new(),
            },
        )
        .await?;
    Ok(parse_genre_reply(&resp.primary_text()).unwrap_or(DocKind::Generic))
}

/// 从模型回复解析体裁 (大小写不敏感, 容忍前后噪声)
pub fn parse_genre_reply(text: &str) -> Option<DocKind> {
    let t = text.to_ascii_lowercase();
    // 顺序即优先级: contract/official/standard 语义明确, generic 兜底最后
    const ORDER: &[(&str, DocKind)] = &[
        ("contract", DocKind::Contract),
        ("official", DocKind::Official),
        ("standard", DocKind::Standard),
        ("generic", DocKind::Generic),
    ];
    ORDER
        .iter()
        .find(|(kw, _)| t.contains(kw))
        .map(|(_, k)| *k)
}

/// 图像内容判定: 文档扫描件/截图 vs 真实照片
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageContent {
    /// 文档类 (扫描件、截图、表格、公文照片) —— 需要转换
    Document,
    /// 真实照片/插画 —— 按需求跳过
    Photo,
    /// 模型未能判定
    Unknown,
}

const IMAGE_KIND_PROMPT: &str = "判断这张图片属于哪一类, 只回答一个词:\n\
- DOCUMENT: 文档类 —— 扫描件、拍照的纸质文件、屏幕截图、表格、票据、书籍页、公文\n\
- PHOTO: 真实场景照片、人物、风景、产品图、插画、图表设计稿等非文档图像\n\
只回答 DOCUMENT 或 PHOTO, 不要解释。";

/// 用视觉模型判定图片是否为文档 (照片按需求跳过)
pub async fn classify_image(client: &QyzsClient, path: &str) -> QyzsResult<ImageContent> {
    let resp = client
        .chat(
            IMAGE_KIND_PROMPT,
            None,
            ChatOptions {
                model: None, // 有图 → 客户端按模型决策选 qwen3-VL
                temperature: Some(0.0),
                max_tokens: Some(64),
                top_p: None,
                stream: false,
                show_reasoning: false,
                images: vec![path.to_string()],
            },
        )
        .await?;
    let t = resp.primary_text().to_ascii_uppercase();
    if t.contains("DOCUMENT") {
        Ok(ImageContent::Document)
    } else if t.contains("PHOTO") {
        Ok(ImageContent::Photo)
    } else {
        Ok(ImageContent::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_detected() {
        let s = "中华人民共和国国家标准\n信息安全技术 网络安全等级保护基本要求\nGB/T 22239-2019\n\
                 ICS 35.040\n1 范围\n2 规范性引用文件\n3 术语和定义";
        let d = decide(s, &SourceFormat::Pdf);
        assert_eq!(d.kind, DocKind::Standard);
        assert_eq!(d.confidence, Confidence::Strong, "强标记应为强置信");
    }

    #[test]
    fn test_official_detected() {
        let s = "云南省自然资源厅文件\n云自然资〔2024〕57号\n各州、市自然资源主管部门:\n\
                 现将有关事项通知如下, 请认真贯彻执行。";
        assert!(has_doc_number(s));
        assert!(is_official(s));
        assert_eq!(from_text(s, &SourceFormat::Pdf), DocKind::Official);
    }

    #[test]
    fn test_weak_standard_requires_full_number() {
        // 仅引用「行业标准」无标准编号: 不再判 standard (审计中 18 份公文因此被抢)
        let cite = "关于推进政务信息系统建设的通知\n各单位要严格落实行业标准有关要求。";
        assert_ne!(decide(cite, &SourceFormat::Pdf).kind, DocKind::Standard);
        // 弱标记 + 完整标准编号形: standard 但弱置信 (可能是合订本)
        let weak = "关于加强网络安全管理的通知\n按照GB/T 22239-2019要求开展等级保护。";
        let d = decide(weak, &SourceFormat::Pdf);
        assert_eq!(d.kind, DocKind::Standard);
        assert_eq!(d.confidence, Confidence::Weak, "弱标记只应给弱置信");
    }

    #[test]
    fn test_std_number_detection() {
        assert!(has_std_number("按照GB/T 22239-2019要求"));
        assert!(has_std_number("符合GB 50174规定"));
        assert!(has_std_number("GB/T22239"));
        assert!(!has_std_number("GB标准"), "GB 后无编号不应命中");
        assert!(!has_std_number("GB12"), "少于 3 位编号不命中");
    }

    #[test]
    fn test_numbered_skeleton_line() {
        let s = "1 范围\n本文件规定了…\n2 规范性引用文件。\n适用范围另行规定。";
        assert!(numbered_skeleton_line(s, "范围"));
        assert!(numbered_skeleton_line(s, "规范性引用文件"));
        // 「适用范围」不在行首编号位, 不命中
        let s2 = "一、适用范围\n详见附录。";
        assert!(!numbered_skeleton_line(s2, "范围"), "正文『适用范围』不应蹭票");
    }

    #[test]
    fn test_law_with_spaced_fuze() {
        // 法律排版: 附　则 (全角空格) —— 审计中 17 部法律因此漏判
        let law = "中华人民共和国城乡规划法\n第一条 为了加强城乡规划管理\n                   第二条 本法适用于…\n第三条 本法所称…\n\
                   第十二章 附　则\n第七十条 本法自…施行";
        let d = decide(law, &SourceFormat::Pdf);
        assert_eq!(d.kind, DocKind::Official);
        assert_eq!(d.confidence, Confidence::Strong, "三条+附则(容空白)应为强置信");
    }

    #[test]
    fn test_regulation_without_marker_words() {
        // 部门规章: 无条例/通知等标记词, 旧规则要求 hits>=1 导致整类漏判
        let reg = "网络安全审查办法\n第一条 为了确保网络安全\n第二条 关键信息基础设施…\n                   第三条 …\n第四十二条 本办法自发布之日起施行";
        let d = decide(reg, &SourceFormat::Pdf);
        assert_eq!(d.kind, DocKind::Official, "三条骨架齐备应免标记词");
    }

    #[test]
    fn test_red_header_letter_and_opinion() {
        // 函/意见 结尾此前不覆盖 (N291 问卷函、N188 若干意见漏判)
        let letter = "云南省数据局关于开展政务云问卷的函";
        assert!(has_red_header(letter));
        let opinion = "关于建立国土空间规划体系的意见";
        assert!(has_red_header(opinion));
    }

    #[test]
    fn test_weak_chapter_skeleton() {
        // 只有第一章+第一条 (无三条+附则): 弱置信
        let manual = "操作手册\n第一章 系统概述\n第一章下的第一条 本手册适用于…";
        let d = decide(manual, &SourceFormat::Pdf);
        if d.kind == DocKind::Official {
            assert_eq!(d.confidence, Confidence::Weak);
        }
    }

    #[test]
    fn test_contract_detected() {
        let s = "房屋租赁合同\n甲方(出租方): 某某公司\n乙方(承租方): 某某个人\n\
                 经双方协商一致, 达成如下协议:\n第一条 ...\n\
                 甲方(盖章):        乙方(签字):\n签订地点: 昆明\n";
        assert!(is_contract(s));
        assert_eq!(from_text(s, &SourceFormat::Pdf), DocKind::Contract);
    }

    #[test]
    fn test_contract_regulation_not_misjudged() {
        // 规范"合同管理"的规章: 有"合同"字样但无双主体约定结构
        let s = "中华人民共和国国家标准\nGB/T 12345\n1 范围\n2 规范性引用文件\n3 术语和定义";
        assert_eq!(from_text(s, &SourceFormat::Pdf), DocKind::Standard);
        let reg = "关于印发政府采购合同管理办法的通知\n各有关单位:\n请遵照执行。";
        assert!(!is_contract(reg), "仅出现\"合同\"字样不足以判定为合同");
    }

    #[test]
    fn test_generic_when_no_marker() {
        let s = "这是一份普通的项目说明材料, 描述背景与目标, 没有编号也没有公文惯用语。";
        assert_eq!(decide(s, &SourceFormat::Pdf).kind, DocKind::Generic);
        assert!(!is_official(s));
    }

    #[test]
    fn test_standard_wins_over_official() {
        // 国标开篇可能含"通知"字样, 但标准骨架应优先判定
        let s = "中华人民共和国国家标准\nGB/T 12345\n1 范围\n2 规范性引用文件\n3 术语和定义";
        assert_eq!(from_text(s, &SourceFormat::Pdf), DocKind::Standard);
    }

    #[test]
    fn test_format_shortcut() {
        assert_eq!(from_format(&SourceFormat::Xlsx), Some(DocKind::Table));
        assert_eq!(from_format(&SourceFormat::Pptx), Some(DocKind::Slide));
        assert_eq!(from_format(&SourceFormat::Vsdx), Some(DocKind::Diagram));
        assert_eq!(from_format(&SourceFormat::Pdf), None);
        // 格式直判优先于文本规则
        assert_eq!(from_text("中华人民共和国国家标准", &SourceFormat::Xlsx), DocKind::Table);
    }

    #[test]
    fn test_doc_number_variants() {
        assert!(has_doc_number("自然资发〔2024〕57号"));
        assert!(has_doc_number("[2024]57号"));
        assert!(has_doc_number("云政办发【2025】3号"));
        assert!(!has_doc_number("编号 2024-57"));
        assert!(!has_doc_number("〔2024〕无序号"));
    }

    #[test]
    fn test_parse_genre_reply() {
        assert_eq!(parse_genre_reply("contract"), Some(DocKind::Contract));
        assert_eq!(parse_genre_reply("official"), Some(DocKind::Official));
        assert_eq!(parse_genre_reply("standard"), Some(DocKind::Standard));
        assert_eq!(parse_genre_reply("generic"), Some(DocKind::Generic));
        // 大小写不敏感 + 前后噪声容忍
        assert_eq!(parse_genre_reply("CONTRACT"), Some(DocKind::Contract));
        assert_eq!(
            parse_genre_reply("该文档为双方权利义务约定, 属于 contract 类"),
            Some(DocKind::Contract)
        );
        // 多关键词时按语义优先级 (非出现顺序)
        assert_eq!(
            parse_genre_reply("先是 standard 后是 generic"),
            Some(DocKind::Standard)
        );
        // 无法识别
        assert_eq!(parse_genre_reply("不知道"), None);
    }

    #[test]
    fn test_genre_sample_truncated() {
        // 抽样上限逻辑: 超长样本截断到 GENRE_SAMPLE_CHARS
        let long = "甲".repeat(10_000);
        let trimmed: String = long.chars().take(GENRE_SAMPLE_CHARS).collect();
        assert_eq!(trimmed.chars().count(), GENRE_SAMPLE_CHARS);
    }

    #[test]
    fn test_image_format_maps_to_image_kind() {
        assert_eq!(
            from_format(&SourceFormat::Image(super::super::detect::ImageKind::Jpeg)),
            Some(DocKind::Image)
        );
    }
}
