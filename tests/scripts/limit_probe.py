#!/usr/bin/env python3
"""
tests/scripts/limit_probe.py
Boundary-limit probes for Qingyang AI Computing Center models:
  A. Context ceiling ladder with needle-in-a-haystack recall (dsv4 / qwen3.6-27b / qwen3-VL-30B-A3B)
  B. Generation parameter boundaries (max_tokens, temperature)
  C. Multi-image concurrency beyond 16 (20 / 24) for qwen3-VL-30B-A3B
  D. Embedding input-length and batch-size limits
  E. Rerank candidate-set scale (10 / 50 / 100 / 200)
  F. MinerU synchronized page-count ladder (5 / 15 / 30)
Credentials are loaded via `_qyzs_env` (QYZS_APIKEY environment variable, or a
git-ignored `.env` file at the repository root); results are
written to tests/scripts/limit_summary.json.
"""

import os
import sys
import time
import json
import base64
import io
import requests
from pypdf import PdfReader, PdfWriter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _qyzs_env import get_api_key

API_KEY = get_api_key()

CHAT_BASE_URL = "http://59.219.113.13:9200/api/aiCenterChat/v1"
MINERU_URL = "http://59.219.113.13:9200/ai-gateway/mineru3.2.3/file_parse"
WORKSPACE = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))

RESULTS = {}


def get_headers():
    return {"Authorization": f"Bearer {API_KEY}", "Content-Type": "application/json"}


def load_haystack():
    with open(os.path.join(WORKSPACE, "tests/data/chat/needle_haystack_64k.json"), encoding="utf-8") as f:
        spec = json.load(f)
    text = spec["haystack_text"]
    secret = spec["secret_key"]
    # Strip the embedded needle sentence so filler contexts contain no accidental keys.
    start = text.find("【绝密归档标识】")
    end = text.find("。", text.find(secret)) + 1
    clean = text[:start] + text[end:] if start != -1 and end > start else text
    needle = f"【绝密归档标识】：庆阳智算中心应急特别调度口令为：{secret}。"
    return clean, needle, secret, spec["query"]


def build_context(clean, needle, chars):
    half = chars // 2
    filler = (clean * (chars // max(len(clean), 1) + 1))[:chars - len(needle)]
    half = min(half, len(filler))
    return filler[:half] + "\n\n" + needle + "\n\n" + filler[half:]


def probe_chat_context(model, clean, needle, secret, query, char_lengths, max_tokens=150, timeout=180):
    """Ladder of context lengths; stops escalating after first failure, then bisects once."""
    out = {}
    last_ok = None
    for chars in char_lengths:
        body = build_context(clean, needle, chars)
        payload = {
            "model": model,
            "messages": [
                {"role": "system", "content": "你是一个严格依照文档事实回答问题的政务助手。"},
                {"role": "user", "content": f"请阅读以下参考文档并回答问题：\n\n{body}\n\n问题：{query}"},
            ],
            "temperature": 0.0,
            "max_tokens": max_tokens,
        }
        label = f"{chars}_chars"
        t0 = time.time()
        try:
            resp = requests.post(f"{CHAT_BASE_URL}/chat/completions", headers=get_headers(),
                                 json=payload, timeout=timeout)
            lat = round(time.time() - t0, 3)
            if resp.status_code == 200:
                data = resp.json()
                msg = data["choices"][0]["message"]
                reply = (msg.get("content") or msg.get("reasoning") or "").strip()
                usage = data.get("usage", {})
                out[label] = {
                    "status": "SUCCESS", "http_code": 200, "latency_sec": lat,
                    "prompt_tokens": usage.get("prompt_tokens"),
                    "completion_tokens": usage.get("completion_tokens"),
                    "finish_reason": data["choices"][0].get("finish_reason"),
                    "needle_recalled": secret in reply, "reply": reply[:120],
                }
                last_ok = chars
                print(f"    [OK] {model} {chars} chars: {lat}s, prompt_tokens={usage.get('prompt_tokens')}, recalled={secret in reply}")
            else:
                out[label] = {"status": "FAILED", "http_code": resp.status_code,
                              "latency_sec": lat, "error": resp.text[:300]}
                print(f"    [FAIL] {model} {chars} chars: HTTP {resp.status_code}: {resp.text[:160]}")
                break
        except Exception as e:
            out[label] = {"status": "ERROR", "latency_sec": round(time.time() - t0, 3), "error": str(e)[:300]}
            print(f"    [EXC] {model} {chars} chars: {e}")
            break
    # One bisection step between the last success and the first failure.
    failed = [c for c, r in out.items() if r["status"] != "SUCCESS"]
    if last_ok is not None and failed:
        fail_chars = min(int(c.replace("_chars", "")) for c in failed)
        mid = (last_ok + fail_chars) // 2
        if abs(fail_chars - last_ok) > 2000:
            body = build_context(clean, needle, mid)
            payload = {
                "model": model,
                "messages": [
                    {"role": "system", "content": "你是一个严格依照文档事实回答问题的政务助手。"},
                    {"role": "user", "content": f"请阅读以下参考文档并回答问题：\n\n{body}\n\n问题：{query}"},
                ],
                "temperature": 0.0, "max_tokens": max_tokens,
            }
            t0 = time.time()
            try:
                resp = requests.post(f"{CHAT_BASE_URL}/chat/completions", headers=get_headers(),
                                     json=payload, timeout=timeout)
                lat = round(time.time() - t0, 3)
                if resp.status_code == 200:
                    data = resp.json()
                    msg = data["choices"][0]["message"]
                    reply = (msg.get("content") or msg.get("reasoning") or "").strip()
                    usage = data.get("usage", {})
                    out[f"{mid}_chars_bisect"] = {
                        "status": "SUCCESS", "http_code": 200, "latency_sec": lat,
                        "prompt_tokens": usage.get("prompt_tokens"),
                        "needle_recalled": secret in reply, "reply": reply[:120],
                    }
                    print(f"    [OK] {model} bisect {mid} chars: recalled={secret in reply}")
                else:
                    out[f"{mid}_chars_bisect"] = {"status": "FAILED", "http_code": resp.status_code,
                                                  "latency_sec": lat, "error": resp.text[:300]}
                    print(f"    [FAIL] {model} bisect {mid} chars: HTTP {resp.status_code}")
            except Exception as e:
                out[f"{mid}_chars_bisect"] = {"status": "ERROR", "error": str(e)[:300]}
                print(f"    [EXC] {model} bisect {mid} chars: {e}")
    return out


def probe_max_tokens(models):
    out = {}
    for model in models:
        res = {}
        for mt in [1, 1024, 8192, 16384, 32768, 65536]:
            payload = {
                "model": model,
                "messages": [{"role": "user", "content": "只回复两个字：收到"}],
                "temperature": 0.0, "max_tokens": mt,
            }
            t0 = time.time()
            try:
                resp = requests.post(f"{CHAT_BASE_URL}/chat/completions", headers=get_headers(),
                                     json=payload, timeout=60)
                lat = round(time.time() - t0, 3)
                if resp.status_code == 200:
                    data = resp.json()
                    res[str(mt)] = {
                        "accepted": True, "latency_sec": lat,
                        "completion_tokens": data.get("usage", {}).get("completion_tokens"),
                        "finish_reason": data["choices"][0].get("finish_reason"),
                    }
                    print(f"    [OK] {model} max_tokens={mt}: accepted, finish={data['choices'][0].get('finish_reason')}")
                else:
                    res[str(mt)] = {"accepted": False, "http_code": resp.status_code, "error": resp.text[:250]}
                    print(f"    [FAIL] {model} max_tokens={mt}: HTTP {resp.status_code}: {resp.text[:140]}")
            except Exception as e:
                res[str(mt)] = {"accepted": False, "error": str(e)[:250]}
                print(f"    [EXC] {model} max_tokens={mt}: {e}")
        out[model] = res
    return out


def probe_temperature(models):
    out = {}
    for model in models:
        res = {}
        for temp in [-0.5, 0.0, 0.7, 1.0, 1.5, 2.0, 2.1]:
            payload = {
                "model": model,
                "messages": [{"role": "user", "content": "只回复两个字：收到"}],
                "temperature": temp, "max_tokens": 16,
            }
            try:
                resp = requests.post(f"{CHAT_BASE_URL}/chat/completions", headers=get_headers(),
                                     json=payload, timeout=60)
                if resp.status_code == 200:
                    res[str(temp)] = {"accepted": True}
                    print(f"    [OK] {model} temperature={temp}: accepted")
                else:
                    res[str(temp)] = {"accepted": False, "http_code": resp.status_code, "error": resp.text[:200]}
                    print(f"    [FAIL] {model} temperature={temp}: HTTP {resp.status_code}")
            except Exception as e:
                res[str(temp)] = {"accepted": False, "error": str(e)[:200]}
                print(f"    [EXC] {model} temperature={temp}: {e}")
        out[model] = res
    return out


def probe_multi_image_extreme(counts=(20, 24)):
    multi_dir = os.path.join(WORKSPACE, "tests/data/vlm/multi_image_batch")
    imgs = sorted(os.path.join(multi_dir, f) for f in os.listdir(multi_dir)
                  if f.endswith((".png", ".jpg")))
    out = {}
    for count in counts:
        content = [{"type": "text",
                    "text": "请统计本请求包含多少张图片，只回答数字。"}]
        for i in range(count):
            p = imgs[i % len(imgs)]
            with open(p, "rb") as f:
                b64 = base64.b64encode(f.read()).decode()
            content.append({"type": "image_url",
                            "image_url": {"url": f"data:image/png;base64,{b64}"}})
        payload = {
            "model": "qwen3-VL-30B-A3B",
            "messages": [{"role": "user", "content": content}],
            "temperature": 0.1, "max_tokens": 300,
        }
        t0 = time.time()
        try:
            resp = requests.post(f"{CHAT_BASE_URL}/chat/completions", headers=get_headers(),
                                 json=payload, timeout=180)
            lat = round(time.time() - t0, 3)
            if resp.status_code == 200:
                data = resp.json()
                msg = data["choices"][0]["message"]
                reply = (msg.get("content") or msg.get("reasoning") or "").strip()
                usage = data.get("usage", {})
                out[f"{count}_images"] = {
                    "status": "SUCCESS", "http_code": 200, "latency_sec": lat,
                    "prompt_tokens": usage.get("prompt_tokens"),
                    "completion_tokens": usage.get("completion_tokens"),
                    "reply": reply[:150].replace("\n", " "),
                }
                print(f"    [OK] {count} images: {lat}s, prompt_tokens={usage.get('prompt_tokens')}, reply={reply[:60]}")
            else:
                out[f"{count}_images"] = {"status": "FAILED", "http_code": resp.status_code,
                                          "latency_sec": lat, "error": resp.text[:250]}
                print(f"    [FAIL] {count} images: HTTP {resp.status_code}: {resp.text[:140]}")
        except Exception as e:
            out[f"{count}_images"] = {"status": "ERROR", "error": str(e)[:250]}
            print(f"    [EXC] {count} images: {e}")
    return out


def probe_embedding_limits(models):
    out = {}
    with open(os.path.join(WORKSPACE, "tests/data/chat/needle_haystack_64k.json"), encoding="utf-8") as f:
        long_text = json.load(f)["haystack_text"]
    for model in models:
        res = {"input_length": {}, "batch_size": {}}
        for n_chars in [1000, 5000, 10000, 20000]:
            try:
                resp = requests.post(f"{CHAT_BASE_URL}/embeddings", headers=get_headers(),
                                     json={"model": model, "input": long_text[:n_chars]}, timeout=60)
                if resp.status_code == 200:
                    dim = len(resp.json()["data"][0]["embedding"])
                    res["input_length"][str(n_chars)] = {"accepted": True, "dimension": dim}
                    print(f"    [OK] {model} input {n_chars} chars: dim={dim}")
                else:
                    res["input_length"][str(n_chars)] = {"accepted": False,
                                                         "http_code": resp.status_code,
                                                         "error": resp.text[:200]}
                    print(f"    [FAIL] {model} input {n_chars} chars: HTTP {resp.status_code}")
            except Exception as e:
                res["input_length"][str(n_chars)] = {"accepted": False, "error": str(e)[:200]}
                print(f"    [EXC] {model} input {n_chars} chars: {e}")
        for batch in [16, 32, 64, 128, 256]:
            inputs = [f"国土空间规划监测样本第{i}号。" for i in range(batch)]
            try:
                resp = requests.post(f"{CHAT_BASE_URL}/embeddings", headers=get_headers(),
                                     json={"model": model, "input": inputs}, timeout=60)
                if resp.status_code == 200:
                    n = len(resp.json()["data"])
                    res["batch_size"][str(batch)] = {"accepted": True, "returned": n}
                    print(f"    [OK] {model} batch {batch}: returned {n}")
                else:
                    res["batch_size"][str(batch)] = {"accepted": False,
                                                     "http_code": resp.status_code,
                                                     "error": resp.text[:200]}
                    print(f"    [FAIL] {model} batch {batch}: HTTP {resp.status_code}")
            except Exception as e:
                res["batch_size"][str(batch)] = {"accepted": False, "error": str(e)[:200]}
                print(f"    [EXC] {model} batch {batch}: {e}")
        out[model] = res
    return out


def probe_rerank_scale(models):
    out = {}
    with open(os.path.join(WORKSPACE, "tests/data/chat/needle_haystack_64k.json"), encoding="utf-8") as f:
        base = json.load(f)["haystack_text"]
    pool = [base[i:i + 200] for i in range(0, 200 * 260, 200)][:260]
    pool[7] = "全国国土空间规划实施监测网络建设方案专项条款：统一时空底座与遥感监测核查机制。"
    for model in models:
        res = {}
        for n in [10, 50, 100, 200]:
            t0 = time.time()
            try:
                resp = requests.post(f"{CHAT_BASE_URL}/rerank", headers=get_headers(),
                                     json={"model": model, "query": "国土空间规划实施监测网络",
                                           "documents": pool[:n], "top_n": min(5, n)},
                                     timeout=90)
                lat = round(time.time() - t0, 3)
                if resp.status_code == 200:
                    data = resp.json()
                    top = data["results"][0]
                    res[str(n)] = {"accepted": True, "latency_sec": lat,
                                   "returned": len(data["results"]),
                                   "top1_index": top.get("index"),
                                   "top1_score": round(top.get("relevance_score", 0), 4)}
                    print(f"    [OK] {model} n={n}: {lat}s, top1_idx={top.get('index')}, score={top.get('relevance_score'):.4f}")
                else:
                    res[str(n)] = {"accepted": False, "http_code": resp.status_code,
                                   "latency_sec": lat, "error": resp.text[:200]}
                    print(f"    [FAIL] {model} n={n}: HTTP {resp.status_code}: {resp.text[:120]}")
            except Exception as e:
                res[str(n)] = {"accepted": False, "latency_sec": round(time.time() - t0, 3),
                               "error": str(e)[:200]}
                print(f"    [EXC] {model} n={n}: {e}")
        out[model] = res
    return out


def probe_mineru_pages(page_counts=(5, 15, 30)):
    src = os.path.join(WORKSPACE, "tests/data/mineru/text_pdf")
    pdfs = [f for f in os.listdir(src) if f.endswith(".pdf")]
    if not pdfs:
        return {"status": "SKIPPED", "reason": "no source pdf"}
    reader = PdfReader(os.path.join(src, pdfs[0]))
    n_src = len(reader.pages)
    out = {"source_pdf": pdfs[0], "source_pages": n_src, "ladder": {}}
    for pages in page_counts:
        writer = PdfWriter()
        for i in range(pages):
            writer.add_page(reader.pages[i % n_src])
        buf = io.BytesIO()
        writer.write(buf)
        buf.seek(0)
        size_mb = round(buf.getbuffer().nbytes / 1024 / 1024, 2)
        t0 = time.time()
        try:
            resp = requests.post(MINERU_URL, headers={"Authorization": f"Bearer {API_KEY}"},
                                 files={"files": (f"ladder_{pages}p.pdf", buf, "application/pdf")},
                                 data={"output_dir": "output"}, timeout=300)
            lat = round(time.time() - t0, 3)
            if resp.status_code == 200:
                data = resp.json()
                status = data.get("status")
                results = data.get("results", {})
                md_len = 0
                for v in results.values():
                    if isinstance(v, dict):
                        md_len = max(md_len, len(v.get("md_content") or ""))
                out["ladder"][str(pages)] = {"accepted": True, "http_code": 200,
                                             "task_status": status, "size_mb": size_mb,
                                             "latency_sec": lat, "md_content_chars": md_len}
                print(f"    [OK] MinerU {pages} pages ({size_mb}MB): {lat}s, status={status}, md_chars={md_len}")
            else:
                out["ladder"][str(pages)] = {"accepted": False, "http_code": resp.status_code,
                                             "size_mb": size_mb, "latency_sec": lat,
                                             "error": resp.text[:250]}
                print(f"    [FAIL] MinerU {pages} pages: HTTP {resp.status_code}: {resp.text[:160]}")
        except Exception as e:
            out["ladder"][str(pages)] = {"accepted": False, "size_mb": size_mb,
                                         "latency_sec": round(time.time() - t0, 3),
                                         "error": str(e)[:250]}
            print(f"    [EXC] MinerU {pages} pages: {e}")
    return out


if __name__ == "__main__":
    print("=" * 60)
    print("  Qingyang AI Computing Center — Model Limit Probes")
    print("=" * 60)

    print("\n=== A. Context ceiling ladder (needle-in-haystack) ===")
    clean, needle, secret, query = load_haystack()
    RESULTS["context_ceiling"] = {
        "dsv4": probe_chat_context("dsv4", clean, needle, secret, query,
                                   [157460, 196000, 230000, 262000]),
        "qwen3.6-27b": probe_chat_context("qwen3.6-27b", clean, needle, secret, query,
                                          [8000, 32000, 64000, 131000], max_tokens=2048, timeout=240),
        "qwen3-VL-30B-A3B_text": probe_chat_context("qwen3-VL-30B-A3B", clean, needle, secret, query,
                                                    [32000, 64000, 131000], timeout=240),
    }

    print("\n=== B. max_tokens boundary ===")
    RESULTS["max_tokens_boundary"] = probe_max_tokens(
        ["dsv4", "qwen3.6-27b", "qwen3-VL-30B-A3B"])

    print("\n=== B2. temperature boundary ===")
    RESULTS["temperature_boundary"] = probe_temperature(["dsv4", "qwen3.6-27b"])

    print("\n=== C. Multi-image beyond 16 ===")
    RESULTS["multi_image_extreme"] = probe_multi_image_extreme((20, 24))

    print("\n=== D. Embedding input-length & batch limits ===")
    RESULTS["embedding_limits"] = probe_embedding_limits(
        ["bge-m3", "qwen3-embadding-8b", "bge-large-zh-v1.5"])

    print("\n=== E. Rerank candidate-set scale ===")
    RESULTS["rerank_scale"] = probe_rerank_scale(
        ["bge-reranker-v2-m3", "qwen3-reranker-8b", "bge-reranker-large"])

    print("\n=== F. MinerU synchronized page ladder ===")
    RESULTS["mineru_page_ladder"] = probe_mineru_pages((5, 15, 30))

    out_p = os.path.join(WORKSPACE, "tests/scripts/limit_summary.json")
    with open(out_p, "w", encoding="utf-8") as f:
        json.dump(RESULTS, f, ensure_ascii=False, indent=2)
    print(f"\n[DONE] Limit probe summary saved to {out_p}")
