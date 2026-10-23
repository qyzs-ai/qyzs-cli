#!/usr/bin/env bash
# 提交历史密钥扫描 (CI 门禁)
#
# 目标: 保证 API Key 等凭据**从未**进入 git 历史 —— 泄露风险由平台侧吊销处理,
# 但历史一旦写入凭据就必须重写历史, 因此必须在提交环节拦住。
#
# 扫描范围:
#   1. 被 git 跟踪的凭据文件 (.env / credentials.toml / *.key / *.pem)
#   2. **全对象库** —— 含可达提交与悬空/不可达对象 (曾 git add 后又删除的也会命中)
#   3. 当前工作区中已跟踪文件的内容
#
# 用法: bash tests/scripts/check_no_secrets.sh
# CI 中需 `fetch-depth: 0` (浅克隆拿不到完整历史)。

set -uo pipefail
cd "$(dirname "$0")/../.." || exit 1

# 已知安全的测试夹具 (tests/integration_tests.rs 的掩码用例, 假值, 非真实凭据)
ALLOWLIST='sk-abcdef1234567890wxyz'

# 疑似凭据模式
#   sk-<16+ 字母数字>  庆阳智算中心 / 通义系 API Key
#   AKIA…              AWS Access Key ID
#   gh?_/github_pat_   GitHub Token
#   npm_…              npm Token
#   PEM 私钥块
PATTERN='sk-[A-Za-z0-9]{16,}|AKIA[0-9A-Z]{16}|gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|npm_[A-Za-z0-9]{30,}|-----BEGIN [A-Z ]*PRIVATE KEY-----'

fail=0

echo "== [1/3] 凭据文件是否被 git 跟踪 =="
tracked=$(git ls-files | grep -E '(^|/)\.env$|(^|/)credentials\.toml$|\.key$|\.pem$' || true)
if [ -n "$tracked" ]; then
    echo "[FAIL] 以下凭据文件被 git 跟踪, 必须移出索引并加入 .gitignore:"
    echo "$tracked" | sed 's/^/    /'
    fail=1
else
    echo "[PASS] 无凭据文件被跟踪"
fi

echo "== [2/3] 全对象库扫描 (含悬空/不可达对象) =="
objects=$(git cat-file --batch-all-objects --batch 2>/dev/null \
    | grep -aoE "$PATTERN" | sort -u | grep -vFx "$ALLOWLIST" || true)
if [ -n "$objects" ]; then
    echo "[FAIL] git 对象库中发现疑似凭据 (可能已写入历史, 须重写历史):"
    echo "$objects" | sed 's/^/    /'
    fail=1
else
    echo "[PASS] 对象库中无疑似凭据"
fi

echo "== [3/3] 已跟踪文件内容扫描 =="
tree=$(git grep -aEIl "$PATTERN" -- . 2>/dev/null | while read -r f; do
    # 逐行排除白名单夹具所在文件中的该行
    git grep -aEn "$PATTERN" -- "$f" 2>/dev/null \
        | grep -vF "$ALLOWLIST"
done | head -20 || true)
if [ -n "$tree" ]; then
    echo "[FAIL] 已跟踪文件中发现疑似凭据:"
    echo "$tree" | sed 's/^/    /'
    fail=1
else
    echo "[PASS] 已跟踪文件中无疑似凭据"
fi

echo
if [ "$fail" -ne 0 ]; then
    echo "[FAIL] 密钥扫描未通过 —— 请勿提交任何凭据"
    exit 1
fi
echo "[OK] 密钥扫描通过: 提交历史中无 API Key 记录"
