//! PDF 页面拆分与分块切片 (基于 lopdf)
//!
//! 性能契约 (实测校准): 切片必须是 O(对象数) 而非 O(页数 × 对象数)。
//!
//! lopdf 的 `delete_pages` 逐页调用 `delete_object`, 而 `delete_object` 内部
//! 每次都跑一遍全文档 `traverse_objects`; 该遍历用 `Vec::contains` 做已访问
//! 判定 (线性查找), 于是总代价是 O(页数 × 引用数 × 对象数)。
//! 实测 N239 (277 页 / 58866 对象 / 215983 引用): 单次 `traverse_objects`
//! 即耗时 31s, 逐页删除 262 页 → 单个切片 >600s 未完成, 整篇不可转换。
//!
//! 本模块改用「按需保留」策略: 先用 HashSet 可达性遍历算出目标页真正依赖的
//! 对象集合 (O(引用数)), 再由该集合重建切片文档, 全程不调用 `delete_pages`。

use crate::error::{QyzsError, QyzsResult};
use lopdf::{Document, Object, ObjectId};
use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

pub struct PdfSplitter;

impl PdfSplitter {
    /// 解析页码范围表达式 (如 "1-3,5,7-9", "1-")，返回升序且去重的页码列表 (1-indexed)
    pub fn parse_page_range(range_str: &str, total_pages: u32) -> QyzsResult<Vec<u32>> {
        let mut selected = BTreeSet::new();

        for part in range_str.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            if let Some((start_str, end_str)) = part.split_once('-') {
                let start = if start_str.trim().is_empty() {
                    1
                } else {
                    start_str
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| QyzsError::Usage(format!("无效的起始页码: '{}'", start_str)))?
                };

                let end = if end_str.trim().is_empty() {
                    total_pages
                } else {
                    end_str
                        .trim()
                        .parse::<u32>()
                        .map_err(|_| QyzsError::Usage(format!("无效的结束页码: '{}'", end_str)))?
                };

                if start == 0 || end == 0 {
                    return Err(QyzsError::Usage("页码必须从 1 开始计数".to_string()));
                }
                if start > end {
                    return Err(QyzsError::Usage(format!("页码范围起始 '{}' 不能大于结束 '{}'", start, end)));
                }

                for p in start..=end.min(total_pages) {
                    selected.insert(p);
                }
            } else {
                let p = part
                    .parse::<u32>()
                    .map_err(|_| QyzsError::Usage(format!("无效的页码: '{}'", part)))?;
                if p == 0 {
                    return Err(QyzsError::Usage("页码必须从 1 开始计数".to_string()));
                }
                if p <= total_pages {
                    selected.insert(p);
                }
            }
        }

        if selected.is_empty() {
            return Err(QyzsError::Usage("未选定任何有效页码".to_string()));
        }

        Ok(selected.into_iter().collect())
    }

    /// 将选定页码提取并保存为新的 PDF 文件
    pub fn split(input_path: &str, pages_to_keep: &[u32], output_path: &str) -> QyzsResult<()> {
        let in_p = Path::new(input_path);
        if !in_p.exists() {
            return Err(QyzsError::Io(format!("待拆分 PDF 文件不存在: {}", input_path)));
        }

        let doc = Document::load(in_p)
            .map_err(|e| QyzsError::Pdf(format!("加载 PDF 失败: {}", e)))?;

        let slice = Self::build_slice(&doc, pages_to_keep)?;
        Self::save_slice(slice, output_path)
    }

    /// 按固定批次大小（如 MinerU 限制的 15 页）切片拆分大文档
    ///
    /// 全程只加载源文档一次: 逐切片复用同一份已解析的文档树, 避免
    /// 「每切片重新解析全文」带来的 O(切片数 × 文件体积) 解析开销。
    pub fn chunk(input_path: &str, chunk_size: usize, output_dir: &str) -> QyzsResult<Vec<PathBuf>> {
        if chunk_size == 0 {
            return Err(QyzsError::Usage("切片页数必须大于 0".to_string()));
        }

        let in_p = Path::new(input_path);
        let doc = Document::load(in_p)
            .map_err(|e| QyzsError::Pdf(format!("加载 PDF 失败: {}", e)))?;

        let total_pages = doc.get_pages().len();
        if total_pages == 0 {
            return Err(QyzsError::Pdf("PDF 不包含任何有效页面".to_string()));
        }

        let stem = in_p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("document");

        let out_dir_p = Path::new(output_dir);
        std::fs::create_dir_all(out_dir_p)?;

        let mut created_files = Vec::new();
        let mut start_page = 1;

        while start_page <= total_pages {
            let end_page = (start_page + chunk_size - 1).min(total_pages);
            let pages: Vec<u32> = (start_page as u32..=end_page as u32).collect();

            let chunk_name = format!("{}_part_{:02}_to_{:02}.pdf", stem, start_page, end_page);
            let chunk_path = out_dir_p.join(chunk_name);

            let slice = Self::build_slice(&doc, &pages)?;
            Self::save_slice(slice, chunk_path.to_str().unwrap_or_default())?;
            created_files.push(chunk_path);

            start_page = end_page + 1;
        }

        Ok(created_files)
    }

    /// 构造只含指定页的切片文档
    ///
    /// 策略: 以目标页的 Page 对象为根, 用 HashSet 做可达性遍历收集
    /// 「目标页 + 其内容流 + 共享资源」的对象子集, 再把原页树裁剪成只挂目标页
    /// 的子树。全程 O(引用数), 不做逐页删除。
    ///
    /// 页树沿用原对象 (只裁剪 /Kids), 而不是新建页树节点: 页的 /Resources、
    /// /MediaBox 等可继承属性挂在页树节点上, 新建节点会丢失它们。
    fn build_slice(doc: &Document, pages_to_keep: &[u32]) -> QyzsResult<Document> {
        let all_pages = doc.get_pages();
        // 去重: 同一页重复出现在 /Kids 里会让页树出现重复条目
        let mut seen_pages: HashSet<ObjectId> = HashSet::new();
        let keep: Vec<ObjectId> = pages_to_keep
            .iter()
            .filter_map(|p| all_pages.get(p).copied())
            .filter(|id| seen_pages.insert(*id))
            .collect();

        if keep.is_empty() {
            return Err(QyzsError::Pdf("未选定任何有效页码".to_string()));
        }

        // 1) 收集目标页所在的页树节点 (页 → /Parent 链)
        let keep_set: HashSet<ObjectId> = keep.iter().copied().collect();
        let mut tree_nodes: HashSet<ObjectId> = HashSet::new();
        for page_id in &keep {
            let mut parent = parent_of(doc, *page_id);
            while let Some(pid) = parent {
                if !tree_nodes.insert(pid) {
                    break; // 该祖先已处理过 (其更上层祖先同理)
                }
                parent = parent_of(doc, pid);
            }
        }

        // 2) 可达性遍历: 目标页 + 其内容/资源引用 + 页树节点。
        //
        //    两个关键约束, 少一个就会把整篇文档拖进切片:
        //    a) 页树边必须跳过 —— 页对象的 /Parent 指向页树节点, 页树节点的
        //       /Kids 列着全部页; 沿 /Parent 上溯再沿 /Kids 下来即回到所有页。
        //    b) 不从原 Catalog 出发 —— Catalog 是引用枢纽 (/StructTreeRoot、
        //       /Outlines、/Names 都会指向全文的页与目标), 沿它遍历必然拉进
        //       整篇文档 (实测: 15 页切片保留了 58866 个对象中的 58748 个)。
        //       改为新建最小 Catalog, 只保留 /Type 与 /Pages。
        let mut keep_ids: HashSet<ObjectId> = HashSet::new();
        let mut stack: Vec<ObjectId> = keep.clone();
        stack.extend(tree_nodes.iter().copied());
        while let Some(id) = stack.pop() {
            if !keep_ids.insert(id) {
                continue;
            }
            if let Some(obj) = doc.objects.get(&id) {
                let is_page = doc
                    .get_dictionary(id)
                    .ok()
                    .and_then(|d| d.get_type().ok())
                    .is_some_and(|t| t == b"Page");
                let is_tree = tree_nodes.contains(&id);
                let mut refs = Vec::new();
                collect_refs_filtered(obj, is_page, is_tree, &mut refs);
                stack.extend(refs);
            }
        }

        // 页树节点被裁剪后的 /Kids 值需要显式纳入保留集合
        for node_id in &tree_nodes {
            if let Some(kids) = doc
                .get_dictionary(*node_id)
                .ok()
                .and_then(|d| d.get(b"Kids").ok())
                .and_then(|o| o.as_array().ok())
            {
                for kid in kids {
                    if let Ok(kid_id) = kid.as_reference() {
                        if keep_set.contains(&kid_id) || tree_nodes.contains(&kid_id) {
                            keep_ids.insert(kid_id);
                        }
                    }
                }
            }
        }

        // 2) 复制保留子集
        let mut slice = Document::new();
        slice.version = doc.version.clone();
        slice.binary_mark = doc.binary_mark.clone();
        slice.max_id = doc.max_id;
        for (id, obj) in doc.objects.iter() {
            if keep_ids.contains(id) {
                slice.objects.insert(*id, obj.clone());
            }
        }

        // 3) 裁剪页树: 把每个页树节点的 /Kids 过滤为「目标页 + 仍在链上的页树节点」
        //
        //    先裁剪 /Kids, 再按裁剪后的结构重算 /Count (顺序不能反: /Count 必须
        //    反映裁剪后的叶子页数, 否则读取端会按旧计数遍历出已删除的页)
        for node_id in &tree_nodes {
            let original_kids: Vec<ObjectId> = doc
                .get_dictionary(*node_id)
                .ok()
                .and_then(|d| d.get(b"Kids").ok())
                .and_then(|o| o.as_array().ok())
                .map(|a| a.iter().filter_map(|k| k.as_reference().ok()).collect())
                .unwrap_or_default();

            let kept_kids: Vec<Object> = original_kids
                .iter()
                .filter(|kid| keep_set.contains(kid) || tree_nodes.contains(kid))
                .map(|kid| Object::Reference(*kid))
                .collect();

            if let Some(Object::Dictionary(dict)) = slice.objects.get_mut(node_id) {
                dict.set("Kids", Object::Array(kept_kids));
            }
        }

        for node_id in &tree_nodes {
            let leaf_count = count_leaf_pages(&slice, *node_id) as i64;
            if let Some(Object::Dictionary(dict)) = slice.objects.get_mut(node_id) {
                dict.set("Count", Object::Integer(leaf_count));
            }
        }

        // 4) 新建最小 Catalog: 只带 /Type 与 /Pages。
        //    不沿用原 Catalog 的 /StructTreeRoot、/Outlines、/Names 等键 ——
        //    它们指向全文的页与目标, 会把整篇文档的引用图拖进切片。
        let root_pages_id = doc
            .catalog()
            .ok()
            .and_then(|c| c.get(b"Pages").ok())
            .and_then(|o| o.as_reference().ok())
            .ok_or_else(|| QyzsError::Pdf("PDF 缺少可用的页树根节点".to_string()))?;

        let catalog_id = alloc_id(&mut slice);
        let mut catalog = lopdf::Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(root_pages_id));
        slice.objects.insert(catalog_id, Object::Dictionary(catalog));
        slice.trailer.set("Root", Object::Reference(catalog_id));

        // /Prev 指向旧文件的 xref 偏移, 对新文件无效
        slice.trailer.remove(b"Prev");
        slice.trailer.remove(b"XRefStm");
        slice.trailer.remove(b"Encrypt");

        Ok(slice)
    }

    /// 保存切片文档
    fn save_slice(mut slice: Document, output_path: &str) -> QyzsResult<()> {
        let out_p = Path::new(output_path);
        if let Some(parent) = out_p.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        slice
            .save(out_p)
            .map_err(|e| QyzsError::Pdf(format!("保存拆分后的 PDF 失败 {}: {}", output_path, e)))?;

        Ok(())
    }
}

/// 在文档中分配一个未被占用的新对象 id (高于当前 max_id)
fn alloc_id(doc: &mut Document) -> ObjectId {
    doc.max_id += 1;
    (doc.max_id, 0)
}

/// 取对象的 /Parent 引用 (页 → 页树节点, 页树节点 → 上级页树节点)
fn parent_of(doc: &Document, id: ObjectId) -> Option<ObjectId> {
    doc.get_dictionary(id)
        .ok()
        .and_then(|d| d.get(b"Parent").ok())
        .and_then(|o| o.as_reference().ok())
}

/// 统计页树节点下的叶子页数 (用于 /Count)
fn count_leaf_pages(doc: &Document, node_id: ObjectId) -> usize {
    let Some(dict) = doc.get_dictionary(node_id).ok() else {
        return 0;
    };
    let Ok(kids) = dict.get(b"Kids").and_then(|o| o.as_array()) else {
        return 0;
    };
    let mut count = 0;
    for kid in kids {
        let Ok(kid_id) = kid.as_reference() else {
            continue;
        };
        match doc
            .get_dictionary(kid_id)
            .ok()
            .and_then(|d| d.get_type().ok())
        {
            Some(b"Page") => count += 1,
            Some(b"Pages") => count += count_leaf_pages(doc, kid_id),
            _ => {}
        }
    }
    count
}

/// 收集引用, 但按需跳过页树边
///
/// `skip_parent`: 页对象 → 跳过 /Parent (页树节点另行接管)
/// `skip_kids`:   页树节点 → 跳过 /Kids (其子项由裁剪结果另行接管)
fn collect_refs_filtered(
    obj: &Object,
    skip_parent: bool,
    skip_kids: bool,
    out: &mut Vec<ObjectId>,
) {
    match obj {
        Object::Reference(id) => out.push(*id),
        Object::Array(arr) => {
            for item in arr {
                collect_refs_filtered(item, skip_parent, skip_kids, out);
            }
        }
        Object::Dictionary(dict) => {
            for (key, v) in dict.iter() {
                if skip_parent && key.as_slice() == b"Parent" {
                    continue;
                }
                if skip_kids && key.as_slice() == b"Kids" {
                    continue;
                }
                collect_refs_filtered(v, skip_parent, skip_kids, out);
            }
        }
        Object::Stream(stream) => {
            for (key, v) in stream.dict.iter() {
                if skip_parent && key.as_slice() == b"Parent" {
                    continue;
                }
                if skip_kids && key.as_slice() == b"Kids" {
                    continue;
                }
                collect_refs_filtered(v, skip_parent, skip_kids, out);
            }
        }
        _ => {}
    }
}
