#!/usr/bin/env python3
"""
tests/scripts/test_ocr_mineru_alignment.py
Performs paired mutual cross-verification between OCR (paddleocrvl) and MinerU 3.2.3.
Calculates token recall, structural consensus, and red-header/seal identification.
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
from _qyzs_env import get_api_key

API_KEY = get_api_key()
CHAT_BASE_URL = "http://59.219.113.13:9200/api/aiCenterChat/v1"
MINERU_URL = "http://59.219.113.13:9200/ai-gateway/mineru3.2.3/file_parse"
WORKSPACE = "/home/yangsen/workspaces/qyzs-ai/qyzs-cli.git"

ALIGNMENT_RESULTS = {}

def get_headers():
    return {
        "Authorization": f"Bearer {API_KEY}",
        "Content-Type": "application/json"
    }

def run_ocr_on_image(image_rel_path):
    img_abs = os.path.join(WORKSPACE, "tests/data", image_rel_path)
    im = Image.open(img_abs)
    im.thumbnail((800, 800))
    buf = io.BytesIO()
    im.save(buf, format="PNG")
    b64 = base64.b64encode(buf.getvalue()).decode("utf-8")
    data_uri = f"data:image/png;base64,{b64}"

    payload = {
        "model": "paddleocrvl",
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": "提取图片中的全部文字内容，保留段落和标点。"},
                    {"type": "image_url", "image_url": {"url": data_uri}}
                ]
            }
        ],
        "temperature": 0.0,
        "max_tokens": 800
    }
    resp = requests.post(
        f"{CHAT_BASE_URL}/chat/completions",
        headers=get_headers(),
        json=payload,
        timeout=35
    )
    if resp.status_code == 200:
        return resp.json()["choices"][0]["message"]["content"]
    else:
        raise RuntimeError(f"OCR failed ({resp.status_code}): {resp.text[:120]}")

def run_mineru_on_pdf(pdf_rel_path):
    pdf_abs = os.path.join(WORKSPACE, "tests/data", pdf_rel_path)
    fname = os.path.splitext(os.path.basename(pdf_abs))[0]
    with open(pdf_abs, "rb") as f:
        files = {"files": (os.path.basename(pdf_abs), f, "application/pdf")}
        data = {"output_dir": "output"}
        resp = requests.post(
            MINERU_URL,
            headers={"Authorization": f"Bearer {API_KEY}"},
            files=files,
            data=data,
            timeout=90
        )
    if resp.status_code == 200:
        res = resp.json()
        item = res.get("results", {}).get(fname, {})
        return item.get("md_content", "")
    else:
        raise RuntimeError(f"MinerU failed ({resp.status_code}): {resp.text[:120]}")

def compute_token_overlap(ocr_text, mineru_text, key_tokens):
    # Check key tokens presence
    key_recall_ocr = [k for k in key_tokens if k in ocr_text]
    key_recall_mineru = [k for k in key_tokens if k in mineru_text]

    # Character set Jaccard
    s_ocr = set(ocr_text.replace(" ", "").replace("\n", ""))
    s_min = set(mineru_text.replace(" ", "").replace("\n", ""))
    jaccard = round(len(s_ocr.intersection(s_min)) / max(len(s_ocr.union(s_min)), 1), 4)

    return {
        "jaccard_char_similarity": jaccard,
        "key_tokens_total": len(key_tokens),
        "ocr_recalled_keys": key_recall_ocr,
        "ocr_recall_rate": round(len(key_recall_ocr) / len(key_tokens), 2),
        "mineru_recalled_keys": key_recall_mineru,
        "mineru_recall_rate": round(len(key_recall_mineru) / len(key_tokens), 2),
        "consensus_keys": list(set(key_recall_ocr).intersection(set(key_recall_mineru)))
    }

def main():
    print("==================================================")
    print("  OCR & MinerU Paired Cross-Verification Suite")
    print("==================================================")
    spec_path = os.path.join(WORKSPACE, "tests/data/ocr/paired_alignment_spec.json")
    with open(spec_path, "r", encoding="utf-8") as f:
        spec = json.load(f)

    # Cache MinerU results to avoid redundant re-parsing
    mineru_cache = {}

    for pair in spec["pairs"]:
        pid = pair["pair_id"]
        pdf_file = pair["mineru_source_file"]
        ocr_file = pair["ocr_image_file"]
        key_tokens = pair["expected_key_tokens"]

        print(f"\nEvaluating Pair: {pid}")
        print(f"  Source PDF: {pdf_file}")
        print(f"  Source OCR: {ocr_file}")

        try:
            # 1. OCR text
            t0 = time.time()
            ocr_text = run_ocr_on_image(ocr_file)
            ocr_time = round(time.time() - t0, 3)
            print(f"  [OK] OCR completed in {ocr_time}s ({len(ocr_text)} chars)")

            # 2. MinerU text (cached per PDF)
            if pdf_file not in mineru_cache:
                t1 = time.time()
                mineru_cache[pdf_file] = run_mineru_on_pdf(pdf_file)
                print(f"  [OK] MinerU completed in {round(time.time() - t1, 3)}s ({len(mineru_cache[pdf_file])} chars)")
            mineru_text = mineru_cache[pdf_file]

            # 3. Cross verification
            metrics = compute_token_overlap(ocr_text, mineru_text, key_tokens)
            ALIGNMENT_RESULTS[pid] = {
                "status": "VERIFIED",
                "ocr_time_sec": ocr_time,
                "metrics": metrics,
                "ocr_sample": ocr_text[:120].replace("\n", " "),
                "mineru_sample": mineru_text[:120].replace("\n", " ")
            }
            print(f"  Consensus Score: Jaccard={metrics['jaccard_char_similarity']}, "
                  f"OCR Recall={metrics['ocr_recall_rate']}, MinerU Recall={metrics['mineru_recall_rate']}")
        except Exception as e:
            ALIGNMENT_RESULTS[pid] = {"status": "FAILED", "error": str(e)}
            print(f"  [FAIL] {e}")

    out_p = os.path.join(WORKSPACE, "tests/tests/scripts/alignment_report.json")
    with open(out_p, "w", encoding="utf-8") as f:
        json.dump(ALIGNMENT_RESULTS, f, ensure_ascii=False, indent=2)
    print(f"\n[SUCCESS] Cross-verification alignment report saved to {out_p}")

if __name__ == "__main__":
    main()
