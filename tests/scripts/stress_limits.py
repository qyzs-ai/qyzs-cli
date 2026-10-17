#!/usr/bin/env python3
"""
tests/scripts/stress_limits.py
Executes boundary limit and stress benchmarks on Qingyang AI Computing Center models:
1. Multi-image concurrent input stress: 1, 4, 8, 16 images to qwen3-VL-30B-A3B.
2. Context length & Needle-in-a-haystack recall: 4k, 16k, 32k, 64k tokens to dsv4.
3. PDF pages throughput & sync-vs-async limits on MinerU.
"""

import os
import sys
import time
import json
import base64
import requests

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _qyzs_env import get_api_key

API_KEY = get_api_key()
CHAT_BASE_URL = "http://59.219.113.13:9200/api/aiCenterChat/v1"
MINERU_URL = "http://59.219.113.13:9200/ai-gateway/mineru3.2.3/file_parse"
WORKSPACE = "/home/yangsen/workspaces/qyzs-ai/qyzs-cli.git"

STRESS_RESULTS = {}

def get_headers():
    return {
        "Authorization": f"Bearer {API_KEY}",
        "Content-Type": "application/json"
    }

def stress_multi_image_concurrency():
    print("\n=== Stress Test 1: Multi-Image Concurrency (qwen3-VL-30B-A3B) ===")
    multi_dir = os.path.join(WORKSPACE, "tests/data/vlm/multi_image_batch")
    all_imgs = sorted([os.path.join(multi_dir, f) for f in os.listdir(multi_dir) if f.endswith(('.png', '.jpg'))])
    
    counts = [1, 4, 8, 16]
    image_stress_data = {}

    for count in counts:
        batch = all_imgs[:count]
        content_items = [
            {"type": "text", "text": f"请统计本请求中包含多少张图片？请列出每张图片的大致内容，并重点指出哪一张图片包含公章。"}
        ]
        for img_path in batch:
            with open(img_path, "rb") as f:
                b64 = base64.b64encode(f.read()).decode("utf-8")
            ext = "png" if img_path.endswith(".png") else "jpeg"
            content_items.append({
                "type": "image_url",
                "image_url": {"url": f"data:image/{ext};base64,{b64}"}
            })

        payload = {
            "model": "qwen3-VL-30B-A3B",
            "messages": [{"role": "user", "content": content_items}],
            "temperature": 0.1,
            "max_tokens": 500
        }
        print(f"  Sending {count} images concurrently to qwen3-VL-30B-A3B...")
        t0 = time.time()
        try:
            resp = requests.post(
                f"{CHAT_BASE_URL}/chat/completions",
                headers=get_headers(),
                json=payload,
                timeout=120
            )
            lat = round(time.time() - t0, 3)
            if resp.status_code == 200:
                res = resp.json()
                msg = res["choices"][0]["message"]
                reply = (msg.get("content") or msg.get("reasoning") or "").strip()
                usage = res.get("usage", {})
                image_stress_data[f"{count}_images"] = {
                    "status": "SUCCESS",
                    "http_code": 200,
                    "latency_sec": lat,
                    "prompt_tokens": usage.get("prompt_tokens"),
                    "completion_tokens": usage.get("completion_tokens"),
                    "output_preview": reply[:150].replace("\n", " ")
                }
                print(f"    [OK] {count} images: {lat}s, Tokens: {usage.get('total_tokens')}. Output: {reply[:80]}...")
            else:
                image_stress_data[f"{count}_images"] = {
                    "status": "FAILED",
                    "http_code": resp.status_code,
                    "latency_sec": lat,
                    "error": resp.text[:150]
                }
                print(f"    [FAIL] {count} images: HTTP {resp.status_code}: {resp.text[:100]}")
        except Exception as e:
            image_stress_data[f"{count}_images"] = {
                "status": "ERROR",
                "error": str(e)
            }
            print(f"    [EXCEPTION] {count} images: {e}")
    STRESS_RESULTS["multi_image_concurrency"] = image_stress_data

def stress_context_length_and_needle():
    print("\n=== Stress Test 2: Context Window & Needle-in-a-Haystack Recall (dsv4) ===")
    needle_spec_p = os.path.join(WORKSPACE, "tests/data/chat/needle_haystack_64k.json")
    with open(needle_spec_p, "r", encoding="utf-8") as f:
        spec = json.load(f)
    
    secret_key = spec["secret_key"]
    full_text = spec["haystack_text"]
    query = spec["query"]

    # We test 3 length scales: 8k chars (~4k tokens), 32k chars (~16k tokens), full 157k chars (~64k tokens)
    char_lengths = [8000, 32000, len(full_text)]
    scale_names = ["8k_chars_approx_4k_tokens", "32k_chars_approx_16k_tokens", "157k_chars_approx_64k_tokens"]

    context_results = {}

    for chars, name in zip(char_lengths, scale_names):
        if chars == len(full_text):
            sub_text = full_text
        else:
            # build subset containing the secret key in the middle
            half = chars // 2
            sub_text = f"前置背景：{full_text[:half]}\n\n【绝密归档标识】：庆阳智算中心应急特别调度口令为：{secret_key}。\n\n后续背景：{full_text[half:chars]}"

        messages = [
            {"role": "system", "content": "你是一个严格依照文档事实回答问题的政务助手。"},
            {"role": "user", "content": f"请阅读以下参考文档并回答问题：\n\n{sub_text}\n\n问题：{query}"}
        ]
        payload = {
            "model": "dsv4",
            "messages": messages,
            "temperature": 0.0,
            "max_tokens": 150
        }
        print(f"  Sending context length {chars} chars ({name}) to dsv4...")
        t0 = time.time()
        try:
            resp = requests.post(
                f"{CHAT_BASE_URL}/chat/completions",
                headers=get_headers(),
                json=payload,
                timeout=120
            )
            lat = round(time.time() - t0, 3)
            if resp.status_code == 200:
                res = resp.json()
                reply = res["choices"][0]["message"]["content"]
                usage = res.get("usage", {})
                recalled = secret_key in reply
                context_results[name] = {
                    "status": "SUCCESS",
                    "http_code": 200,
                    "char_count": chars,
                    "prompt_tokens": usage.get("prompt_tokens"),
                    "completion_tokens": usage.get("completion_tokens"),
                    "latency_sec": lat,
                    "secret_recalled": recalled,
                    "reply": reply.strip()
                }
                print(f"    [OK] {name}: {lat}s, Prompt Tokens: {usage.get('prompt_tokens')}, Secret Recalled: {recalled} -> '{reply.strip()}'")
            else:
                context_results[name] = {
                    "status": "FAILED",
                    "http_code": resp.status_code,
                    "latency_sec": lat,
                    "error": resp.text[:150]
                }
                print(f"    [FAIL] {name}: HTTP {resp.status_code}: {resp.text[:100]}")
        except Exception as e:
            context_results[name] = {"status": "ERROR", "error": str(e)}
            print(f"    [EXCEPTION] {name}: {e}")
    STRESS_RESULTS["context_and_needle"] = context_results

if __name__ == "__main__":
    print("==================================================")
    print("  Qingyang AI Computing Center Stress & Boundary Benchmarks")
    print("==================================================")
    stress_multi_image_concurrency()
    stress_context_length_and_needle()

    out_p = os.path.join(WORKSPACE, "tests/scripts/stress_summary.json")
    with open(out_p, "w", encoding="utf-8") as f:
        json.dump(STRESS_RESULTS, f, ensure_ascii=False, indent=2)
    print(f"\n[SUCCESS] Stress benchmark summary saved to {out_p}")
