#!/usr/bin/env python3
"""发布流水线本地验证 (无需真实发布)

在没有 tag / npm token / 多台真实机器的条件下, 用本地模拟的 GitHub Release
端到端验证发布链路:

  1. 构建宿主平台的 release 二进制 (CI 同一 target)
  2. 按 CI 的资产命名组装 .tar.gz / .zip + Release API 响应
  3. 起一个本地 HTTP 服务模拟 GitHub Release
  4. install.sh 全链路: 查版本 → 下载 → 解包 → 安装 → 可执行
  5. qyzs upgrade --check: 版本比较
  6. qyzs upgrade: 下载 → 解包 → 原子替换自身 (用可辨识标记包验证真的换了)
  7. qyzs upgrade --package: 本地安装包路径
  8. npm 包族装配 + 启动器平台调度

覆盖不到 (仍需真实环境, 见 docs/RELEASE.md §3):
  - Windows / macOS 平台的实际安装 (需对应 runner 与真机)
  - npm publish 与 provenance (需 NPM_TOKEN)
  - loong64 真机安装

用法: python3 tests/scripts/verify_release_pipeline.py
"""

from __future__ import annotations

import http.server
import io
import json
import os
import pathlib
import platform
import shutil
import subprocess
import sys
import tarfile
import threading
import zipfile

REPO = pathlib.Path(__file__).resolve().parents[2]
WORK = REPO / "tmp" / "release-verify"
PORT = 0  # 由内核分配空闲端口, 避免与其他服务冲突
BASE = ""  # serve() 后回填

PASS, FAIL = "\033[32mPASS\033[0m", "\033[31mFAIL\033[0m"
results: list[tuple[bool, str, str]] = []


def check(ok: bool, name: str, detail: str = "") -> bool:
    results.append((ok, name, detail))
    print(f"  [{PASS if ok else FAIL}] {name}" + (f" — {detail}" if detail else ""))
    return ok


def run(cmd, **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, cwd=REPO, **kw)


def host_target() -> str:
    """当前宿主对应的 CI target 三元组"""
    system, machine = platform.system(), platform.machine()
    if system == "Linux":
        if machine in ("x86_64", "AMD64"):
            return "x86_64-unknown-linux-musl"
        if machine in ("aarch64", "arm64"):
            return "aarch64-unknown-linux-musl"
        if machine in ("loongarch64", "loong64"):
            return "loongarch64-unknown-linux-gnu"
    if system == "Darwin":
        return "x86_64-apple-darwin" if machine in ("x86_64",) else "aarch64-apple-darwin"
    if system == "Windows":
        return "x86_64-pc-windows-msvc"
    raise SystemExit(f"不支持的宿主平台: {system}/{machine}")


def tar_gz(path: pathlib.Path, entry: str, data: bytes, mode: int = 0o755) -> None:
    with tarfile.open(path, "w:gz") as tf:
        ti = tarfile.TarInfo(entry)
        ti.size, ti.mode = len(data), mode
        tf.addfile(ti, io.BytesIO(data))


def zip_file(path: pathlib.Path, entry: str, data: bytes) -> None:
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zf:
        zi = zipfile.ZipInfo(entry)
        zi.external_attr = 0o755 << 16
        zf.writestr(zi, data)


def serve(root: pathlib.Path) -> http.server.ThreadingHTTPServer:
    global BASE
    handler = lambda *a, **kw: http.server.SimpleHTTPRequestHandler(*a, directory=str(root), **kw)
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), handler)
    BASE = f"http://127.0.0.1:{srv.server_address[1]}"
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def api_payload(tag: str, assets_dir: pathlib.Path) -> str:
    assets = [
        {
            "name": f.name,
            "browser_download_url": f"{BASE}/releases/download/{tag}/{f.name}",
            "size": f.stat().st_size,
        }
        for f in sorted(assets_dir.iterdir())
    ]
    # 必须与真实 GitHub API 一致: 压缩成单行 (真实响应的 body 里同样含
    # "tag_name" 字样, 换行解析会漏掉这个坑)
    return json.dumps(
        {
            "tag_name": tag,
            "name": tag,
            "html_url": BASE,
            "published_at": "2026-01-01T00:00:00Z",
            "assets": assets,
        },
        separators=(",", ":"),
    )


def main() -> int:
    target = host_target()
    ext = ".zip" if "windows" in target else ".tar.gz"
    print(f"宿主平台: {target}  (资产格式 {ext})\n")

    # ── 0. 环境准备 ────────────────────────────────────────────────────────
    if WORK.exists():
        shutil.rmtree(WORK)
    (WORK / "home").mkdir(parents=True)
    (WORK / "mock" / "api").mkdir(parents=True)

    print("== [1/8] 构建宿主平台 release 二进制 ==")
    env = dict(os.environ)
    if "musl" in target:
        env["PATH"] = "/usr/lib/llvm-19/bin:" + env.get("PATH", "")
    r = run(["cargo", "build", "--release", "--target", target], env=env)
    if not check(r.returncode == 0, "cargo build --release 成功", r.stderr.strip()[-200:]):
        return 1
    binname = "qyzs.exe" if "windows" in target else "qyzs"
    binary = REPO / "target" / target / "release" / binname
    check(binary.exists(), f"产物存在: {binary.relative_to(REPO)}")

    # ── 1. 组装模拟 Release ────────────────────────────────────────────────
    print("\n== [2/8] 组装模拟 Release 资产与 API ==")
    tag_install, tag_update = "v9.9.1", "v9.9.2"
    for tag in (tag_install, tag_update):
        d = WORK / "mock" / "releases" / "download" / tag
        d.mkdir(parents=True)
        data = binary.read_bytes()
        if ext == ".zip":
            zip_file(d / f"qyzs-{target}.zip", binname, data)
        else:
            tar_gz(d / f"qyzs-{target}.tar.gz", binname, data)
    # 自更新用可辨识标记包: 若解包逻辑失效, 替换结果会是压缩包字节而非可执行脚本
    marker = b'#!/bin/sh\necho "RELEASE-VERIFY-UPDATE-OK"\n'
    upd = WORK / "mock" / "releases" / "download" / tag_update
    if ext == ".zip":
        zip_file(upd / f"qyzs-{target}.zip", binname, marker)
    else:
        tar_gz(upd / f"qyzs-{target}.tar.gz", binname, marker)

    check(True, "资产已生成 (tag %s / %s)" % (tag_install, tag_update))

    srv = serve(WORK / "mock")
    # BASE 需在服务起��后回填, API 响应此时才能写出正确的下载地址
    (WORK / "mock" / "api" / "latest-install.json").write_text(
        api_payload(tag_install, WORK / "mock" / "releases" / "download" / tag_install)
    )
    (WORK / "mock" / "api" / "latest.json").write_text(
        api_payload(tag_update, upd)
    )
    check(True, "API 响应已生成")

    try:
        # ── 2. install.sh 全链路 ───────────────────────────────────────────
        print("\n== [3/8] install.sh 全链路 ==")
        env2 = dict(os.environ)
        env2.update(
            HOME=str(WORK / "home"),
            QYZS_RELEASE_API_URL=f"{BASE}/api/latest-install.json",
            QYZS_GITHUB_URL=BASE,
            QYZS_FALLBACK_TAG=tag_install,
        )
        r = run(["bash", "install.sh"], env=env2)
        installed = WORK / "home" / ".local" / "bin" / binname
        check(r.returncode == 0, "install.sh 退出码为 0", r.stdout.strip().splitlines()[-1] if r.stdout else "")
        check(installed.exists(), f"安装产物存在: {installed}")
        if installed.exists():
            os.chmod(installed, 0o755)
            r = run([str(installed), "--version"], env=env2)
            check(r.returncode == 0 and "qyzs" in r.stdout, "安装后可执行", r.stdout.strip())

        # ── 3. upgrade --check ─────────────────────────────────────────────
        print("\n== [4/8] qyzs upgrade --check ==")
        env3 = dict(os.environ)
        env3["QYZS_RELEASE_API_URL"] = f"{BASE}/api/latest.json"
        r = run([str(installed), "upgrade", "--check"], env=env3)
        check(r.returncode == 0, "upgrade --check 退出码为 0", r.stderr.strip()[:160])
        check("可升级" in r.stdout or "已是最新" in r.stdout, "给出可升级/已最新结论", r.stdout.strip()[:120])

        # ── 4. upgrade 真正替换自身 ────────────────────────────────────────
        print("\n== [5/8] qyzs upgrade (下载→解包→原子替换) ==")
        probe = WORK / "probe" / binname
        probe.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy(installed, probe)
        probe.chmod(0o755)
        before = probe.read_bytes()
        r = run([str(probe), "upgrade"], env=env3)
        check(r.returncode == 0, "upgrade 退出码为 0", r.stderr.strip()[:160])
        after = probe.read_bytes()
        check(before != after, "二进制已被替换")
        check(b"RELEASE-VERIFY-UPDATE-OK" in after, "替换后是解包出的内容 (而非压缩包原始字节)")
        r = run([str(probe)], env=env3)
        check(
            "RELEASE-VERIFY-UPDATE-OK" in r.stdout,
            "替换后的二进制可执行",
            r.stdout.strip()[:80],
        )
        mode = probe.stat().st_mode & 0o777
        check(mode == 0o755, "替换后仍具可执行权限", oct(mode))

        # ── 5. dataset 命令 (tests_data.zip 解压落点) ──────────────────────
        print("\n== [6/8] qyzs dataset 解压落点 ==")
        # CI 打包形态: cd tests && zip -qr ... data/* → 归档根即 data/ 的内容
        # 解压契约: 归档内容原样展开到 --dest, 不多不少一层
        ds_zip = WORK / "mock" / "releases" / "download" / tag_install / "tests_data.zip"
        with zipfile.ZipFile(ds_zip, "w", zipfile.ZIP_DEFLATED) as zf:
            zf.writestr("chat/demo.json", "{}")
            zf.writestr("ocr/demo2.json", "{}")
        ds_dest = WORK / "dsdata"
        r = run([str(installed), "dataset", "--url", f"{BASE}/releases/download/{tag_install}/tests_data.zip",
                 "--dest", str(ds_dest)])
        check(r.returncode == 0, "qyzs dataset 退出码为 0", r.stderr.strip()[:160])
        check(
            (ds_dest / "chat" / "demo.json").exists(),
            "数据集落在 <dest>/<类目>/<文件> (无多余层级)",
            f"实际: {sorted(str(p.relative_to(ds_dest)) for p in ds_dest.rglob('*') if p.is_file())}",
        )
        check(
            not (ds_dest / "data").exists(),
            "不应残留多余的 data/ 层",
        )


        # ── 7. install.ps1 逻辑 (Windows 脚本, 用 pwsh 在 Linux 上跑逻辑) ──
        print("\n== [7/8] install.ps1 逻辑 (pwsh) ==")
        if shutil.which("pwsh") is None:
            print("  [SKIP] 未安装 pwsh —— 跳过 (Windows 侧仍须真机验证)")
        else:
            # Windows 资产: .zip (install.ps1 走 Expand-Archive)
            win_zip = WORK / "mock" / "releases" / "download" / tag_install / "qyzs-x86_64-pc-windows-msvc.zip"
            with zipfile.ZipFile(win_zip, "w", zipfile.ZIP_DEFLATED) as zf:
                # 同时放 qyzs.exe (CI/Windows 真实形态) 与 qyzs (Linux 适配副本筛选用)
                for entry in ("qyzs.exe", "qyzs"):
                    zi = zipfile.ZipInfo(entry)
                    zi.external_attr = 0o755 << 16
                    zf.writestr(zi, binary.read_bytes())
            # Linux 兼容副本: 只改路径分隔符与可执行文件名, 逻辑一字不动
            ps_src = (REPO / "install.ps1").read_text(encoding="utf-8")
            ps_linux = (
                ps_src.replace("$env:LOCALAPPDATA\\qyzs\\bin", "$env:LOCALAPPDATA/qyzs/bin")
                .replace("qyzs.exe", "qyzs")
            )
            ps_copy = WORK / "install.ps1"
            ps_copy.write_text(ps_linux, encoding="utf-8")
            ps_home = WORK / "pshome"
            ps_home.mkdir(exist_ok=True)
            env4 = dict(os.environ)
            env4.update({
                "LOCALAPPDATA": str(ps_home),
                "TEMP": str(WORK),
                "TMP": str(WORK),
                "QYZS_RELEASE_API_URL": f"{BASE}/api/latest-install.json",
                "QYZS_GITHUB_URL": BASE,
                "QYZS_FALLBACK_TAG": tag_install,
            })
            r = run(["pwsh", "-NoProfile", "-File", str(ps_copy)], env=env4)
            ps_bin = ps_home / "qyzs" / "bin" / "qyzs"
            check(r.returncode == 0, "install.ps1 退出码为 0", (r.stdout + r.stderr).strip()[-200:])
            check(ps_bin.exists(), f"安装产物存在: {ps_bin}")
            if ps_bin.exists():
                ps_bin.chmod(0o755)
                r = run([str(ps_bin), "--version"], env=env4)
                check(r.returncode == 0 and "qyzs" in r.stdout, "安装后可执行", r.stdout.strip())

        # ── 8. npm 包族 ────────────────────────────────────────────────────
        print("\n== [8/8] npm 包族装配与启动器调度 ==")
        # npm/build.sh 要求 dist/<bin>-<pkg> 处预先放置二进制 (缺失即失败,
        # 避免发布空壳包); 这里放占位文件, 同时验证该严格校验生效
        dist = REPO / "dist"
        placeholders = {
            "qyzs-linux-x64": "qyzs",
            "qyzs-linux-arm64": "qyzs",
            "qyzs-linux-loong64": "qyzs",
            "qyzs-darwin-x64": "qyzs",
            "qyzs-darwin-arm64": "qyzs",
            "qyzs.exe-win32-x64": "qyzs.exe",
        }
        for fname in placeholders:
            (dist / fname).write_bytes(b"#!/bin/sh\n# placeholder\n")
        r = run(["bash", "npm/build.sh", "9.9.9"])
        check(r.returncode == 0, "npm/build.sh 成功", r.stderr.strip()[-160:])
        # 平台包必须真的带上了可执行文件
        empty = [
            pkg
            for pkg, binname in placeholders.items()
            if not (REPO / "dist" / "npm" / pkg.replace("qyzs.exe-", "").replace("qyzs-", "") / "bin" / binname).exists()
        ]
        check(not empty, "六个平台包均含可执行文件 (非空壳)", f"缺失: {empty}" if empty else "")
        pkg = json.loads((REPO / "dist/npm/cli/package.json").read_text())
        check(pkg["version"] == "9.9.9", "顶层包版本与传入值一致", pkg["version"])
        deps = {v.replace("@qyzs-ai/cli-", ""): v for v in pkg["optionalDependencies"]}
        check(
            all(v == "9.9.9" for v in pkg["optionalDependencies"].values()),
            "平台包版本钉与顶层同步",
            str(pkg["optionalDependencies"]),
        )
        # 平台覆盖三方一致: 启动器 / npm 装配 / CI 矩阵
        launcher = set(
            __import__("re").findall(
                r'return "@qyzs-ai/cli-([\w-]+)";',
                (REPO / "npm/cli/bin/qyzs.js").read_text(),
            )
        )
        built = {
            p.name
            for p in (REPO / "dist/npm").iterdir()
            if p.is_dir() and p.name != "cli"
        }
        check(launcher == built == set(deps), "启动器 / npm 装配 / 版本钉 平台包三方一致", f"{sorted(launcher)}")

        # 启动器实际调度 (仅 Linux/macOS 且有 node 时可验证)
        if platform.system() != "Windows" and shutil.which("node") is not None:
            nm = WORK / "nm" / "node_modules" / "@qyzs-ai"
            nm.mkdir(parents=True, exist_ok=True)
            shutil.copytree(REPO / "dist/npm/cli", nm / "cli")
            plat = {
                ("Linux", "x86_64"): "linux-x64",
                ("Linux", "aarch64"): "linux-arm64",
                ("Linux", "loongarch64"): "linux-loong64",
                ("Darwin", "x86_64"): "darwin-x64",
                ("Darwin", "arm64"): "darwin-arm64",
            }.get((platform.system(), platform.machine()))
            if plat:
                shutil.copytree(REPO / f"dist/npm/{plat}", nm / f"cli-{plat}")
                (nm / f"cli-{plat}" / "bin" / binname).write_bytes(binary.read_bytes())
                (nm / f"cli-{plat}" / "bin" / binname).chmod(0o755)
                r = run(["node", str(nm / "cli" / "bin" / "qyzs.js"), "--version"])
                check(r.returncode == 0 and "qyzs" in r.stdout, "npm 启动器调度到正确平台包", r.stdout.strip()[:80])
    finally:
        srv.shutdown()

    # ── 汇总 ───────────────────────────────────────────────────────────────
    failed = [x for x in results if not x[0]]
    print("\n" + "=" * 62)
    print(f"  发布流水线本地验证：通过 {len(results) - len(failed)} 项，失败 {len(failed)} 项")
    for _, name, detail in failed:
        print(f"    [FAIL] {name} — {detail}")
    print("=" * 62)
    print("  仍需真实环境验证：Windows/macOS 真机安装、npm publish + provenance、")
    print("  loong64 真机安装、真实 Release 上的 upgrade 链路 (docs/RELEASE.md §3)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
