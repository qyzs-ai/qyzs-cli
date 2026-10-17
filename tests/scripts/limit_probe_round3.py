#!/usr/bin/env python3
"""Round-3 probes: push past the remaining unbounded ceilings."""
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


def context_probe(model, chars, max_tokens=150, timeout=300):
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
        print(f"  [FAIL] {model} {chars} chars: HTTP {r.status_code}: {r.text[:220]}")
        return {"accepted": False, "http_code": r.status_code, "latency_sec": lat, "error": r.text[:300]}
    except Exception as e:
        print(f"  [EXC] {model} {chars} chars: {e}")
        return {"accepted": False, "error": str(e)[:300]}


print("=== dsv4 ceiling push: 400k / 530k chars (~197k / ~262k tokens) ===")
OUT["dsv4_400000"] = context_probe("dsv4", 400000)
OUT["dsv4_530000"] = context_probe("dsv4", 530000)

print("=== qwen3.6-27b push past 65,536 tokens: 140k chars ===")
OUT["qwen36_140000"] = context_probe("qwen3.6-27b", 140000, max_tokens=2048)

print("=== qwen3-VL text push: 200k chars (~98k tokens) ===")
OUT["vl_200000"] = context_probe("qwen3-VL-30B-A3B", 200000)

print("=== qwen3.6-27b temperature 8.0 ===")
try:
    r = requests.post(f"{CHAT}/chat/completions", headers=H, json={
        "model": "qwen3.6-27b", "messages": [{"role": "user", "content": "只回复两个字：收到"}],
        "temperature": 8.0, "max_tokens": 16}, timeout=60)
    OUT["qwen36_temp_8.0"] = {"accepted": r.status_code == 200}
    print(f"  [TEMP] qwen3.6-27b 8.0: {'OK' if r.status_code == 200 else 'HTTP ' + str(r.status_code)}")
except Exception as e:
    OUT["qwen36_temp_8.0"] = {"accepted": False, "error": str(e)[:150]}
    print(f"  [TEMP] EXC {e}")

print("=== qwen3-embadding-8b length bisect: 11k / 12k chars ===")
OUT["qwen3_emb_length_bisect"] = {}
for n in [11000, 12000]:
    try:
        r = requests.post(f"{CHAT}/embeddings", headers=H,
                          json={"model": "qwen3-embadding-8b", "input": text[:n]}, timeout=30)
        ok = r.status_code == 200
        OUT["qwen3_emb_length_bisect"][str(n)] = {"accepted": ok} if ok else {
            "accepted": False, "http_code": r.status_code, "error": r.text[:150]}
        print(f"  [EMB] qwen3-embadding-8b {n} chars: {'OK' if ok else 'HTTP ' + str(r.status_code)}")
    except Exception as e:
        OUT["qwen3_emb_length_bisect"][str(n)] = {"accepted": False, "error": str(e)[:150]}
        print(f"  [EMB] EXC {e}")

print("=== 48-image gateway body limit ===")
multi_dir = os.path.join(WORKSPACE, "tests/data/vlm/multi_image_batch")
imgs = sorted(os.path.join(multi_dir, f) for f in os.listdir(multi_dir) if f.endswith((".png", ".jpg")))
content = [{"type": "text", "text": "请统计本请求包含多少张图片，只回答数字。"}]
total_bytes = 0
for i in range(48):
    p = imgs[i % len(imgs)]
    total_bytes += os.path.getsize(p)
    with open(p, "rb") as f:
        b64 = base64.b64encode(f.read()).decode()
    content.append({"type": "image_url", "image_url": {"url": f"data:image/png;base64,{b64}"}})
t0 = time.time()
try:
    r = requests.post(f"{CHAT}/chat/completions", headers=H, json={
        "model": "qwen3-VL-30B-A3B",
        "messages": [{"role": "user", "content": content}],
        "temperature": 0.1, "max_tokens": 300}, timeout=300)
    lat = round(time.time() - t0, 3)
    if r.status_code == 200:
        d = r.json()
        reply = (d["choices"][0]["message"].get("content") or "").strip()
        OUT["images_48"] = {"accepted": True, "latency_sec": lat, "raw_bytes_mb": round(total_bytes / 1048576, 1),
                            "prompt_tokens": d.get("usage", {}).get("prompt_tokens"), "reply": reply[:80]}
        print(f"  [OK] 48 images ({total_bytes/1048576:.1f}MB raw): {lat}s prompt_tokens={d.get('usage', {}).get('prompt_tokens')} reply={reply[:50]}")
    else:
        OUT["images_48"] = {"accepted": False, "http_code": r.status_code,
                            "raw_bytes_mb": round(total_bytes / 1048576, 1),
                            "latency_sec": lat, "error": r.text[:200]}
        print(f"  [FAIL] 48 images: HTTP {r.status_code}: {r.text[:140]}")
except Exception as e:
    OUT["images_48"] = {"accepted": False, "error": str(e)[:200]}
    print(f"  [EXC] 48 images: {e}")

out_p = os.path.join(WORKSPACE, "tests/scripts/limit_summary_round3.json")
with open(out_p, "w", encoding="utf-8") as f:
    json.dump(OUT, f, ensure_ascii=False, indent=2)
print(f"[DONE] round-3 saved to {out_p}")
