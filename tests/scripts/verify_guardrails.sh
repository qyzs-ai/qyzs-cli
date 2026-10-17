#!/usr/bin/env bash
# 客户端护栏实测验证脚本（对齐 tests/docs/QYZS-ANALYZE.md §3.6 与 §5.1 实测边界）
#
# 验证内容：
#   1. bge-m3 短文本基线
#   2. bge-m3 超长文本（13,000 字）自动分块 + 均值池化
#   3. bge-large-zh-v1.5 短向量模型 600 字分块（实测 500 通过 / 800 拒绝）
#   4. 批量拆分：300 条输入按 256 条/请求自动拆分
#   5. chat 20 图自动分批（16 图可靠上限，拆为 16+4 两个子请求）
#
# 凭证：经环境变量 QYZS_APIKEY 提供，或写入仓库根目录 .env（模板见 .env.example）。
#       脚本自身不落盘任何 Key。
#
# 用法：bash tests/scripts/verify_guardrails.sh

set -euo pipefail
cd "$(dirname "$0")/../.."

REPO_ROOT="$(pwd)"
BIN="$REPO_ROOT/target/debug/qyzs"

# ── 凭证预检（只检查存在性，不回显内容）─────────────────────────────────────
# qyzs CLI 本体不解析 .env，脚本检测到 .env 中的 Key 时代为注入当前进程环境。
have_key=false
if [ -n "${QYZS_APIKEY:-}" ]; then
  have_key=true
elif [ -f "$REPO_ROOT/.env" ]; then
  while IFS='=' read -r k v; do
    k="$(echo "$k" | tr -d '[:space:]')"
    if [ "$k" = "QYZS_APIKEY" ]; then
      v="$(echo "$v" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//;s/^"\(.*\)"$/\1/;s/^'"'"'\(.*\)'"'"'$/\1/')"
      if [ -n "$v" ]; then export "QYZS_APIKEY=$v"; have_key=true; fi
    fi
  done < <(grep -v '^[[:space:]]*#' "$REPO_ROOT/.env")
fi
if ! $have_key; then
  echo "[SKIP] 未检测到凭证（环境变量 QYZS_APIKEY 或仓库根目录 .env）。"
  echo "       设置后重新运行本脚本即可执行护栏实测。"
  exit 2
fi

echo "== [0/5] 构建调试版 CLI =="
cargo build -q

pass=0; fail=0
check() { # check <用例名> <期望grep模式>
  local name="$1" pattern="$2"
  if echo "$OUT" | grep -q "$pattern"; then
    echo "[PASS] $name"; pass=$((pass+1))
  else
    echo "[FAIL] $name （输出中未找到: $pattern）"
    echo "------- 实际输出（截断）-------"
    echo "$OUT" | head -15
    fail=$((fail+1))
  fi
}

echo "== [1/5] bge-m3 短文本基线 =="
OUT=$("$BIN" embed "短文本基线测试")
check "短文本基线 (Index 0, 1024 维)" "Index 0: 向量维度 1024"

echo "== [2/5] bge-m3 超长文本分块 (13,000 字 → 12,000+1,000 均值池化) =="
LONG=$(python3 -c "print('自然资源空间规划政策条款测试。' * 1000)")
OUT=$("$BIN" embed "$LONG")
check "超长文本分块 (Index 0, 1024 维, 无 HTTP 500)" "Index 0: 向量维度 1024"

echo "== [3/5] bge-large-zh-v1.5 短向量模型 600 字分块 (阈值 500) =="
MID=$(python3 -c "print('公文测试文本。' * 100)")
OUT=$("$BIN" embed -m bge-large-zh-v1.5 "$MID")
check "短向量模型 600 字分块 (Index 0, 1024 维)" "Index 0: 向量维度 1024"

echo "== [4/5] 批量拆分：300 条输入 (>256 条/请求上限) =="
ARGS=()
for i in $(seq 1 300); do ARGS+=("批量拆分测试文本$i"); done
OUT=$("$BIN" embed "${ARGS[@]}")
COUNT=$(echo "$OUT" | grep -c "向量维度" || true)
if [ "$COUNT" -eq 300 ]; then
  echo "[PASS] 300 条输入全部回填 (实际 $COUNT 条)"; pass=$((pass+1))
else
  echo "[FAIL] 300 条输入回填数不符 (期望 300，实际 $COUNT)"; fail=$((fail+1))
fi

echo "== [5/5] chat 20 图自动分批 (16 图可靠上限 → 16+4) =="
python3 - <<'PY'
from PIL import Image
import os
d = "target/vlm_batch_imgs"
os.makedirs(d, exist_ok=True)
for i in range(20):
    Image.new("RGB", (64, 64), ((i * 12) % 255, 100, 200)).save(f"{d}/img{i:02d}.png")
PY
CHAT_ARGS=()
for f in target/vlm_batch_imgs/img*.png; do CHAT_ARGS+=(-i "$f"); done
OUT=$("$BIN" chat --no-stream "请简要描述这些图片的颜色特征。" "${CHAT_ARGS[@]}")
check "批次 1/2 分区输出" "批次 1/2"
check "批次 2/2 分区输出" "批次 2/2"

echo ""
echo "=================================================="
echo "  护栏实测完成：通过 $pass 项，失败 $fail 项"
echo "=================================================="
[ "$fail" -eq 0 ]
