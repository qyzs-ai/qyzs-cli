#!/usr/bin/env python3
"""帮助文本 ↔ 参数校验 ↔ 文档 三方一致性检查

从源码直接抽取事实, 不依赖人工记忆:

  1. `src/cli/args.rs`   —— 每个参数的 help 文案、默认值、取值白名单
  2. `src/cli/validate.rs` —— 每个参数的硬校验边界 (区间文案)
  3. `src/cli/dispatch.rs` —— dispatch 层的内联校验 (如 --min-confidence)

然后检查:
  - 有硬校验边界的参数, 其 help 是否写明了该边界
  - 有取值白名单的参数, 其 help 是否列出了全部取值
  - 有默认值的参数, 其 help 是否写明了该默认值

用法:
    python3 tests/scripts/check_help_contract.py            # 报告不一致
    python3 tests/scripts/check_help_contract.py --check   # 有不一致时退出码 1
"""

from __future__ import annotations

import pathlib
import re
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]
ARGS = (ROOT / "src/cli/args.rs").read_text(encoding="utf-8")
VALIDATE = (ROOT / "src/cli/validate.rs").read_text(encoding="utf-8")
DISPATCH = (ROOT / "src/cli/dispatch.rs").read_text(encoding="utf-8")

# validate.rs 函数名 -> (参数 flag, 边界文案)
# 边界文案须与函数体里的报错文案一致 (用户看到的是报错, help 应当预告同样的范围)
VALIDATORS = {
    "validate_chat_temperature": ("--temperature", "0.0~2.0"),
    "validate_chat_top_p": ("--top-p", "0.0~1.0"),
    "validate_chat_max_tokens": ("--max-tokens", "1~1000000"),
    "validate_chat_images": ("--image", "16"),
    "validate_ocr_max_dim": ("--max-dim", "1~8192"),
    "validate_rerank_top_n": ("--top-n", "1~1000"),
    "validate_usage_days": ("--days", "1~3650"),
    "validate_scan_segment_chars": ("--segment-chars", "1~100000"),
    "validate_sam_threshold": ("--threshold", "0.0~1.0"),
    "validate_usage_kind": ("--kind", None),  # 白名单类, 单独校验
    "validate_pdf_split_chunk_size": ("--chunk-size", "1"),
}

# dispatch.rs 内联校验 (无独立 validate 函数)
INLINE = [("--min-confidence", "0.0~1.0", "min_confidence 取值必须在 0.0~1.0 之间")]


def _scan_arg_bodies(src: str) -> list[tuple[str, str]]:
    """扫描出所有 #[arg(...)] 的 (body, 紧随其后的字段名)。

    不能用正则: help 文案里含括号 (如 "(0.0~2.0)"), 非贪婪匹配会在字符串内的
    ')' 处提前截断。故手工扫描, 跟踪引号状态与括号深度。
    """
    out: list[tuple[str, str]] = []
    i = 0
    while True:
        j = src.find("#[arg(", i)
        if j < 0:
            break
        k = j + len("#[arg(")
        depth = 1
        in_str = False
        esc = False
        body_start = k
        while k < len(src) and depth > 0:
            c = src[k]
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = not in_str
            elif not in_str:
                if c == "(":
                    depth += 1
                elif c == ")":
                    depth -= 1
            k += 1
        body = src[body_start : k - 1]
        # 其后紧跟的字段名
        m = re.match(r"\]\s*\n\s*(?:pub\s+)?(r#\w+|\w+)\s*:", src[k:])
        if m:
            out.append((body, m.group(1)))
        i = k
    return out


def parse_args() -> list[dict]:
    """抽取 args.rs 里每个 #[arg(...)] 定义的参数"""
    out: list[dict] = []
    for body, field in _scan_arg_bodies(ARGS):
        # clap derive 有两种写法: `long = "显式名"` 与裸 `long` (由字段名推导)
        long = re.search(r'long\s*=\s*"([\w-]+)"', body)
        if long:
            flag = "--" + long.group(1)
        elif re.search(r'(?<![\w-])long(?![\w-])', body):
            bare = field.removeprefix("r#")
            flag = "--" + bare.replace("_", "-")
        else:
            continue  # 只有短形式, 不参与长参数契约
        help_m = re.search(r'help\s*=\s*"((?:[^"\\]|\\.)*)"', body, re.S)
        help_text = help_m.group(1) if help_m else ""
        help_text = help_text.replace('\\"', '"').replace("\\\\", "\\")
        default_m = re.search(r'default_value\s*=\s*"([^"]*)"', body)
        default = default_m.group(1) if default_m else None
        values_m = re.search(
            r'PossibleValuesParser::new\(\[([^\]]*)\]\)', body, re.S
        )
        values = None
        if values_m:
            values = re.findall(r'"([^"]*)"', values_m.group(1))
        out.append(
            {
                "flag": flag,
                "field": field,
                "help": help_text,
                "default": default,
                "values": values,
            }
        )
    return out


def validate_bounds() -> dict[str, str]:
    """从 validate.rs 的报错文案里取真实边界"""
    bounds: dict[str, str] = {}
    for fn, (flag, expect) in VALIDATORS.items():
        m = re.search(
            r"pub\(crate\) fn " + fn + r".*?\n\}", VALIDATE, re.S
        )
        if not m:
            continue
        body = m.group(0)
        # 报错文案里的边界, 如 "--temperature 取值必须在 0.0~2.0 之间"
        bm = re.search(r"取值必须在 ([^ ]+) 之间|取值必须 ([^ ]+)", body)
        if bm:
            bounds[flag] = (bm.group(1) or bm.group(2)).strip()
    return bounds


# ── 文档侧 ────────────────────────────────────────────────────────────────
# 扫描所有文档里出现的 `--flag`, 与 args.rs 的真实参数面比对:
# 文档若提到已删除/从未存在的参数, 用户照抄会得到 "unexpected argument"。
DOC_GLOBS = [
    "README.md",
    "CHANGELOG.md",
    "AGENTS.md",
    "docs/*.md",
    "tests/docs/*.md",
]
# 这些是文档里出现但不属于 CLI 参数面的用法 (工具自身的选项等)
DOC_FLAG_ALLOWLIST = {
    "--provenance",       # npm publish 的选项
    "--access",          # npm publish 的选项
    "--target",          # cargo 的选项
    "--locked",
    "--frozen",
    "--offline",
    "--release",
    "--version",
    "--help",
    "--all-targets",
    "--deny-warnings",
    "--check",
    "--allow-dirty",
    "--allow-staged",
    "--all-targets",
    "--no-modify-path",
    "--default-toolchain",
    "--no-default-features",
    "--features",
    "--package",
    "--workspace",
    "--manifest-path",
    "--dry-run",
    "--fail-on-unmatched-files",
    "--generate-release-notes",
    "--password",
    "--otp",
    "--scopes",
    "--orgs",
    "--bypass-2fa",
    "--packages",
    "--packages-all",
    "--read-only",
    "--expires",
    "--token-description",
    "--cidr",
    "--otp",
    "--tag",
    "--registry",
    "--prefix",
    # 文档中明确标注「从未实现」的参数 (保留说明其不存在, 避免读者照抄)
    "--strip-seals",
}


def doc_flag_problems() -> list[str]:
    """文档里提到的 --flag 是否都真实存在"""
    real = {a["flag"] for a in parse_args()}
    # 短形式也算 (如 -m/--model)
    real |= set(re.findall(r"--[a-z][a-z0-9-]*", ARGS))
    problems: list[str] = []
    for pat in DOC_GLOBS:
        for f in sorted(ROOT.glob(pat)):
            in_shell_fence = False
            for i, line in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
                stripped = line.strip()
                # 跟踪 shell 代码块: 块内的 --flag 才可能是 qyzs 参数
                if stripped.startswith("```"):
                    in_shell_fence = not in_shell_fence
                    continue
                if stripped.startswith(">"):
                    continue  # 引用块
                # 判定: 该行确实是 qyzs 命令 (cargo/rustc 自身的选项不算)
                looks_like_qyzs = bool(
                    re.search(r"(?<![\w-])qyzs(?![\w-])", stripped)
                )
                if not looks_like_qyzs:
                    continue
                for flag in sorted(set(re.findall(r"(?<![\w-])--[a-z][a-z0-9-]+", line))):
                    if flag in real or flag in DOC_FLAG_ALLOWLIST:
                        continue
                    if f.name == "CHANGELOG.md":
                        continue  # CHANGELOG 记述历史参数名, 非当前用法
                    problems.append(
                        f"{f.relative_to(ROOT)}:{i}: 文档提到 {flag}, 但 CLI 无此参数 — {stripped[:80]}"
                    )
    return problems


def script_list_problems() -> list[str]:
    """文档里的脚本清单 ↔ tests/scripts/ 实际内容。

    两个方向都要查:
      - 文档提到但目录里没有 → 读者照抄会失败
      - 目录里有但任何文档都没提 → 新脚本无人知晓, 等于没写
    """
    problems: list[str] = []
    scripts_dir = ROOT / "tests" / "scripts"
    actual = {
        p.name
        for p in scripts_dir.iterdir()
        if p.is_file() and p.suffix in (".py", ".sh") and not p.name.startswith("_")
    }
    # 文档里出现的脚本名。只认带 tests/scripts/ 前缀的写法, 以及与实际目录
    # 同名的反引号写法 —— 否则会把 npm/build.sh、packaging/*.py 这类别处脚本
    # 误判成缺失。
    mentioned: set[str] = set()
    for pat in DOC_GLOBS:
        for f in sorted(ROOT.glob(pat)):
            text = f.read_text(encoding="utf-8")
            prefixed = set(re.findall(r"tests/scripts/([\w.]+\.(?:py|sh))", text))
            for name in prefixed:
                mentioned.add(name)
            # 反引号写法: 仅当它确实是 tests/scripts/ 下的脚本才算数
            for name in re.findall(r"`([\w.]+\.(?:py|sh))`", text):
                if name in actual:
                    mentioned.add(name)
    for name in sorted(mentioned - actual):
        if name.endswith(".json") or name.startswith("_"):
            continue  # 辅助模块与工件文件
        problems.append(f"文档提到 {name}, 但 tests/scripts/ 下不存在")
    for name in sorted(actual - mentioned):
        problems.append(
            f"tests/scripts/{name} 未被任何文档提及 (新脚本无人知晓, 等于没写)"
        )
    # 权威脚本清单 (tests/docs/QYZS-TESTPLAN.md §7) 必须列出全部脚本 ——
    # "任何文档提到即可" 太宽松: 该节是清单的正式载体, 漏了它读者就看不到
    canon = ROOT / "tests" / "docs" / "QYZS-TESTPLAN.md"
    if canon.exists():
        ctext = canon.read_text(encoding="utf-8")
        for name in sorted(actual):
            if name not in ctext:
                problems.append(
                    f"tests/scripts/{name} 未列入 {canon.relative_to(ROOT)} §7 的脚本清单"
                )
    return problems


# ── 运行时退出码 ↔ README 退出码表 ──────────────────────────────────────────
# README 的退出码表是给脚本调用方看的契约, 必须与真实行为一致。
# 下面每一项都用真实调用触发, 再与 README 表里声明的码比对。
def runtime_exit_code_problems(binary: pathlib.Path) -> list[str]:
    import http.server
    import os
    import subprocess
    import tempfile
    import threading

    problems: list[str] = []
    env_base = dict(os.environ)
    env_base["QYZS_APIKEY"] = "dummy-key-for-exit-code-probe"
    env_no_key = {k: v for k, v in env_base.items() if k != "QYZS_APIKEY"}

    def run(args: list[str], env: dict[str, int] | None = None) -> int:
        r = subprocess.run(
            [str(binary), *args], capture_output=True, text=True, timeout=60, env=env or env_base
        )
        return r.returncode

    # 本地 HTTP 服务: /500 返回 5xx, /slow 挂起 (触发客户端超时)
    class H(http.server.BaseHTTPRequestHandler):
        def do_POST(self):
            try:
                if self.path.startswith("/slow"):
                    time.sleep(5)
                self.send_response(500)
                self.end_headers()
                self.wfile.write(b"server error")
            except (BrokenPipeError, ConnectionResetError):
                pass  # 客户端超时先断开, 属预期

        def log_message(self, *a):
            pass

    srv = http.server.HTTPServer(("127.0.0.1", 0), H)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    tmp = pathlib.Path(tempfile.mkdtemp())
    bad_cfg = tmp / "cfg"
    (bad_cfg / "qyzs").mkdir(parents=True)
    (bad_cfg / "qyzs" / "config.toml").write_text("not valid toml [[[\n", encoding="utf-8")

    cases: list[tuple[str, int, list[str], dict | None]] = [
        ("成功 (--help)", 0, ["--help"], None),
        ("用法错误: 无参数", 2, [], None),
        ("用法错误: 未知子命令", 2, ["bogus"], None),
        ("用法错误: 未知参数", 2, ["chat", "--bogus"], None),
        ("用法错误: 枚举值非法", 2, ["chat", "--lang", "french", "hi"], None),
        ("用法错误: 数值越界", 2, ["chat", "--temperature", "9.9", "hi"], None),
        ("用法错误: 互斥参数", 2, ["chat", "--stream", "--no-stream", "hi"], None),
        ("认证失败: 缺 API Key", 3, ["chat", "hi"], env_no_key),
        ("IO 失败: 文件不存在", 5, ["asr", "/nonexistent/x.wav"], None),
        # 连接失败: 用一个确定没在监听的端口 (9 = discard, 通常未开放)
        ("服务不可达", 69, ["-b", "http://127.0.0.1:9/x", "chat", "hi"], None),
        ("服务端错误 (5xx)", 70, ["-b", f"http://127.0.0.1:{port}/500", "chat", "hi"], None),
        ("请求超时", 4, ["-b", f"http://127.0.0.1:{port}/slow", "--timeout", "1", "chat", "hi"], None),
        (
            "配置错误: 配置文件格式错误",
            78,
            ["config", "get", "network.base_url"],
            {**env_base, "XDG_CONFIG_HOME": str(bad_cfg)},
        ),
    ]
    for name, expect, args, env in cases:
        got = run(args, env)
        if got != expect:
            problems.append(f"退出码不符: {name} 期望 {expect} 实际 {got}")
        else:
            print(f"    [PASS] {name} → {got}")

    srv.shutdown()

    # README 的退出码表须与 error.rs 的映射一致
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    err_rs = (ROOT / "src" / "error.rs").read_text(encoding="utf-8")
    code_map = {
        m.group(1): int(m.group(2))
        for m in re.finditer(
            r"pub const (USAGE_ERR|AUTH_ERR|TIMEOUT_ERR|IO_ERR|EX_UNAVAILABLE|EX_SOFTWARE|EX_CONFIG): i32 = (\d+);",
            err_rs,
        )
    }
    for const, val in code_map.items():
        if str(val) not in readme:
            problems.append(f"README 退出码表缺 {val} ({const})")
    return problems


def main() -> int:
    args = parse_args()
    bounds = validate_bounds()
    problems: list[str] = []
    notes: list[str] = []

    by_flag = {a["flag"]: a for a in args}

    # 1) 硬校验边界须在 help 中预告
    for flag, bound in bounds.items():
        a = by_flag.get(flag)
        if not a:
            problems.append(f"{flag}: validate.rs 有边界校验, 但 args.rs 里找不到该参数")
            continue
        if bound not in a["help"]:
            problems.append(
                f"{flag}: help 未写明校验边界 '{bound}' (实际校验: {bound}) — "
                f"help: \"{a['help'][:90]}\""
            )

    # 2) dispatch 内联校验
    for flag, bound, _ in INLINE:
        a = by_flag.get(flag)
        if not a:
            problems.append(f"{flag}: dispatch 有内联校验, 但 args.rs 里找不到该参数")
            continue
        if bound not in a["help"]:
            problems.append(
                f"{flag}: help 未写明边界 '{bound}' — help: \"{a['help'][:90]}\""
            )

    # 3) 取值白名单须在 help 中列全
    for a in args:
        if not a["values"]:
            continue
        missing = [v for v in a["values"] if v not in a["help"]]
        if missing:
            problems.append(
                f"{a['flag']}: help 未列出取值 {missing} (白名单共 {a['values']})"
            )

    # 4) 默认值须在 help 中写明 (仅对有 default_value 的)
    for a in args:
        if a["default"] is None:
            continue
        d = a["default"]
        if d in ("true", "false"):
            # 布尔默认值: 检查 help 是否提到 "默认开启" / "默认关闭" 或该字面量
            want = "默认开启" if d == "true" else "默认"
            if want not in a["help"] and d not in a["help"]:
                problems.append(
                    f"{a['flag']}: 默认值 {d} 未在 help 中说明 — \"{a['help'][:90]}\""
                )
            continue
        if d not in a["help"]:
            # URL 型默认值: help 里写"默认 vX.Y.Z 快照"即可, 不必重复整条 URL
            tag_m = re.search(r"v\d+\.\d+(\.\d+)?", d)
            if tag_m and tag_m.group(0) in a["help"]:
                continue
            problems.append(
                f"{a['flag']}: 默认值 '{d}' 未在 help 中写明 — \"{a['help'][:90]}\""
            )

    doc_problems = doc_flag_problems()
    problems.extend(doc_problems)
    script_problems = script_list_problems()
    problems.extend(script_problems)

    # 运行时退出码核验 (需已构建的二进制; 没有则跳过)
    binary = ROOT / "target" / "debug" / "qyzs"
    if binary.exists():
        print("  运行时退出码核验:")
        problems.extend(runtime_exit_code_problems(binary))
    else:
        notes.append("未找到 target/debug/qyzs, 跳过运行时退出码核验")

    print(f"检查参数 {len(args)} 个, 边界映射 {len(bounds)} 条, 文档 {len(DOC_GLOBS)} 类")
    for p in problems:
        print(f"  [FAIL] {p}")
    for n in notes:
        print(f"  [WARN] {n}")
    print(f"  → {'全部一致' if not problems else f'{len(problems)} 处不一致'}")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
