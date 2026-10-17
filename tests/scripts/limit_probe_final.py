#!/usr/bin/env python3
"""Final round: bracket the remaining context ceilings, then stop."""
import os, sys, time, json, requests

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


def context_probe(model, chars, max_tokens=150, timeout=420):
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


print("=== dsv4 final bracket: 700k chars (~347k tokens) ===")
r1 = context_probe("dsv4", 700000)
OUT["dsv4_700000"] = r1
if r1.get("accepted"):
    print("=== dsv4 final bracket: 1,050,000 chars (~521k tokens) ===")
    OUT["dsv4_1050000"] = context_probe("dsv4", 1050000)

print("=== qwen3.6-27b final bracket: 150k chars (~73.5k tokens) ===")
OUT["qwen36_150000"] = context_probe("qwen3.6-27b", 150000, max_tokens=2048)

print("=== qwen3-VL text final bracket: 400k chars (~205k tokens) ===")
OUT["vl_400000"] = context_probe("qwen3-VL-30B-A3B", 400000)

out_p = os.path.join(WORKSPACE, "tests/scripts/limit_summary_final.json")
with open(out_p, "w", encoding="utf-8") as f:
    json.dump(OUT, f, ensure_ascii=False, indent=2)
print(f"[DONE] final-round saved to {out_p}")
