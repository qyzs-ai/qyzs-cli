#!/usr/bin/env python3
"""
tests/scripts/prepare_dataset.py
Populate and validate data/ directory for Qingyang AI Computing Center model testing.
Strict compliance:
1. Only public policy docs (NO feasibility reports).
2. Clean folder names: data/{chat,vlm,ocr,mineru,rag,audio,rs}.
3. Paired OCR and MinerU test documents for mutual cross-verification.
4. Stress test datasets (16 multi-image batch, 64k needle in a haystack).
"""

import os
import sys
import shutil
import json
import wave
import struct
import math
import fitz  # PyMuPDF
from PIL import Image

WORKSPACE = "/home/yangsen/workspaces/qyzs-ai/qyzs-cli.git"
REF_DIR = "/home/yangsen/workspaces/YNDNR/2027-FSR-NR-Agents-v2/references"

def ensure_dir(path):
    os.makedirs(path, exist_ok=True)

def prepare_mineru_and_ocr():
    print(">>> 1. Preparing mineru/ and ocr/ datasets...")
    mineru_base = os.path.join(WORKSPACE, "tests/data/mineru")
    ocr_base = os.path.join(WORKSPACE, "tests/data/ocr")
    
    ensure_dir(f"{mineru_base}/text_pdf")
    ensure_dir(f"{mineru_base}/scanned_pdf")
    ensure_dir(f"{mineru_base}/public_doc")
    ensure_dir(f"{ocr_base}/images")

    # Sources
    src_text_pdf = f"{REF_DIR}/policies/policies/REF-0056_《政府采购评审专家管理办法》（财库〔2016〕198号）/original.pdf"
    src_scan_pdf = f"{REF_DIR}/policies/policies/REF-0101_《关于进一步加强自然资源部网络安全等级保护工作的指导意见》（自然资网信办发〔2024〕5号）/original.pdf"
    src_doc = f"{REF_DIR}/policies/policies/REF-0017_《全国国土空间规划实施监测网络建设工作方案-2023—2027年》（自然资办发〔2023〕36号）/original.doc"
    src_docx = f"{REF_DIR}/policies/policies/REF-0106_《云南省省级政务信息化服务类项目验收实施细则（试行）》（云数资源〔2025〕94号）/attachments/附件2-项目验收前符合性审查表.docx"

    dst_text_pdf = f"{mineru_base}/text_pdf/REF-0056_政府采购评审专家管理办法.pdf"
    dst_scan_pdf = f"{mineru_base}/scanned_pdf/REF-0101_网络安全等级保护指导意见_scan.pdf"
    dst_doc = f"{mineru_base}/public_doc/REF-0017_国土空间规划实施监测网络工作方案.doc"
    dst_docx = f"{mineru_base}/public_doc/REF-0106_项目验收前符合性审查表.docx"

    shutil.copy2(src_text_pdf, dst_text_pdf)
    shutil.copy2(src_scan_pdf, dst_scan_pdf)
    shutil.copy2(src_doc, dst_doc)
    shutil.copy2(src_docx, dst_docx)
    print(f"  [OK] Copied mineru files: {os.path.basename(dst_text_pdf)}, {os.path.basename(dst_scan_pdf)}, {os.path.basename(dst_doc)}, {os.path.basename(dst_docx)}")

    # Render paired OCR images from these exact PDFs
    # 1. Page 1 of text PDF (Title and header)
    doc_text = fitz.open(dst_text_pdf)
    p0_text = doc_text[0]
    pix0_text = p0_text.get_pixmap(dpi=200)
    img_text_p1 = f"{ocr_base}/images/ref0056_p1_title.png"
    pix0_text.save(img_text_p1)
    doc_text.close()

    # 2. Page 1 and last page of scanned PDF
    doc_scan = fitz.open(dst_scan_pdf)
    p0_scan = doc_scan[0]
    pix0_scan = p0_scan.get_pixmap(dpi=200)
    img_scan_p1 = f"{ocr_base}/images/ref0101_p1_redhead_scan.png"
    pix0_scan.save(img_scan_p1)

    # Find the page with official seal (typically near the end of dispatch)
    seal_page_idx = min(7, len(doc_scan) - 1)
    p_seal = doc_scan[seal_page_idx]
    pix_seal = p_seal.get_pixmap(dpi=200)
    img_scan_seal = f"{ocr_base}/images/ref0101_p8_seal_scan.png"
    pix_seal.save(img_scan_seal)
    doc_scan.close()

    print(f"  [OK] Rendered paired OCR images: {os.path.basename(img_text_p1)}, {os.path.basename(img_scan_p1)}, {os.path.basename(img_scan_seal)}")

    # Manifest for OCR & MinerU mutual cross-verification
    ocr_mineru_alignment = {
        "description": "OCR and MinerU paired dataset for mutual cross-verification and consensus arbitration",
        "pairs": [
            {
                "pair_id": "pair_01_text_pdf_p1",
                "mineru_source_file": "mineru/text_pdf/REF-0056_政府采购评审专家管理办法.pdf",
                "page_index": 0,
                "ocr_image_file": "ocr/images/ref0056_p1_title.png",
                "expected_key_tokens": ["政府采购评审专家管理办法", "财库〔2016〕198号", "第一章 总则", "财政部"]
            },
            {
                "pair_id": "pair_02_scanned_redhead_p1",
                "mineru_source_file": "mineru/scanned_pdf/REF-0101_网络安全等级保护指导意见_scan.pdf",
                "page_index": 0,
                "ocr_image_file": "ocr/images/ref0101_p1_redhead_scan.png",
                "expected_key_tokens": ["自然资源部网络安全和信息化领导小组办公室文件", "自然资网信办发〔2024〕5号", "关于进一步加强自然资源部网络安全等级保护工作的指导意见"]
            },
            {
                "pair_id": "pair_03_scanned_seal_p8",
                "mineru_source_file": "mineru/scanned_pdf/REF-0101_网络安全等级保护指导意见_scan.pdf",
                "page_index": 7,
                "ocr_image_file": "ocr/images/ref0101_p8_seal_scan.png",
                "expected_key_tokens": ["自然资源部网络安全和信息化领导小组办公室", "印发", "抄送"]
            }
        ]
    }
    with open(f"{ocr_base}/paired_alignment_spec.json", "w", encoding="utf-8") as f:
        json.dump(ocr_mineru_alignment, f, ensure_ascii=False, indent=2)
    print("  [OK] Saved tests/data/ocr/paired_alignment_spec.json")

def prepare_vlm():
    print(">>> 2. Preparing vlm/ dataset (images, 16-image stress batch, test cases)...")
    vlm_base = os.path.join(WORKSPACE, "tests/data/vlm")
    img_dir = f"{vlm_base}/images"
    multi_dir = f"{vlm_base}/multi_image_batch"
    ensure_dir(img_dir)
    ensure_dir(multi_dir)

    # 1. Primary test images from public references
    seal_src = f"{REF_DIR}/policies/policies/REF-0005_《云南省数据局关于推进省级政务信息系统使用省级政务云有关工作的通知》（云南省数据局-2024-03-20印发）/assets/REF-0005-p05-seal.png"
    flow_src = f"{REF_DIR}/policies/policies/REF-0022_《国土空间规划实施监测网络建设技术指南-试行》（自然资办发〔2025〕37号）/assets/figure-1.png"
    cloud_flow_src = f"{REF_DIR}/policies/policies/REF-0005_《云南省数据局关于推进省级政务信息系统使用省级政务云有关工作的通知》（云南省数据局-2024-03-20印发）/assets/REF-0005-p62-flow.png"

    shutil.copy2(seal_src, f"{img_dir}/sig_seal_official.png")
    shutil.copy2(flow_src, f"{img_dir}/system_arch_flowchart.png")
    shutil.copy2(cloud_flow_src, f"{img_dir}/gov_cloud_flow.png")
    print("  [OK] Copied primary VLM images (seal, system arch flowchart, cloud workflow)")

    # 2. Build 16-image batch for multi-image concurrency limit testing
    # We will pick 16 distinct public images (render pages from doc & pdf, plus public diagrams)
    doc_scan = fitz.open(f"{WORKSPACE}/tests/data/mineru/scanned_pdf/REF-0101_网络安全等级保护指导意见_scan.pdf")
    for i in range(min(12, len(doc_scan))):
        pix = doc_scan[i].get_pixmap(dpi=100)
        pix.save(f"{multi_dir}/page_scan_{i+1:02d}.png")
    doc_scan.close()

    # Also include the 3 architecture/seal diagrams and 1 render from text pdf
    shutil.copy2(f"{img_dir}/sig_seal_official.png", f"{multi_dir}/asset_seal_13.png")
    shutil.copy2(f"{img_dir}/system_arch_flowchart.png", f"{multi_dir}/asset_arch_14.png")
    shutil.copy2(f"{img_dir}/gov_cloud_flow.png", f"{multi_dir}/asset_flow_15.png")
    shutil.copy2(f"{WORKSPACE}/tests/data/ocr/images/ref0056_p1_title.png", f"{multi_dir}/asset_text_16.png")

    total_multi = len(os.listdir(multi_dir))
    print(f"  [OK] Multi-image batch generated: {total_multi} images in tests/data/vlm/multi_image_batch/")

    # 3. VLM test cases
    vlm_cases = {
        "test_cases": [
            {
                "id": "vlm_01_seal_ocr",
                "image": "images/sig_seal_official.png",
                "prompt": "请识别图中的印章文字、发文单位名称及日期，并判断印章形态是否完整无缺损。",
                "expected_entities": ["云南省数据局", "印章"]
            },
            {
                "id": "vlm_02_flowchart_reasoning",
                "image": "images/system_arch_flowchart.png",
                "prompt": "请详细分析该架构图的分层结构，列出各层级名称以及核心组件流向。",
                "expected_entities": ["国土空间", "监测网络", "架构", "数据"]
            },
            {
                "id": "vlm_03_multi_image_16_count",
                "image_batch_dir": "multi_image_batch",
                "prompt": "请统计本次请求输入的图片总数，并指出其中哪张图片包含红色的公章？",
                "expected_answer_contains": ["16", "公章", "云南省数据局"]
            }
        ]
    }
    with open(f"{vlm_base}/vlm_test_cases.json", "w", encoding="utf-8") as f:
        json.dump(vlm_cases, f, ensure_ascii=False, indent=2)
    print("  [OK] Saved tests/data/vlm/vlm_test_cases.json")

def prepare_audio():
    print(">>> 3. Preparing audio/ dataset (16kHz Mono WAV)...")
    audio_base = os.path.join(WORKSPACE, "tests/data/audio")
    sample_dir = f"{audio_base}/samples"
    ensure_dir(sample_dir)

    # Generate 16kHz, 16-bit Mono PCM speech-like modulated acoustic signal (3 seconds)
    wav_path = f"{sample_dir}/sample_speech_cmd.wav"
    sample_rate = 16000
    duration = 3.0  # seconds
    num_samples = int(sample_rate * duration)
    
    with wave.open(wav_path, "w") as wav_file:
        wav_file.setnchannels(1)  # Mono
        wav_file.setsampwidth(2)  # 16-bit
        wav_file.setframerate(sample_rate)
        
        frames = []
        for i in range(num_samples):
            t = float(i) / sample_rate
            # Synthesize voiced fundamental frequency with formant harmonics (vowel-like /a/)
            f0 = 180.0 + 30.0 * math.sin(2.0 * math.pi * 1.5 * t)
            f1 = 700.0
            f2 = 1220.0
            env = 0.5 * (1.0 - math.cos(2.0 * math.pi * t / duration))
            val = env * (0.6 * math.sin(2.0 * math.pi * f0 * t) + 
                         0.3 * math.sin(2.0 * math.pi * f1 * t) + 
                         0.1 * math.sin(2.0 * math.pi * f2 * t))
            sample = int(val * 24000.0)
            frames.append(struct.pack("<h", sample))
        
        wav_file.writeframes(b"".join(frames))
    
    print(f"  [OK] Generated 16kHz mono audio: {wav_path} ({os.path.getsize(wav_path)} bytes)")

    audio_spec = {
        "audio_files": [
            {
                "file": "samples/sample_speech_cmd.wav",
                "format": "wav",
                "sample_rate": 16000,
                "channels": 1,
                "duration_seconds": 3.0,
                "description": "Standard 16kHz mono acoustic test probe for Qwen3-ASR-1.7B gateway verification"
            }
        ]
    }
    with open(f"{audio_base}/audio_prompts.json", "w", encoding="utf-8") as f:
        json.dump(audio_spec, f, ensure_ascii=False, indent=2)
    print("  [OK] Saved tests/data/audio/audio_prompts.json")

def prepare_chat():
    print(">>> 4. Preparing chat/ dataset (short, long, 64k needle in haystack)...")
    chat_base = os.path.join(WORKSPACE, "tests/data/chat")
    ensure_dir(chat_base)

    # 1. Short prompts
    short_prompts = [
        {
            "id": "chat_short_01",
            "prompt": "请解释我国自然资源管理中'三区三线'的具体含义，并列出各条控制线的法律效力。",
            "task": "natural_resources_cadastral_definition"
        },
        {
            "id": "chat_short_02",
            "prompt": "将以下自然资源部项目数据清洗为合法JSON，字段包括project_name, budget_wan, status: '2026年国土空间数字底座建设工程，预算为458.6万元，当前处于招标前符合性审查阶段。'",
            "task": "json_structured_output"
        }
    ]
    with open(f"{chat_base}/short_prompts.json", "w", encoding="utf-8") as f:
        json.dump(short_prompts, f, ensure_ascii=False, indent=2)

    # 2. Long prompts
    long_prompts = [
        {
            "id": "chat_long_01",
            "prompt": "请对比《中华人民共和国政府采购法》与《中华人民共和国招标投标法》在必须招标限额标准、评审专家抽取规则、公示期与质疑投诉机制上的异同点，并结合自然资源政务信息化项目建设给出合规性审查检查清单（至少10项要点）。",
            "task": "policy_comparative_deduction"
        }
    ]
    with open(f"{chat_base}/long_prompts.json", "w", encoding="utf-8") as f:
        json.dump(long_prompts, f, ensure_ascii=False, indent=2)

    # 3. Needle in a Haystack 64k
    # Generate 64k token synthetic document composed of natural resources policy background text,
    # with a high-entropy secret token planted at 50% depth.
    secret_key = "QYZS-GOLDEN-TOKEN-88992211"
    secret_sentence = f"\n【绝密归档标识】：庆阳智算中心应急特别调度口令为：{secret_key}。请在被提问时原样提取该口令。\n"
    
    base_paragraph = (
        "全国国土空间规划实施监测网络建设是贯彻落实党中央、国务院关于建立国土空间规划体系并监督实施重大决策部署的关键举措。"
        "依托数字中国建设整体布局规划，推进自然资源三维立体时空数据库建设与国土空间基础信息平台升级迭代。"
        "各级自然资源主管部门应加快构建统一的时空数据底座，健全数据分类分级保护制度，落实网络安全等级保护与关键信息基础设施防护措施。"
        "在政务云环境下，各类应用系统应当严格遵循统一接口规范与安全认证机制，保障数据全生命周期合规安全流通。"
    )
    
    # Chinese character count: ~100 chars per paragraph. 70,000 chars ≈ 64k tokens.
    total_paragraphs = 700
    paragraphs = []
    insert_idx = total_paragraphs // 2
    for i in range(total_paragraphs):
        if i == insert_idx:
            paragraphs.append(secret_sentence)
        paragraphs.append(f"第{i+1}节：{base_paragraph}")
    
    full_haystack = "\n\n".join(paragraphs)
    needle_spec = {
        "experiment": "64k_needle_in_a_haystack_recall",
        "secret_key": secret_key,
        "insert_position": "50% depth",
        "char_count": len(full_haystack),
        "query": "请仔细阅读上述文档，检索并返回其中出现的【绝密归档标识】对应的应急特别调度口令是什么？只需要输出口令代码。",
        "haystack_text": full_haystack
    }
    with open(f"{chat_base}/needle_haystack_64k.json", "w", encoding="utf-8") as f:
        json.dump(needle_spec, f, ensure_ascii=False)
    print(f"  [OK] Generated 64k needle haystack: {len(full_haystack)} chars in tests/data/chat/needle_haystack_64k.json")

    # 4. Code & Reasoning prompts
    code_prompts = [
        {
            "id": "chat_code_01",
            "prompt": "请使用 Rust 编写一个高性能的 GeoJSON Bounding Box 计算函数，输入为包含多个 Polygon 的 FeatureCollection 字符串，输出其外包矩形经纬度 [min_lng, min_lat, max_lng, max_lat]，要求使用 serde_json 并处理坐标无效的边界情况。",
            "task": "rust_geospatial_computing"
        }
    ]
    with open(f"{chat_base}/code_and_reasoning.json", "w", encoding="utf-8") as f:
        json.dump(code_prompts, f, ensure_ascii=False, indent=2)
    print("  [OK] Saved tests/data/chat prompts")

def prepare_rag():
    print(">>> 5. Preparing rag/ dataset (chunks and queries for embedding/reranker)...")
    rag_base = os.path.join(WORKSPACE, "tests/data/rag")
    ensure_dir(rag_base)

    chunks = [
        {"id": "doc_01", "text": "自然资源部发布《全国国土空间规划实施监测网络建设工作方案（2023—2027年）》，推进统一国土空间规划一张图建设。"},
        {"id": "doc_02", "text": "《政府采购评审专家管理办法》规定评审专家抽取必须在财政部门建立的评审专家库中随机抽取，不得明示或者暗示倾向。"},
        {"id": "doc_03", "text": "云南省省级政务信息化项目建设需严格执行三同步原则，即网络安全保障措施与信息化工程同步规划、同步建设、同步使用。"},
        {"id": "doc_04", "text": "庆阳智算中心部署 DeepSeek-V3.2、Qwen3-VL 等先进模型，为国土空间规划和自然资源监测提供强大AI算力支撑。"},
        {"id": "doc_05", "text": "遥感影像变化检测采用双时相高分辨率卫星影像，利用 ACID-SAM3 基础大模型实现地表覆盖与建筑群动态识别。"}
    ]
    queries = [
        {
            "query": "国土空间规划一张图和监测网络建设方案",
            "expected_top1": "doc_01"
        },
        {
            "query": "政府采购评审专家随机抽取合规规定",
            "expected_top1": "doc_02"
        },
        {
            "query": "卫星遥感影像变化检测与建筑物识别算法",
            "expected_top1": "doc_05"
        }
    ]
    with open(f"{rag_base}/chunks_sample.json", "w", encoding="utf-8") as f:
        json.dump(chunks, f, ensure_ascii=False, indent=2)
    with open(f"{rag_base}/queries_sample.json", "w", encoding="utf-8") as f:
        json.dump(queries, f, ensure_ascii=False, indent=2)
    print("  [OK] Saved tests/data/rag/ chunks and queries")

def prepare_rs():
    print(">>> 6. Preparing rs/ dataset (AOI GeoJSON)...")
    rs_base = os.path.join(WORKSPACE, "tests/data/rs")
    ensure_dir(rs_base)

    aoi = {
        "type": "FeatureCollection",
        "features": [
            {
                "type": "Feature",
                "properties": {
                    "name": "Kunming Dianchi Pilot AOI",
                    "region": "Yunnan",
                    "task": "surface_change_detection"
                },
                "geometry": {
                    "type": "Polygon",
                    "coordinates": [
                        [
                            [102.68, 24.95],
                            [102.75, 24.95],
                            [102.75, 24.88],
                            [102.68, 24.88],
                            [102.68, 24.95]
                        ]
                    ]
                }
            }
        ]
    }
    with open(f"{rs_base}/sample_aoi.geojson", "w", encoding="utf-8") as f:
        json.dump(aoi, f, ensure_ascii=False, indent=2)
    print("  [OK] Saved tests/data/rs/sample_aoi.geojson")

if __name__ == "__main__":
    print("=== Starting Test Dataset Population ===")
    prepare_mineru_and_ocr()
    prepare_vlm()
    prepare_audio()
    prepare_chat()
    prepare_rag()
    prepare_rs()
    print("=== Test Dataset Population Completed Successfully! ===")
