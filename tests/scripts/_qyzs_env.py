"""Shared credential loader for Qingyang gateway test scripts.

Resolution order (highest first):
  1. `QYZS_APIKEY` environment variable (统一命名规范：QYZS_ 前缀)
  2. `QYZS_APIKEY` entry in the `.env` file at the repository root
     (git-ignored, see .env.example)

Usage in test scripts:
    from _qyzs_env import get_api_key
    API_KEY = get_api_key()
"""

import os
import sys

ENV_VAR = "QYZS_APIKEY"
REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))


def _from_dotenv() -> str:
    path = os.path.join(REPO_ROOT, ".env")
    if not os.path.exists(path):
        return ""
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, _, value = line.partition("=")
            if key.strip() == ENV_VAR:
                value = value.strip().strip('"').strip("'")
                if value:
                    return value
    return ""


def get_api_key() -> str:
    api_key = os.environ.get(ENV_VAR, "").strip() or _from_dotenv()
    if not api_key:
        sys.exit(
            f"ERROR: credential not found. Export {ENV_VAR}=<your-key> or set "
            f"{ENV_VAR}= in the .env file at the repository root (see .env.example)."
        )
    return api_key


def mask_key(api_key: str) -> str:
    if len(api_key) <= 10:
        return "***"
    return f"{api_key[:7]}***{api_key[-3:]}"
