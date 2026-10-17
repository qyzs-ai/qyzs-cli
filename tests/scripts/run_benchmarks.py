#!/usr/bin/env python3
"""
tests/scripts/run_benchmarks.py
Calibrated empirical benchmark suite for all models in Qingyang AI Computing Center.
Records latency, throughput, token usage, status codes, and model behaviors.
"""

import os
import sys
import time
import json
import base64
import requests
from PIL import Image
import io

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _qyzs_env import get_api_key, mask_key

API_KEY = get_api_key()
CHAT_BASE_URL = "http://59.219.113.13:9200/api/aiCenterChat/v1"
MINERU_URL = "http://59.219.113.13:9200/ai-gateway/mineru3.2.3/file_parse"
WORKSPACE = "/home/yangsen/workspaces/qyzs-ai/qyzs-cli.git"
BENCHMARK_RESULTS = {}

def get_headers():
    return {
        "Authorization": f"Bearer {API_KEY}",
        "Content-Type": "application/json"
    }

def benchmark_chat_models():
    print("\n--- 1. Benchmarking Chat & Reasoning Models ---")
    models = ["dsv4", "qwen3.6-27b", "qwen36", "deepseek_v3_2", "qwen2.5-72b"]
    prompt = "请用30字以内简述自然资源三维立体时空数据库的核心价值。"
    
    results = {}
    for m in models:
        payload = {
            "model": m,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0.2,
            "max_tokens": 400
        }
        t0 = time.time()
        try:
            resp = requests.post(
                f"{CHAT_BASE_URL}/chat/completions",
                headers=get_headers(),
                json=payload,
                timeout=25
            )
            lat = round(time.time() - t0, 3)
            if resp.status_code == 200:
                data = resp.json()
                msg = data["choices"][0]["message"]
                content = (msg.get("content") or msg.get("reasoning") or "").strip()
                results[m] = {
                    "status": "ONLINE",
                    "http_code": 200,
                    "latency_sec": lat,
                    "response_sample": content.replace("\n", " ")[:80],
                    "usage": data.get("usage", {})
                }
                print(f"  [ONLINE] {m} ({lat}s): {content[:60]}...")
            else:
                err_text = resp.text[:120]
                results[m] = {
                    "status": "OFFLINE_OR_UNMOUNTED",
                    "http_code": resp.status_code,
                    "latency_sec": lat,
                    "error": err_text
                }
                print(f"  [OFFLINE] {m} (HTTP {resp.status_code}): {err_text}")
        except Exception as e:
            lat = round(time.time() - t0, 3)
            results[m] = {
                "status": "UNREACHABLE",
                "latency_sec": lat,
                "error": str(e)
            }
            print(f"  [UNREACHABLE] {m}: {e}")
    BENCHMARK_RESULTS["chat_models"] = results

def benchmark_vlm_models():
    print("\n--- 2. Benchmarking Vision-Language Models ---")
    img_path = os.path.join(WORKSPACE, "tests/data/vlm/images/system_arch_flowchart.png")
    
    # PaddleOCR-VL requires <= 800px to not exceed max_model_len(2048) in FastDeploy
    im = Image.open(img_path)
    im.thumbnail((800, 800))
    buf = io.BytesIO()
    im.save(buf, format="PNG")
    b64_opt = base64.b64encode(buf.getvalue()).decode("utf-8")
    data_uri_opt = f"data:image/png;base64,{b64_opt}"
    
    with open(img_path, "rb") as f:
        b64_raw = base64.b64encode(f.read()).decode("utf-8")
    data_uri_raw = f"data:image/png;base64,{b64_raw}"

    models = ["qwen3-VL-30B-A3B", "paddleocrvl", "qwen3.6-27b"]
    prompt = "识别并概述本架构图顶层与底层的核心内容。"
    results = {}

    for m in models:
        uri = data_uri_opt if m == "paddleocrvl" else data_uri_raw
        payload = {
            "model": m,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": prompt},
                        {"type": "image_url", "image_url": {"url": uri}}
                    ]
                }
            ],
            "temperature": 0.0 if m == "paddleocrvl" else 0.2,
            "max_tokens": 500
        }
        t0 = time.time()
        try:
            resp = requests.post(
                f"{CHAT_BASE_URL}/chat/completions",
                headers=get_headers(),
                json=payload,
                timeout=40
            )
            lat = round(time.time() - t0, 3)
            if resp.status_code == 200:
                data = resp.json()
                msg = data["choices"][0]["message"]
                content = (msg.get("content") or msg.get("reasoning") or "").strip()
                results[m] = {
                    "status": "ONLINE",
                    "http_code": 200,
                    "latency_sec": lat,
                    "response_sample": content.replace("\n", " ")[:80],
                    "usage": data.get("usage", {})
                }
                print(f"  [ONLINE] {m} ({lat}s): {content[:60]}...")
            else:
                results[m] = {
                    "status": "ERROR",
                    "http_code": resp.status_code,
                    "latency_sec": lat,
                    "error": resp.text[:120]
                }
                print(f"  [ERROR] {m} (HTTP {resp.status_code}): {resp.text[:80]}")
        except Exception as e:
            results[m] = {
                "status": "UNREACHABLE",
                "latency_sec": round(time.time() - t0, 3),
                "error": str(e)
            }
            print(f"  [UNREACHABLE] {m}: {e}")
    BENCHMARK_RESULTS["vlm_models"] = results

def benchmark_embedding_and_reranker():
    print("\n--- 3. Benchmarking Embeddings and Rerankers ---")
    emb_models = ["bge-m3", "qwen3-embadding-8b", "bge-large-zh-v1.5", "text-embedding-v4"]
    rerank_models = ["bge-reranker-v2-m3", "bge-reranker-large", "qwen3-reranker-8b"]
    
    test_text = "自然资源部国土空间规划实施监测网络"
    emb_results = {}
    for m in emb_models:
        t0 = time.time()
        try:
            resp = requests.post(
                f"{CHAT_BASE_URL}/embeddings",
                headers=get_headers(),
                json={"model": m, "input": test_text},
                timeout=10
            )
            lat = round(time.time() - t0, 3)
            if resp.status_code == 200:
                data = resp.json()
                emb = data["data"][0]["embedding"]
                emb_results[m] = {
                    "status": "ONLINE",
                    "dimension": len(emb),
                    "latency_sec": lat
                }
                print(f"  [ONLINE] Embedding {m} ({lat}s): Dimension={len(emb)}")
            else:
                emb_results[m] = {
                    "status": "OFFLINE_OR_UNMOUNTED",
                    "http_code": resp.status_code,
                    "error": resp.text[:80]
                }
                print(f"  [OFFLINE] Embedding {m} (HTTP {resp.status_code})")
        except Exception as e:
            emb_results[m] = {"status": "UNREACHABLE", "error": str(e)}
            print(f"  [UNREACHABLE] Embedding {m}: {e}")
    BENCHMARK_RESULTS["embeddings"] = emb_results

    rerank_results = {}
    query = "国土空间规划体系"
    docs = ["全国国土空间规划实施监测网络建设方案", "政府采购评审专家管理办法", "自然资源部关于做好自然资源要素保障的通知"]
    for m in rerank_models:
        t0 = time.time()
        try:
            resp = requests.post(
                f"{CHAT_BASE_URL}/rerank",
                headers=get_headers(),
                json={"model": m, "query": query, "documents": docs, "top_n": 2},
                timeout=10
            )
            lat = round(time.time() - t0, 3)
            if resp.status_code == 200:
                data = resp.json()
                rerank_results[m] = {
                    "status": "ONLINE",
                    "latency_sec": lat,
                    "top1_index": data["results"][0]["index"],
                    "top1_score": round(data["results"][0]["relevance_score"], 4)
                }
                print(f"  [ONLINE] Reranker {m} ({lat}s): Top1 Idx={data['results'][0]['index']}, Score={data['results'][0]['relevance_score']:.4f}")
            else:
                rerank_results[m] = {
                    "status": "ERROR",
                    "http_code": resp.status_code,
                    "error": resp.text[:80]
                }
                print(f"  [ERROR] Reranker {m} (HTTP {resp.status_code})")
        except Exception as e:
            rerank_results[m] = {"status": "UNREACHABLE", "error": str(e)}
            print(f"  [UNREACHABLE] Reranker {m}: {e}")
    BENCHMARK_RESULTS["rerankers"] = rerank_results

def benchmark_mineru():
    print("\n--- 4. Benchmarking MinerU 3.2.3 Document Engine ---")
    doc_path = os.path.join(WORKSPACE, "tests/data/mineru/public_doc/REF-0106_项目验收前符合性审查表.docx")
    fname = os.path.splitext(os.path.basename(doc_path))[0]
    t0 = time.time()
    try:
        with open(doc_path, "rb") as f:
            files = {"files": (os.path.basename(doc_path), f, "application/vnd.openxmlformats-officedocument.wordprocessingml.document")}
            data = {"output_dir": "output", "is_ocr": "true"}
            resp = requests.post(
                MINERU_URL,
                headers={"Authorization": f"Bearer {API_KEY}"},
                files=files,
                data=data,
                timeout=45
            )
        lat = round(time.time() - t0, 3)
        if resp.status_code == 200:
            res_json = resp.json()
            item = res_json.get("results", {}).get(fname, {})
            md_content = item.get("md_content", "")
            BENCHMARK_RESULTS["mineru"] = {
                "status": "ONLINE",
                "http_code": 200,
                "latency_sec": lat,
                "markdown_length": len(md_content),
                "markdown_preview": md_content[:100].replace("\n", " ")
            }
            print(f"  [ONLINE] MinerU 3.2.3 ({lat}s): Parsed Markdown {len(md_content)} chars")
            print(f"           Preview: {md_content[:80]}...")
        else:
            BENCHMARK_RESULTS["mineru"] = {
                "status": "ERROR",
                "http_code": resp.status_code,
                "latency_sec": lat,
                "error": resp.text[:100]
            }
            print(f"  [ERROR] MinerU (HTTP {resp.status_code}): {resp.text[:80]}")
    except Exception as e:
        BENCHMARK_RESULTS["mineru"] = {"status": "UNREACHABLE", "error": str(e)}
        print(f"  [UNREACHABLE] MinerU: {e}")

def benchmark_audio_asr():
    print("\n--- 5. Benchmarking Qwen3-ASR-1.7B Speech Recognition ---")
    wav_path = os.path.join(WORKSPACE, "tests/data/audio/samples/sample_speech_cmd.wav")
    with open(wav_path, "rb") as f:
        b64 = base64.b64encode(f.read()).decode("utf-8")
    audio_uri = f"data:audio/wav;base64,{b64}"
    
    payload = {
        "model": "Qwen3-ASR-1.7B",
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "audio_url", "audio_url": {"url": audio_uri}}
                ]
            }
        ]
    }
    t0 = time.time()
    try:
        resp = requests.post(
            f"{CHAT_BASE_URL}/chat/completions",
            headers=get_headers(),
            json=payload,
            timeout=20
        )
        lat = round(time.time() - t0, 3)
        if resp.status_code == 200:
            data = resp.json()
            asr_text = data["choices"][0]["message"]["content"]
            BENCHMARK_RESULTS["audio_asr"] = {
                "status": "ONLINE",
                "http_code": 200,
                "latency_sec": lat,
                "recognized_text": asr_text
            }
            print(f"  [ONLINE] Qwen3-ASR-1.7B ({lat}s): Result='{asr_text}'")
        else:
            BENCHMARK_RESULTS["audio_asr"] = {
                "status": "ERROR",
                "http_code": resp.status_code,
                "latency_sec": lat,
                "error": resp.text[:100]
            }
            print(f"  [ERROR] Qwen3-ASR-1.7B (HTTP {resp.status_code}): {resp.text[:80]}")
    except Exception as e:
        BENCHMARK_RESULTS["audio_asr"] = {"status": "UNREACHABLE", "error": str(e)}
        print(f"  [UNREACHABLE] Qwen3-ASR-1.7B: {e}")

if __name__ == "__main__":
    print("==================================================")
    print("  Qingyang AI Computing Center Calibrated Benchmark")
    print(f"  Auth: Bearer {mask_key(API_KEY)}")
    print("==================================================")
    benchmark_chat_models()
    benchmark_vlm_models()
    benchmark_embedding_and_reranker()
    benchmark_mineru()
    benchmark_audio_asr()

    output_path = os.path.join(WORKSPACE, "tests/scripts/benchmark_summary.json")
    with open(output_path, "w", encoding="utf-8") as f:
        json.dump(BENCHMARK_RESULTS, f, ensure_ascii=False, indent=2)
    print(f"\n[SUCCESS] Calibrated benchmark saved to {output_path}")
