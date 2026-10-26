#!/usr/bin/env python3
"""OCR/VLM 模型能力探针。

默认仅打印计划；显式 --live 后才访问网关。测试产物包含生成的公开基准输入、脱敏
请求元数据、模型原始 JSON 响应、尺寸边界、延时和关键词召回，不保存 API Key。

这不是普通 CI：尺寸极限和多模型调用有成本，应在专用测试环境手动运行。
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import io
import json
import os
import sys
import time
import tomllib
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import requests
from PIL import Image

SCRIPT_DIR = Path(__file__).resolve().parent
REPO = SCRIPT_DIR.parent.parent
sys.path.insert(0, str(SCRIPT_DIR))
from _qyzs_env import get_api_key  # noqa: E402

DEFAULT_LONG_EDGES = [256, 512, 800, 1024, 1536, 2048, 3072, 4096]
DEFAULT_PROMPT = (
    "逐字转录图片中的全部文字，保留标题、段落、表格、文号、日期和印章内容。"
    "只输出转录结果，不要解释。"
)


@dataclass
class ProbeInput:
    source: str
    generated_file: str
    mime: str
    width: int
    height: int
    long_edge: int
    bytes: int
    sha256: str


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="实测 OCR/VLM 图像输入与输出能力")
    parser.add_argument("--live", action="store_true", help="实际访问网关；默认只打印计划")
    parser.add_argument(
        "--model",
        action="append",
        dest="models",
        help="仅测试指定模型，可重复；默认读取 capability_matrix.toml 全部 OCR 模型",
    )
    parser.add_argument(
        "--long-edges",
        default=",".join(map(str, DEFAULT_LONG_EDGES)),
        help="尺寸阶梯，逗号分隔",
    )
    parser.add_argument("--max-requests", type=int, default=40, help="请求总数硬上限")
    parser.add_argument("--timeout", type=int, default=120, help="单请求超时秒数")
    parser.add_argument(
        "--base-url",
        default=os.getenv("QYZS_BASE_URL", "http://59.219.113.13:9200"),
        help="网关根地址",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=REPO / "tests" / "artifacts" / "ocr-capabilities",
    )
    return parser.parse_args()


def load_matrix_models() -> list[str]:
    with (REPO / "capability_matrix.toml").open("rb") as handle:
        matrix = tomllib.load(handle)
    return [item["id"] for item in matrix.get("ocr_capabilities", {}).get("models", [])]


def load_cases() -> list[dict[str, Any]]:
    spec_path = REPO / "tests" / "data" / "ocr" / "paired_alignment_spec.json"
    with spec_path.open(encoding="utf-8") as handle:
        spec = json.load(handle)
    cases = []
    for pair in spec["pairs"]:
        cases.append(
            {
                "id": pair["pair_id"],
                "path": REPO / "tests" / "data" / pair["ocr_image_file"],
                "expected_key_tokens": pair["expected_key_tokens"],
            }
        )
    return cases


def resize_png(source: Path, long_edge: int) -> tuple[bytes, int, int]:
    with Image.open(source) as original:
        image = original.convert("RGB")
        width, height = image.size
        scale = long_edge / max(width, height)
        target = (max(1, round(width * scale)), max(1, round(height * scale)))
        image = image.resize(target, Image.Resampling.LANCZOS)
        buffer = io.BytesIO()
        image.save(buffer, format="PNG")
        return buffer.getvalue(), target[0], target[1]


def response_text(data: dict[str, Any]) -> str:
    try:
        message = data["choices"][0]["message"]
    except (KeyError, IndexError, TypeError):
        return ""
    return (message.get("content") or message.get("reasoning") or "").strip()


def safe_error(text: str) -> str:
    # 服务端错误可能回显请求片段；限制长度，且不记录 Authorization。
    return text[:2000]


def run_probe(
    *,
    session: requests.Session,
    endpoint: str,
    model: str,
    case: dict[str, Any],
    long_edge: int,
    artifact_dir: Path,
    timeout: int,
) -> dict[str, Any]:
    image_bytes, width, height = resize_png(case["path"], long_edge)
    digest = hashlib.sha256(image_bytes).hexdigest()
    image_dir = artifact_dir / "inputs"
    image_dir.mkdir(parents=True, exist_ok=True)
    generated = image_dir / f"{case['id']}-{long_edge}.png"
    generated.write_bytes(image_bytes)

    probe_input = ProbeInput(
        source=str(case["path"].relative_to(REPO)),
        generated_file=str(generated.relative_to(artifact_dir)),
        mime="image/png",
        width=width,
        height=height,
        long_edge=max(width, height),
        bytes=len(image_bytes),
        sha256=digest,
    )
    data_url = "data:image/png;base64," + base64.b64encode(image_bytes).decode("ascii")
    payload = {
        "model": model,
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": DEFAULT_PROMPT},
                    {"type": "image_url", "image_url": {"url": data_url}},
                ],
            }
        ],
        "temperature": 0.0,
        "max_tokens": 4096,
    }
    request_meta = {
        "model": model,
        "endpoint": endpoint,
        "prompt": DEFAULT_PROMPT,
        "temperature": 0.0,
        "max_tokens": 4096,
        "input": asdict(probe_input),
    }
    started = time.monotonic()
    try:
        response = session.post(endpoint, json=payload, timeout=timeout)
        latency = round(time.monotonic() - started, 3)
        try:
            body: Any = response.json()
        except ValueError:
            body = {"raw_text": safe_error(response.text)}
        text = response_text(body) if isinstance(body, dict) else ""
        expected = case["expected_key_tokens"]
        matched = [token for token in expected if token in text]
        return {
            "request": request_meta,
            "http_status": response.status_code,
            "accepted": response.ok,
            "latency_sec": latency,
            "usage": body.get("usage") if isinstance(body, dict) else None,
            "finish_reason": (
                body.get("choices", [{}])[0].get("finish_reason")
                if isinstance(body, dict) and body.get("choices")
                else None
            ),
            "output_text": text,
            "expected_key_tokens": expected,
            "matched_key_tokens": matched,
            "key_token_recall": len(matched) / len(expected) if expected else None,
            "raw_response": body,
        }
    except requests.RequestException as exc:
        return {
            "request": request_meta,
            "accepted": False,
            "latency_sec": round(time.monotonic() - started, 3),
            "error": str(exc),
        }


def main() -> int:
    args = parse_args()
    if args.max_requests < 1:
        raise SystemExit("--max-requests 必须 >= 1")
    long_edges = sorted({int(value) for value in args.long_edges.split(",") if value.strip()})
    if not long_edges or min(long_edges) < 1:
        raise SystemExit("--long-edges 必须包含正整数")
    models = args.models or load_matrix_models()
    cases = load_cases()
    planned = len(models) * len(cases) * len(long_edges)
    actual = min(planned, args.max_requests)
    print(
        json.dumps(
            {
                "live": args.live,
                "models": models,
                "cases": [case["id"] for case in cases],
                "long_edges": long_edges,
                "planned_requests": planned,
                "capped_requests": actual,
            },
            ensure_ascii=False,
            indent=2,
        )
    )
    if not args.live:
        print("[DRY-RUN] 未访问网关；加 --live 才执行实测。")
        return 0

    api_key = get_api_key()
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    artifact_dir = args.output_dir / timestamp
    artifact_dir.mkdir(parents=True, exist_ok=False)
    endpoint = args.base_url.rstrip("/") + "/api/aiCenterChat/v1/chat/completions"
    session = requests.Session()
    session.headers.update(
        {"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"}
    )

    records = []
    for model in models:
        for case in cases:
            for long_edge in long_edges:
                if len(records) >= args.max_requests:
                    break
                print(f"[PROBE] {model} {case['id']} {long_edge}px")
                records.append(
                    run_probe(
                        session=session,
                        endpoint=endpoint,
                        model=model,
                        case=case,
                        long_edge=long_edge,
                        artifact_dir=artifact_dir,
                        timeout=args.timeout,
                    )
                )
            if len(records) >= args.max_requests:
                break
        if len(records) >= args.max_requests:
            break

    result = {
        "schema_version": 1,
        "tested_at": datetime.now(timezone.utc).isoformat(),
        "endpoint": endpoint,
        "models": models,
        "records": records,
    }
    (artifact_dir / "results.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8"
    )
    print(f"[DONE] {artifact_dir / 'results.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
