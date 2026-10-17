#!/usr/bin/env python3
"""Follow-up probes to pin down exact ceilings found by limit_probe.py."""
import os, sys, time, json, base64, requests

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _qyzs_env import get_api_key

API_KEY = get_api_key()
CHAT = "http://59.219.113.13:9200/api/aiCenterChat/v1"
WORKSPACE = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
H = {"Authorization": f"Bearer {API_KEY}", "Content-Type": "application/json"}
OUT = {}

with open(os.path.join(WORKSPACE, "tests/data/chat/needle_haystack_64k.json"), encoding="utf-8") as f:
    spec = json.load(f)
text = spec["haystack_text"]
secret = spec["secret_key"]
start = text.find("【绝密归档标识】")
end = text.find("。", text.find(secret)) + 1
clean = text[:start] + text[end:]
needle = f"【绝密归档标识】：庆阳智算中心应急特别调度口令为：{secret}。"


def context_probe(model, chars, max_tokens=150, timeout=240):
    half = chars // 2
    filler = (clean * (chars // max(len(clean), 1) + 1))[: chars - len(needle)]
    half = min(half, len(filler))
    body = filler[:half] + "\n\n" + needle + "\n\n" + filler[half:]
    payload = {
        "model": model,
        "messages": [{"role": "user", "content": f"请阅读以下参考文档并回答问题：\n\n{body}\n\n问题：{spec['query']}"}],
        "temperature": 0.0, "max_tokens": max_tokens,
    }
    t0 = time.time()
    try:
        r = requests.post(f"{CHAT}/chat/completions", headers=H, json=payload, timeout=timeout)
        lat = round(time.time() - t0, 3)
        if r.status_code == 200:
            d = r.json()
            reply = (d["choices"][0]["message"].get("content") or d["choices"][0]["message"].get("reasoning") or "").strip()
            u = d.get("usage", {})
            print(f"  [OK] {model} {chars} chars: {lat}s prompt_tokens={u.get('prompt_tokens')} recalled={secret in reply}")
            return {"accepted": True, "latency_sec": lat, "prompt_tokens": u.get("prompt_tokens"),
                    "needle_recalled": secret in reply}
        print(f"  [FAIL] {model} {chars} chars: HTTP {r.status_code}: {r.text[:200]}")
        return {"accepted": False, "http_code": r.status_code, "latency_sec": lat, "error": r.text[:300]}
    except Exception as e:
        print(f"  [EXC] {model} {chars} chars: {e}")
        return {"accepted": False, "error": str(e)[:300]}


def temp_probe(model, temps):
    res = {}
    for t in temps:
        try:
            r = requests.post(f"{CHAT}/chat/completions", headers=H, json={
                "model": model, "messages": [{"role": "user", "content": "只回复两个字：收到"}],
                "temperature": t, "max_tokens": 16}, timeout=60)
            res[str(t)] = {"accepted": r.status_code == 200}
            if r.status_code != 200:
                res[str(t)]["error"] = r.text[:150]
            print(f"  [TEMP] {model} {t}: {'OK' if r.status_code == 200 else 'HTTP ' + str(r.status_code)}")
        except Exception as e:
            res[str(t)] = {"accepted": False, "error": str(e)[:150]}
            print(f"  [TEMP] {model} {t}: EXC {e}")
    return res


print("=== A+. Context ceiling confirmation ===")
OUT["context_ceiling_confirm"] = {
    # dsv4 passed 129,425 prompt tokens (< 131,072) — probe just above the presumed 128k vLLM cap
    "dsv4_267000_chars": context_probe("dsv4", 267000),
    "dsv4_330000_chars": context_probe("dsv4", 330000),
    # qwen3.6-27b passed 63,489 tokens — probe just above 65,536
    "qwen3.6-27b_134500_chars": context_probe("qwen3.6-27b", 134500, max_tokens=2048),
    # qwen3-VL passed 66,990 tokens — probe above 68k
    "qwen3-VL-30B-A3B_140000_chars": context_probe("qwen3-VL-30B-A3B", 140000),
}

print("=== B3. temperature upper bound for qwen3.6-27b ===")
OUT["temperature_upper_qwen36"] = temp_probe("qwen3.6-27b", [2.2, 2.5, 3.0, 4.0])

print("=== D2. bge-large-zh-v1.5 input length floor ===")
OUT["bge_large_zh_input_length"] = {}
for n in [50, 100, 300, 500, 800]:
    try:
        r = requests.post(f"{CHAT}/embeddings", headers=H,
                          json={"model": "bge-large-zh-v1.5", "input": "国土空间规划监测" * (n // 8)}, timeout=30)
        ok = r.status_code == 200
        OUT["bge_large_zh_input_length"][str(n)] = {"accepted": ok} if ok else {
            "accepted": False, "http_code": r.status_code, "error": r.text[:150]}
        print(f"  [EMB] bge-large-zh-v1.5 ~{n} chars: {'OK' if ok else 'HTTP ' + str(r.status_code)}")
    except Exception as e:
        OUT["bge_large_zh_input_length"][str(n)] = {"accepted": False, "error": str(e)[:150]}
        print(f"  [EMB] bge-large-zh-v1.5 ~{n} chars: EXC {e}")

print("=== D3. bge-m3 / qwen3-embadding-8b length bisect (10k OK, 20k fail) ===")
for model in ["bge-m3", "qwen3-embadding-8b"]:
    OUT[f"{model}_length_bisect"] = {}
    for n in [13000, 16000, 18000]:
        try:
            r = requests.post(f"{CHAT}/embeddings", headers=H,
                              json={"model": model, "input": text[:n]}, timeout=30)
            ok = r.status_code == 200
            OUT[f"{model}_length_bisect"][str(n)] = {"accepted": ok} if ok else {
                "accepted": False, "http_code": r.status_code, "error": r.text[:150]}
            print(f"  [EMB] {model} {n} chars: {'OK' if ok else 'HTTP ' + str(r.status_code)}")
        except Exception as e:
            OUT[f"{model}_length_bisect"][str(n)] = {"accepted": False, "error": str(e)[:150]}
            print(f"  [EMB] {model} {n} chars: EXC {e}")

print("=== C2. 32-image gateway body limit ===")
multi_dir = os.path.join(WORKSPACE, "tests/data/vlm/multi_image_batch")
imgs = sorted(os.path.join(multi_dir, f) for f in os.listdir(multi_dir) if f.endswith((".png", ".jpg")))
content = [{"type": "text", "text": "请统计本请求包含多少张图片，只回答数字。"}]
for i in range(32):
    with open(imgs[i % len(imgs)], "rb") as f:
        b64 = base64.b64encode(f.read()).decode()
    content.append({"type": "image_url", "image_url": {"url": f"data:image/png;base64,{b64}"}})
t0 = time.time()
try:
    r = requests.post(f"{CHAT}/chat/completions", headers=H, json={
        "model": "qwen3-VL-30B-A3B",
        "messages": [{"role": "user", "content": content}],
        "temperature": 0.1, "max_tokens": 300}, timeout=240)
    lat = round(time.time() - t0, 3)
    if r.status_code == 200:
        d = r.json()
        reply = (d["choices"][0]["message"].get("content") or "").strip()
        OUT["images_32"] = {"accepted": True, "latency_sec": lat,
                            "prompt_tokens": d.get("usage", {}).get("prompt_tokens"),
                            "reply": reply[:80]}
        print(f"  [OK] 32 images: {lat}s prompt_tokens={d.get('usage', {}).get('prompt_tokens')}")
    else:
        OUT["images_32"] = {"accepted": False, "http_code": r.status_code,
                            "latency_sec": lat, "error": r.text[:200]}
        print(f"  [FAIL] 32 images: HTTP {r.status_code}: {r.text[:140]}")
except Exception as e:
    OUT["images_32"] = {"accepted": False, "error": str(e)[:200]}
    print(f"  [EXC] 32 images: {e}")

out_p = os.path.join(WORKSPACE, "tests/scripts/limit_summary_followup.json")
with open(out_p, "w", encoding="utf-8") as f:
    json.dump(OUT, f, ensure_ascii=False, indent=2)
print(f"[DONE] follow-up saved to {out_p}")
