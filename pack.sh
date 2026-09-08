#!/usr/bin/env bash
# nekos 跨平台打包：Go 内核 sidecar + 前端 + tauri bundle（deb/rpm/AppImage | dmg | nsis）。
#
# 用法:
#   ./pack.sh               为本机 OS 打包（先编本平台的 core sidecar，再 tauri bundle）
#   ./pack.sh core          为常见目标平台交叉编译 core sidecar（不打包）:
#                              linux amd64/arm64, darwin amd64/arm64, windows amd64
#
# 环境变量:
#   NEKOS_ARCH         覆盖 core 的 GOARCH（amd64|arm64）；默认取当前机器
#   NEKOS_BUNDLES      覆盖 tauri 打包类型（如 "deb rpm" / "nsis" / "dmg app"）；默认按 OS 取
#   NEKOS_CONFIG_EXTRA 追加合并进 tauri 配置的 JSON（如 '{"bundle":{"category":"Network"}}'）
#
# 产物位置:
#   core sidecar → src-tauri/binaries/nekos-core-<rust-triple>[.exe]（tauri externalBin 命名约定）
#   bundle       → src-tauri/target/release/bundle/{deb,rpm,appimage,nsis,dmg}
#
# 说明:
#   - tauri.conf.json 中 bundle.active 保持 false；脚本在打包时用 `tauri build --config`
#     临时开启，仓库默认配置不动，`tauri dev` / `build.sh release` 不受影响。
#   - bundle 只能在对应 OS 上生成（linux→deb/rpm/AppImage，macOS→dmg，Windows→nsis）；
#     脚本按当前 OS 选默认列表。core sidecar 可在这台机器上用 `./pack.sh core` 交叉预编译。
#   - 需要 go、node/npm、rust 及目标平台依赖。

set -euo pipefail
cd "$(dirname "$0")"

GO_TAGS="with_utls,with_grpc"

log() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m!!\033[0m %s\n' "$*" >&2; exit 1; }

host_os() {
  case "$(uname -s)" in
    Linux)  echo linux ;;
    Darwin) echo darwin ;;
    MINGW*|MSYS*|CYGWIN*) echo windows ;;
    *) die "不支持的系统: $(uname -s)" ;;
  esac
}

host_arch() {
  case "$(uname -m)" in
    x86_64|amd64) echo amd64 ;;
    aarch64|arm64) echo arm64 ;;
    *) die "不支持的架构: $(uname -m)" ;;
  esac
}

# Go (GOOS, GOARCH) → tauri/rust 目标三元组（externalBin 文件名用）。
triple_of() {
  local os="$1" arch="$2"
  case "$os/$arch" in
    linux/amd64)   echo x86_64-unknown-linux-gnu ;;
    linux/arm64)   echo aarch64-unknown-linux-gnu ;;
    darwin/amd64)  echo x86_64-apple-darwin ;;
    darwin/arm64)  echo aarch64-apple-darwin ;;
    windows/amd64) echo x86_64-pc-windows-msvc ;;
    windows/arm64) echo aarch64-pc-windows-msvc ;;
    *) die "未知 os/arch: $os/$arch" ;;
  esac
}

# 打包时 sidecar 必须匹配 cargo 的目标三元组（默认 = rustc host）。
rust_host_triple() {
  local t
  t="$(rustc -vV 2>/dev/null | sed -n 's/^host: //p')"
  if [ -n "$t" ]; then echo "$t"; else triple_of "$(host_os)" "$(host_arch)"; fi
}

# 编译单个 core sidecar → src-tauri/binaries/nekos-core-<triple>[.exe]
build_core_sidecar() {
  local os="$1" arch="$2"
  local triple exe
  triple="$(triple_of "$os" "$arch")"
  exe="src-tauri/binaries/nekos-core-${triple}"
  [ "$os" = windows ] && exe="${exe}.exe"

  mkdir -p src-tauri/binaries
  log "core sidecar: GOOS=$os GOARCH=$arch → ${exe}"
  (cd core && CGO_ENABLED=0 GOOS="$os" GOARCH="$arch" \
    go build -trimpath -ldflags "-s -w" -tags "$GO_TAGS" \
    -o "../$exe" ./cmd/nekos-core)
}

build_frontend() {
  [ -d node_modules ] || { log "npm ci"; npm ci; }
}

default_bundles() {
  case "$(host_os)" in
    linux)   echo "deb rpm appimage" ;;
    darwin)  echo "app dmg" ;;
    windows) echo "nsis" ;;
  esac
}

cmd="${1:-pack}"
if [ "$cmd" = core ]; then
  for target in linux/amd64 linux/arm64 darwin/amd64 darwin/arm64 windows/amd64; do
    build_core_sidecar "${target%/*}" "${target#*/}"
  done
  log "core sidecars 就绪:"
  ls -1 src-tauri/binaries/ | sed 's/^/  /'
  exit 0
fi
[ "$cmd" != pack ] && die "未知子命令: $cmd（可用: pack | core）"

OS="$(host_os)"
ARCH="${NEKOS_ARCH:-$(host_arch)}"
TRIPLE="$(rust_host_triple)"

build_core_sidecar "$OS" "$ARCH"
build_frontend

if [ -n "${NEKOS_BUNDLES:-}" ]; then
  read -r -a bundles <<<"$NEKOS_BUNDLES"
else
  read -r -a bundles <<<"$(default_bundles)"
fi

log "tauri bundle (os=$OS, triple=$TRIPLE, bundles=${bundles[*]})"
args=(--config '{"bundle":{"active":true}}')
[ -n "${NEKOS_CONFIG_EXTRA:-}" ] && args+=(--config "$NEKOS_CONFIG_EXTRA")
args+=(--bundles "${bundles[@]}")

npx tauri build --ci "${args[@]}"

log "完成。产物:"
find src-tauri/target/release/bundle -maxdepth 2 -type f 2>/dev/null | sed 's/^/  /'
