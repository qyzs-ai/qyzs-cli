#!/usr/bin/env python3
"""核验线上 Release 资产 (发布后自检)

用法:
    python3 tests/scripts/verify_published_release.py [--tag v0.2.0] [--repo qyzs-ai/qyzs-cli]

检查项:
  1. Release 存在, 且资产集合完整 (六个平台二进制 + 三个安装脚本 + SHA256SUMS
     + tests_data.zip)
  2. SHA256SUMS 与每个资产的实际内容逐一相符 (重新下载重算)
  3. Release 里的 install.sh / install.ps1 已钉定到该 tag (带 v 前缀)
  4. 线上二进制里的 help 已包含参数边界 (与仓库 args.rs 的契约一致) ——
     防止「仓库已修、发布产物还是旧的」
  5. 线上资产对应的源文件与仓库当前内容一致 (忽略版本钉定行)
"""

from __future__ import annotations

import argparse
import hashlib
import io
import pathlib
import re
import sys
import tarfile
import time
import urllib.request
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[2]

EXPECTED_ASSETS = {
    "install.sh",
    "install.ps1",
    "install.cmd",
    "SHA256SUMS",
    "tests_data.zip",
    "qyzs-x86_64-unknown-linux-musl.tar.gz",
    "qyzs-aarch64-unknown-linux-musl.tar.gz",
    "qyzs-loongarch64-unknown-linux-gnu.tar.gz",
    "qyzs-x86_64-apple-darwin.tar.gz",
    "qyzs-aarch64-apple-darwin.tar.gz",
    "qyzs-x86_64-pc-windows-msvc.zip",
}

results: list[tuple[bool, str, str]] = []


def check(ok: bool, name: str, detail: str = "") -> bool:
    results.append((ok, name, detail))
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}" + (f" — {detail}" if detail else ""))
    return ok


def get(url: str, retries: int = 4) -> bytes:
    """带重试的 GET —— 重打 tag 时 Release 会被重建, API 可能瞬时不可用"""
    last: Exception | None = None
    for attempt in range(retries):
        try:
            req = urllib.request.Request(
                url, headers={"User-Agent": "qyzs-release-verify"}
            )
            with urllib.request.urlopen(req, timeout=180) as r:
                return r.read()
        except Exception as e:  # noqa: BLE001
            last = e
            time.sleep(2 * (attempt + 1))
    raise last if last else RuntimeError("unknown error")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--tag", default="v0.2.0")
    ap.add_argument("--repo", default="qyzs-ai/qyzs-cli")
    args = ap.parse_args()
    base = f"https://github.com/{args.repo}/releases/download/{args.tag}"
    tag = args.tag

    print(f"== 核验 {args.repo} 的 {tag} ==")

    # 1) 资产集合
    try:
        import json

        api = json.loads(
            get(f"https://api.github.com/repos/{args.repo}/releases/tags/{tag}").decode()
        )
    except Exception as e:
        print(f"  [FAIL] 取 Release 元信息失败: {e}")
        return 1
    names = {a["name"] for a in api.get("assets", [])}
    missing = EXPECTED_ASSETS - names
    extra = names - EXPECTED_ASSETS
    check(not missing, "资产集合完整", f"缺 {sorted(missing)}" if missing else "")
    if extra:
        print(f"    [WARN] 额外资产: {sorted(extra)}")

    # 2) SHA256SUMS 逐一核对
    try:
        sums_txt = get(f"{base}/SHA256SUMS").decode()
    except Exception as e:
        check(False, "取 SHA256SUMS", str(e))
        return 1
    sums = {}
    for line in sums_txt.splitlines():
        parts = line.split()
        if len(parts) == 2 and re.fullmatch(r"[0-9a-f]{64}", parts[0]):
            sums[parts[1]] = parts[0]
    bad = []
    for name, expect in sorted(sums.items()):
        try:
            actual = hashlib.sha256(get(f"{base}/{name}")).hexdigest()
        except Exception as e:
            bad.append(f"{name}: 下载失败 {e}")
            continue
        if actual != expect:
            bad.append(f"{name}: 期望 {expect[:16]} 实际 {actual[:16]}")
    check(not bad, f"SHA256SUMS 与 {len(sums)} 个资产逐一相符", "; ".join(bad[:3]))

    # 3) 安装脚本已钉定到该 tag
    try:
        sh = get(f"{base}/install.sh").decode()
        ps = get(f"{base}/install.ps1").decode()
    except Exception as e:
        check(False, "取安装脚本", str(e))
        return 1
    m = re.search(r'^PINNED_VERSION="([^"]+)"', sh, re.M)
    check(
        bool(m) and m.group(1) == tag,
        "install.sh 已钉定到该 tag (带 v 前缀)",
        f"实际 {m.group(1) if m else '未找到 PINNED_VERSION'}",
    )
    m2 = re.search(r'^\$PinnedVersion = "([^"]+)"', ps, re.M)
    check(
        bool(m2) and m2.group(1) == tag,
        "install.ps1 已钉定到该 tag",
        f"实际 {m2.group(1) if m2 else '未找到'}",
    )

    # 4) 线上安装脚本与仓库当前实现一致 (忽略钉定行)
    for fname in ("install.sh", "install.ps1"):
        live = sh if fname.endswith(".sh") else ps
        live_n = re.sub(
            r'^(PINNED_VERSION\s*=\s*|\$PinnedVersion\s*=\s*).*$', r"\1X", live, flags=re.M
        )
        repo_n = re.sub(
            r'^(PINNED_VERSION\s*=\s*|\$PinnedVersion\s*=\s*).*$',
            r"\1X",
            (ROOT / fname).read_text(encoding="utf-8"),
            flags=re.M,
        )
        check(
            live_n.strip() == repo_n.strip(),
            f"{fname} 线上内容与仓库一致",
            "" if live_n.strip() == repo_n.strip() else "线上是旧版本, 需重打 tag",
        )

    # 5) 线上二进制里的 help 已含参数边界
    #    取 x86_64-linux 的 musl 静态二进制, 跑 --help 验证
    try:
        blob = get(f"{base}/qyzs-x86_64-unknown-linux-musl.tar.gz")
        with tarfile.open(fileobj=io.BytesIO(blob), mode="r:gz") as tf:
            member = next(m for m in tf.getmembers() if m.name.endswith("qyzs"))
            bin_bytes = tf.extractfile(member).read()
        tmp = ROOT / "tmp" / "release-verify-bin"
        tmp.parent.mkdir(parents=True, exist_ok=True)
        tmp.write_bytes(bin_bytes)
        tmp.chmod(0o755)
        import subprocess

        # 每个片段归所属子命令 —— 拿去别的子命令的 help 里找必然落空
        def help_of(sub: str) -> str:
            r = subprocess.run(
                [str(tmp), sub, "--help"], capture_output=True, text=True, timeout=60
            )
            return r.stdout + r.stderr

        helps = {sub: help_of(sub) for sub in ("markscan", "dataset", "upgrade")}
        # 片段 -> (子命令, 说明)。取自仓库 args.rs: 仓库里有、线上没有 = 产物落后
        args_rs = (ROOT / "src/cli/args.rs").read_text(encoding="utf-8")
        fragments = [
            ("1~8192", "markscan", "--max-dim 边界"),
            ("默认 tests/data", "dataset", "--dest 默认值"),
            ("默认 qyzs-ai/qyzs-cli", "upgrade", "--repo 默认值"),
        ]
        for frag, sub, label in fragments:
            if frag not in args_rs:
                continue  # 仓库里已不存在该文案, 不必核
            check(frag in helps[sub], f"线上二进制的 {sub} --help 含 {label}")
        tmp.unlink(missing_ok=True)
    except Exception as e:
        check(False, "核验线上二进制的 help", str(e))

    failed = [r for r in results if not r[0]]
    print(f"\n  → {'全部通过' if not failed else f'{len(failed)} 项失败'}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
