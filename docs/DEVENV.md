# 本地开发环境搭建

本文档说明在 Linux 开发机上把 `qyzs` 的**六平台构建矩阵**在本地复现出来所需的
工具链。CI 的构建矩阵以 `.github/workflows/ci.yml` 为准，本文档与之对齐。

## 1. 必需组件

CI 使用 `dtolnay/rust-toolchain@stable`，本地保持同一 stable 工具链即可：

```bash
rustup component add clippy rustfmt
```

> 注意：`cargo clippy` / `cargo fmt` 在只装了 `rustup` 的机器上常常是「有 shim 无
> 组件」——命令存在但报 `is not installed for the toolchain`。用
> `rustup component list --installed` 核对。

## 2. 六个 CI 目标的 Rust 标准库

CI 已通过 `cross-target-check` job 在**原生 runner** 上检查 Windows 与 macOS 目标
本（windows-latest / macos-15-intel / macos-latest），PR 阶段不再有平台分支盲区。本地装齐
目标 std 的意义在于**提前发现平台相关编译问题**，而不用等 CI：

```bash
rustup target add \
  x86_64-unknown-linux-musl \
  aarch64-unknown-linux-musl \
  loongarch64-unknown-linux-gnu \
  x86_64-apple-darwin \
  aarch64-apple-darwin \
  x86_64-pc-windows-msvc
```

## 3. 交叉编译工具链

`ring`（rustls 的 TLS 实现）在 build script 里要编译目标平台的 C/汇编代码，
因此**仅装 Rust 标准库不足以完成交叉构建**，还需要目标平台的 C 工具链：

```bash
sudo apt-get install -y \
  musl-tools \                # x86_64-unknown-linux-musl
  gcc-aarch64-linux-gnu \     # aarch64 (glibc)
  gcc-loongarch64-linux-gnu \ # loongarch64-unknown-linux-gnu
  gcc-mingw-w64-x86-64 \      # x86_64-pc-windows-gnu (本地自检用)
  clang                       # aarch64-unknown-linux-musl + macOS 目标类型检查
```

## 4. 各目标的构建命令与已知边界

| 目标 | 命令 | 本地可行性 |
|---|---|---|
| `x86_64-unknown-linux-musl` | `cargo build --release --target x86_64-unknown-linux-musl` | ✅ 直接可用（musl-tools） |
| `aarch64-unknown-linux-musl` | 见下方 clang 配置 | ✅ 需 clang |
| `loongarch64-unknown-linux-gnu` | 见下方交叉编译器配置 | ✅ 需 `gcc-loongarch64-linux-gnu` |
| `x86_64-pc-windows-gnu` | `cargo build --release --target x86_64-pc-windows-gnu` | ✅ 需 mingw（**仅本地自检**，非发布目标） |
| `x86_64-pc-windows-msvc` | `cargo build --release --target x86_64-pc-windows-msvc` | ❌ 需 MSVC CRT / Windows SDK |
| `x86_64-apple-darwin` / `aarch64-apple-darwin` | 同上 | ❌ 需 Apple SDK |

### aarch64-unknown-linux-musl（Debian 无 aarch64 musl 工具链，用 clang 代替）

Debian 只提供 `gcc-aarch64-linux-gnu`（glibc），与 musl 目标不兼容。改用
clang 交叉编译 C 代码，musl libc 由 rustc 自带（`link-self-contained=yes`）：

```bash
export PATH="/usr/lib/llvm-19/bin:$PATH"
CC_aarch64_unknown_linux_musl="clang --target=aarch64-linux-musl" \
AR_aarch64_unknown_linux_musl="llvm-ar" \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="clang" \
RUSTFLAGS="-C linker=clang -C link-arg=--target=aarch64-linux-musl" \
cargo build --release --target aarch64-unknown-linux-musl
```

### loongarch64-unknown-linux-gnu

`cc` crate 不认识该 triple，会回退到宿主 `cc`，导致 `ring` 的 C 目标文件被编译成
x86_64（链接时报 `incompatible with elf64-x86-64`）。必须显式指定：

```bash
CC_loongarch64_unknown_linux_gnu=loongarch64-linux-gnu-gcc \
AR_loongarch64_unknown_linux_gnu=loongarch64-linux-gnu-ar \
CARGO_TARGET_LOONGARCH64_UNKNOWN_LINUX_GNU_LINKER=loongarch64-linux-gnu-gcc \
cargo build --release --target loongarch64-unknown-linux-gnu
```

### macOS / Windows msvc 目标的类型检查

`ring` 的 C 代码需要目标平台 SDK，Linux 上装不了。但 `src/term/i18n.rs` 中
macOS 分支（手写 CoreFoundation FFI）可单独抽出做类型检查——它不依赖 `ring`：

```bash
# 从 src/term/i18n.rs 抽出 #[cfg(target_os = "macos")] 块, 补最小桩类型后:
rustc --edition 2021 --crate-type lib --target x86_64-apple-darwin \
      --emit=metadata -o /tmp/rmeta lib.rs
```

Windows 分支同理用 `x86_64-pc-windows-gnu` 目标整体编译即可覆盖（ABI 不同，
但 `#[cfg(windows)]` 代码路径一致）。

## 5. 质量门禁

```bash
cargo fmt --all -- --check     # 格式门禁 (CI 已纳入)
cargo clippy --all-targets -- --deny warnings   # lint 门禁 (CI 已纳入, 须 0 告警)
cargo test                      # 284 项离线用例全绿（另 3 项真实网关用例按 #[ignore] 门控）
```

Shell 脚本（`install.sh` / `tests/scripts/*.sh` / `npm/build.sh`）用 shellcheck
检查（`install.cmd` 是 Windows batch，shellcheck 无法解析，勿纳入）：

```bash
sudo apt-get install -y shellcheck
shellcheck -S warning install.sh tests/scripts/*.sh npm/build.sh
```

> **当前代码库已是 rustfmt-clean**（`cargo fmt --all -- --check` 无差异），CI 亦
> 已将 fmt 纳入门禁。若日后引入超长行（如 `#[arg(...)]` 属性里的长中文 help
> 串），rustfmt 会把它折成多行——**直接跑 `cargo fmt` 提交结果即可**，不要手工
> 折行绕过。

CI 现有六个 job：`rust-check`（fmt + clippy + check + test）、`secret-scan`
（提交历史密钥扫描，checkout 需 `fetch-depth: 0`）、`release-pipeline-verify`
（模拟 Release 验证发布链路，必要时装 pwsh）、`cross-target-check`
（Windows/macOS/Linux-musl 目标类型检查，各自在原生 runner 上跑）、
`release-binaries`（tag 触发，六平台二进制）、`release-npm-and-scripts`
（tag 触发，npm 包族 + 安装脚本 + `tests_data.zip`）。

## 6. 脚本

`tests/scripts/` 下的脚本分为三类：

### 6.1 需真实网关凭证（经 `_qyzs_env.py` 读 `QYZS_APIKEY` 或仓库根目录 `.env`）

```bash
bash tests/scripts/verify_guardrails.sh   # 客户端护栏实测（5 阶段 / 7 项断言）
python3 tests/scripts/run_benchmarks.py    # 模型基准
python3 tests/scripts/stress_limits.py    # 上下文极限压测
python3 tests/scripts/limit_probe.py       # 参数与输入边界探针
python3 tests/scripts/probe_ocr_models.py  # OCR/VLM 探针计划（默认不联网）
python3 tests/scripts/probe_ocr_models.py --live --max-requests 20  # 有界在线实测
```

依赖：`requests`（多数脚本）、`Pillow`（`run_benchmarks` / `verify_guardrails`）、
`pypdf`（`limit_probe`）——均已验证可用。脚本自身不落盘任何 Key。
`probe_ocr_models.py` 的在线产物写入已忽略的 `tests/artifacts/ocr-capabilities/`，包含
公开基准输入、脱敏请求元数据与原始模型响应；必须显式 `--live` 才联网，并受
`--max-requests` 硬上限保护。Authorization 从不写入产物。

### 6.2 纯离线（不读凭证、不连网关）

```bash
python3 tests/scripts/prepare_dataset.py   # 基准数据集准备
python3 tests/scripts/validate_data.py     # 数据集静态校验
bash tests/scripts/check_no_secrets.sh     # 提交历史密钥扫描（CI secret-scan 门禁入口）
```

`prepare_dataset.py` 依赖 `PyMuPDF`（fitz）。`check_no_secrets.sh` 扫描**全对象库**
（含悬空对象）与已跟踪文件——`git add` 后又删除的凭据仍留在 `.git/objects` 里，
只有 `git gc` 才清除，故扫描不能只覆盖跟踪文件。

### 6.3 npm 发布（npm 的 OTP 策略与操作手册）

**npm 的现行策略（2026-10 实测）**：

| 操作 | Granular Access Token 的表现 |
|---|---|
| 创建**新**包（含 org scope） | **强制一次性密码**：网页/passkey 认证，或 `npm publish --otp=<认证器 6 位码>` |
| 给**已存在**的包发新版本 | 不需要 OTP |
| 读 org 的包列表 | 需要 org 读写权限，否则 `npm access list packages <org>` 返回 403 |

> 实测记录：同一个 token 在 **org scope 与 personal scope 建包时都报 EOTP**，说明这是
> 「建新包」的通用策略；而缺 org 权限时报的是 **E404**（npm 用 404 表示 token 看不见
> 该 org 的包，而非 403）——排查时极易误判为包名或网络问题。

**为什么 CI 不能自己建包**：OTP 需要网页/passkey 或认证器，GitHub Actions 无法完成。
npm 官方给 CI 的答案是 **Trusted Publishing (OIDC)**，但它的配置入口在「包设置页」，
即**包必须先存在** → 首包必须人工引导一次。

**操作手册**：

```bash
# ① 装配包族 (版本用 prerelease 占位, 让 CI 之后能发正式版 0.2.0 —— npm 不允许
#    重复发布同一版本)
#    注意: npm/build.sh 要求 dist/<bin>-<pkg> 处预先放好二进制, 缺失即失败
bash npm/build.sh 0.2.0-0

  # ② 人工发布首包 —— 7 个包逐个发布 (用 npm 认证器 App 的 6 位码)
  #    一个 OTP 能否复用于同批多个包未经实测; 若报 OTP 失效就重新取码。
  #    用 --tag next 把预发布版标为 next, 避免抢占 latest 标签。
  for p in cli linux-x64 linux-arm64 linux-loong64 darwin-x64 darwin-arm64 win32-x64; do
    (cd "dist/npm/$p" && npm publish --tag next --access public --otp=<6位码>)
  done

# ③ 为每个包配置 Trusted Publishing (OIDC):
#    npmjs.com → 包设置页 → Trusted Publisher → GitHub Actions
#    填 Organization/user=qyzs-ai, Repository=qyzs-cli, Workflow filename=ci.yml
#    7 个包各配一次 (顶层 @qyzs-ai/cli + 六个平台包)

# ④ 此后 CI 发版不再需要任何 secret:
#    npm publish --provenance 走 OIDC, provenance 自动生成
```

**已知踩坑**：

- `npm` 的报错与调试日志里，OTP 认证 URL 会被打码成 `***`；只有在 **TTY 环境**
  才显示真地址（`script -qec "npm publish" /dev/null` 可拿到）。
- GitHub 的 **Actions secret 属于 sudo-protected 操作**：更新/删除会跳转
  "Confirm access"（passkey / 认证器 / 密码三选一），且该确认页与 secret 更新
  **是同一个表单**（填好验证码提交即一步完成两件事）。但「新建」secret 不触发该确认，
  可脚本化提交 —— 自动化场景的正确姿势是**首次创建用脚本**，后续轮换需人工过一次确认
  （确认后进入数小时的 sudo 模式，期间可继续操作）。
- secret 值在表单里是 `encrypted_value`，由 `js-encrypt-submit` 用仓库公钥在浏览器端
  加密；直接 `form.submit()` 跳过该处理器会提交明文，服务端解密失败报 Server Error。
  正确做法是**派发 `submit` 事件**让处理器自己加密。

### 6.4 Release 资产构成（业界惯例对照）

调研 2026-10 的对标项目（deno / bun / rustup / gh / azure-cli / uv）：

| 项目 | Release 含引导脚本 | 脚本规范入口 | in-release 脚本钉版本 | SHA256SUMS |
|---|---|---|---|---|
| deno / bun / rustup / gh | ❌ | 独立稳定地址 | — | ✅ |
| azure-cli | ❌（含 `.msi` 真安装包） | 官方文档 | — | ✅ |
| uv | ✅ | `astral.sh/uv/install.sh` | ✅ | ✅ |
| **本项目** | ✅ **钉定版本** | `raw@main`（最新版）+ Release 资产（钉定版） | ✅ | ✅ |

规则：**仓库里的脚本 = 装最新版；Release 资产里的脚本 = 装该 tag**。
CI 通过替换 `install.sh` 的 `PINNED_VERSION` 与 `install.ps1` 的 `$PinnedVersion`
实现（见 `.github/workflows/ci.yml` 的 release-create job）。新增 `release-checksums`
job 在所有资产就位后生成 `SHA256SUMS` 并上传；两个安装脚本下载后自动比对，
不匹配即中止安装（校验和文件缺失时降级为警告）。

未做（需向外部仓库提 PR，属独立决策）：**winget manifest**（Windows 主流安装
路径，ripgrep / sd 均在其仓库中）与 **Homebrew formula**（macOS 主流路径）。

### 6.5 发布流水线本地验证（无需真实发布）

```bash
python3 tests/scripts/verify_release_pipeline.py
```

用本地模拟的 GitHub Release 端到端验证发布链路：构建宿主平台 release 二进制 →
按 CI 资产命名组装 `.tar.gz`/`.zip` 与 Release API 响应 → 起本地 HTTP 服务 →
验证 install.sh 全链路、`upgrade --check` 版本比较、`upgrade` 下载→解包→原子替换
自身、`upgrade --package` 本地安装包、`qyzs dataset` 解压落点、install.ps1 逻辑、
npm 包族装配与启动器平台调度。

当前 25/25 通过。其中 install.ps1 段需要 `pwsh`（PowerShell 7），未安装则自动跳过：

```bash
PWSH_VERSION=7.6.8
curl -sSL -o /tmp/pwsh.tar.gz \
  "https://github.com/PowerShell/PowerShell/releases/download/v${PWSH_VERSION}/powershell-${PWSH_VERSION}-linux-x64.tar.gz"
sudo mkdir -p /opt/microsoft/powershell/7
sudo tar -xzf /tmp/pwsh.tar.gz -C /opt/microsoft/powershell/7
sudo chmod +x /opt/microsoft/powershell/7/pwsh
sudo ln -sf /opt/microsoft/powershell/7/pwsh /usr/local/bin/pwsh
```

该脚本**不能替代**真实发布验证——Windows/macOS 真机安装、`npm publish` +
provenance、loong64 真机安装、真实 Release 上的 upgrade 链路，仍须按
`docs/RELEASE.md` §3 的 C8~C10 逐项完成。
