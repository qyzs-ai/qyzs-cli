# 打包清单 (winget / Homebrew)

本目录是 **Windows Package Manager (winget)** 与 **Homebrew** 的打包清单源。
两者都是各平台的**主流安装路径**——没有它们，Windows 用户只能靠 PowerShell 脚本、
macOS 用户只能靠 shell 脚本, 都不是平台惯例。

## 目录内容

```
packaging/
├── winget/
│   ├── QYZS-AI.qyzs.yaml                 版本清单 (ManifestType: version)
│   ├── QYZS-AI.qyzs.installer.yaml       安装器清单 (ManifestType: installer)
│   └── QYZS-AI.qyzs.locale.en-US.yaml    本地化清单 (ManifestType: defaultLocale)
├── homebrew/
│   └── qyzs.rb                            Homebrew formula
└── generate_manifests.py                 按 Release 真实资产重算清单
```

## 为什么要生成器

每次发版, 三个平台归档的**哈希都会变**, 而 winget 的 `InstallerSha256` 与
Homebrew 的 `sha256` 必须与实际资产一致——手改极易漏改或抄错。`generate_manifests.py`
从该 tag 的 `SHA256SUMS` 自动取真实哈希并重写清单, 消除这类错误。

```bash
# 校验现有清单 (不改文件)
python3 packaging/generate_manifests.py --check --version v0.2.0

# 按线上 Release 重新生成 (版本 + 哈希)
python3 packaging/generate_manifests.py --version v0.2.0
```

## 版本升级流程

1. 打 tag 并等 Release 流水线完成 (资产与 `SHA256SUMS` 均已上传)；
2. 更新 `Cargo.toml` 的 `version` 与 `CHANGELOG.md` 的版本段;
3. 跑生成器: `python3 packaging/generate_manifests.py --version vX.Y.Z`;
4. 提交本目录的变更;
5. 提交 winget PR (见下)。

## 提交到 winget-pkgs

winget 清单存放在社区仓库 `microsoft/winget-pkgs`, 路径规范为
`manifests/<Publisher 首字母小写>/<Publisher>.<Package>/<version>/`。

```bash
# 1) fork 并克隆
git clone https://github.com/<你的账号>/winget-pkgs.git
cd winget-pkgs

# 2) 拷贝生成的三个文件到约定路径
DEST="manifests/q/QYZS-AI/qyzs/0.2.0"
mkdir -p "$DEST"
cp <仓库>/packaging/winget/QYZS-AI.qyzs.yaml              "$DEST/"
cp <仓库>/packaging/winget/QYZS-AI.qyzs.installer.yaml    "$DEST/"
cp <仓库>/packaging/winget/QYZS-AI.qyzs.locale.en-US.yaml "$DEST/"

# 3) 提交并开 PR
git checkout -b add-qyzs-0.2.0
git add "$DEST"
git commit -m "New package: QYZS-AI.qyzs version 0.2.0"
git push origin add-qyzs-0.2.0
# 然后在 GitHub 上开 PR 到 microsoft/winget-pkgs
```

> **新 publisher 的 PR 需要人工审核** (winget 的常规流程)。本项目尚未注册为
> verified publisher, 首个 PR 会被排队人工审核, 属正常。

### 本地校验清单格式

清单已按官方 JSON schema 校验 (schema 来自
`microsoft/winget-cli/schemas/JSON/manifests/latest/manifest.*.latest.json`):

```bash
pip install --break-system-packages pyyaml jsonschema
for f in version installer defaultLocale; do
  curl -sL "https://raw.githubusercontent.com/microsoft/winget-cli/master/schemas/JSON/manifests/latest/manifest.$f.latest.json" -o "/tmp/$f.json"
done
# 然后用 jsonschema 校验 packaging/winget/ 下对应的三个文件
```

## 提交到 Homebrew

Homebrew 官方 tap (`homebrew/core`) 只收录成熟项目; 建议先发到**自有 tap**:

```bash
# 1) 建 tap 仓库 (如 github.com/<你的账号>/homebrew-tap)
# 2) 目录结构: Formula/qyzs.rb
cp packaging/homebrew/qyzs.rb <tap>/Formula/qyzs.rb
# 3) 用户即可: brew install <你的账号>/tap/qyzs
```

若希望进 `homebrew/core`, 需先满足其收录要求 (有稳定下载源、活跃维护等),
按其 PR 流程提交 `Formula/qyzs.rb`。

## 清单来源与核对

- **winget**: 便携 zip (`qyzs-x86_64-pc-windows-msvc.zip`, 内含 `qyzs.exe`),
  用 `InstallerType: zip` + `NestedInstallerType: portable` +
  `NestedInstallerFiles` 声明可执行文件与命令别名 `qyzs`
- **Homebrew**: 两个架构的 tar.gz, 包内为单文件 `qyzs`, 故用 `bin.install "qyzs"`
- **哈希**: 一律取自该 Release 的 `SHA256SUMS` (CI 自动生成, 已与线上资产逐一核对)
