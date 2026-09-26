#!/usr/bin/env bash
# nekos macOS 通用 dmg 打包：编译 → 打印 dmg 路径。只做这两件事。
# 不装 node 依赖、不做额外校验、不提供子命令；编译之后不做任何安装/部署动作。
#
# 用法:
#   ./pack-macos.sh
#
# 前置（脚本不代劳）:
#   · go、node/npm（前端依赖需已 `npm install` 到 node_modules）
#   · Rust（两个 std 目标缺了脚本会用 rustup 补装，见下）
#     Homebrew/MacPorts 版 rust 只能编译 host 目标，需换 rustup 工具链。私有安装
#     （不动全局环境；装好放 ~/.cache/nekos-universal-rust 时下面会自动启用，用完可删）:
#       SB=~/.cache/nekos-universal-rust
#       mkdir -p "$SB"
#       curl -sSf https://static.rust-lang.org/rustup/dist/x86_64-apple-darwin/rustup-init -o "$SB/rustup-init"
#       RUSTUP_HOME="$SB/rustup" CARGO_HOME="$SB/cargo" sh "$SB/rustup-init" -y \
#         --no-modify-path --profile minimal --default-toolchain 1.98.1 -t aarch64-apple-darwin
#   · Xcode 命令行工具（lipo / hdiutil）
#
# 产物（脚本结束打印绝对路径）:
#   src-tauri/target/universal-apple-darwin/release/bundle/dmg/*.dmg
#
# 说明:
#   · 三种 sidecar 文件名是 tauri 的硬性约定（`{externalBin 路径}-{target_triple}`，
#     见 tauri-utils::resources::external_binaries）：`tauri build --target universal-apple-darwin`
#     会分别以 `--target x86_64-apple-darwin` / `--target aarch64-apple-darwin` 编译两次，
#     每次 tauri-build 都按该 triple 名字校验 externalBin 存在，故两个单架构切片必须先就位；
#     打包阶段读取的是 lipo 合成的 `universal-apple-darwin` 那个通用文件。
#   · bundle.active 与 externalBin 只在打包命令里用 --config 注入，仓库默认 tauri.conf.json
#     保持不动（tauri-build 编译期无条件校验 externalBin 存在，tauri dev / build.sh release 不受影响）。
#   · `--bundles dmg` 出完 dmg 会把中间 .app 删掉（tauri 日志 "Cleaning …/nekos.app"）。

set -euo pipefail
cd "$(dirname "$0")"

GO_TAGS="with_utls,with_grpc"
BIN_DIR="src-tauri/binaries"
BUNDLE_DIR="src-tauri/target/universal-apple-darwin/release/bundle"

# 私有 Rust 沙箱（见「前置」）在本机存在就直接用，省得每次手写 PATH/RUSTUP_HOME/CARGO_HOME。
SB="${NEKOS_RUST_SANDBOX-$HOME/.cache/nekos-universal-rust}"
if [ -x "$SB/cargo/bin/rustup" ]; then
  export RUSTUP_HOME="$SB/rustup" CARGO_HOME="$SB/cargo" PATH="$SB/cargo/bin:$PATH"
fi

# 缺 std 目标就装（唯一允许的安装动作，且必须发生在编译之前）。
need=()
sysroot="$(rustc --print sysroot)"
for t in x86_64-apple-darwin aarch64-apple-darwin; do
  [ -d "$sysroot/lib/rustlib/$t" ] || need+=("$t")
done
if [ ${#need[@]} -gt 0 ]; then
  if command -v rustup >/dev/null 2>&1; then
    rustup target add "${need[@]}"
  else
    printf '缺少 std 目标: %s，且当前 rust 没有 rustup，请自行安装（见文件头「前置」）\n' "${need[*]}" >&2
    exit 1
  fi
fi

# Go 内核 sidecar：dmg 的必要输入（两个单架构切片 + lipo 合成通用）。
mkdir -p "$BIN_DIR"
for pair in "x86_64-apple-darwin:amd64" "aarch64-apple-darwin:arm64"; do
  triple="${pair%%:*}"; arch="${pair##*:}"
  (cd core && CGO_ENABLED=0 GOOS=darwin GOARCH="$arch" \
    go build -trimpath -ldflags "-s -w" -tags "$GO_TAGS" \
      -o "../${BIN_DIR}/nekos-core-${triple}" ./cmd/nekos-core)
done
lipo -create -output "${BIN_DIR}/nekos-core-universal-apple-darwin" \
  "${BIN_DIR}/nekos-core-x86_64-apple-darwin" "${BIN_DIR}/nekos-core-aarch64-apple-darwin"

# tauri 的 dmg bundler 默认会挂载临时可写镜像并跑 AppleScript 打开 Finder 窗口做图标排版
# （tauri-bundler 只在 CI=true 时加 --skip-jenkins）。这里固定 CI=true：不弹窗、不挂载卷，
# 代价只是 dmg 里不再写图标位置排版（Applications 拖拽链接与卷图标不受影响）。
CI=true npx tauri build --ci --target universal-apple-darwin --bundles dmg \
  --config '{"bundle":{"active":true,"externalBin":["binaries/nekos-core"]}}'

for p in "$PWD/${BUNDLE_DIR}"/dmg/*.dmg; do
  if [ -f "$p" ]; then printf 'dmg: %s\n' "$p"; fi
done
