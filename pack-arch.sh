#!/usr/bin/env bash
# nekos → Arch Linux pacman 包 (.pkg.tar.zst)
#
# 流程: 复用 pack.sh 构建 deb → 临时目录生成 PKGBUILD → makepkg 重新封装为 Arch 包。
# Tauri 不直接产出 pacman 包;deb 已含完整安装布局(主程序 + nekos-core sidecar +
# desktop/图标),makepkg 只需把它解包重排,无需二次编译。
#
# 用法:
#   ./pack-arch.sh             构建 deb 并生成 Arch 包
#   ./pack-arch.sh -i          生成并安装 (pacman -U, 需要 sudo)
#   ./pack-arch.sh --no-deb    跳过 deb 构建,直接封装已有 deb
#
# 环境变量:
#   NEKOS_PKGREL   打包版本号 (默认 1)
#
# 产物:
#   src-tauri/target/release/bundle/arch/nekos-<ver>-<pkgrel>-<arch>.pkg.tar.zst
#
# 前置: makepkg (pacman)、libarchive(bsdtar, base 自带);打包 deb 还需
# pack.sh 的全部工具链 (go/node/rust + webkit2gtk-4.1 等)。makepkg 拒绝 root 运行。

set -euo pipefail
cd "$(dirname "$0")"

log() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m!!\033[0m %s\n' "$*" >&2; exit 1; }

# ---------- 参数 ----------
INSTALL=0
NO_DEB=0
for arg in "$@"; do
  case "$arg" in
    -i|--install) INSTALL=1 ;;
    --no-deb)     NO_DEB=1 ;;
    *) die "未知参数: $arg（可用: -i/--install | --no-deb）" ;;
  esac
done

# ---------- 环境检查 ----------
command -v makepkg >/dev/null 2>&1 || die "缺少 makepkg,请安装 pacman 工具链"
[ "$(id -u)" -eq 0 ] && die "makepkg 不能以 root 运行,请改用普通用户执行"
command -v bsdtar >/dev/null 2>&1 || die "缺少 bsdtar,请安装 libarchive"
[ "$NO_DEB" -eq 0 ] && {
  command -v go >/dev/null 2>&1 || die "缺少 go"
  command -v node >/dev/null 2>&1 || die "缺少 node/npm"
  command -v rustc >/dev/null 2>&1 || die "缺少 rustc/cargo"
}

# ---------- 读 tauri 配置 ----------
PRODUCT="$(sed -nE 's/^[[:space:]]*"productName"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p' src-tauri/tauri.conf.json | head -n1)"
VERSION="$(sed -nE 's/^[[:space:]]*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p' src-tauri/tauri.conf.json | head -n1)"
[ -n "$PRODUCT" ] && [ -n "$VERSION" ] || die "无法从 src-tauri/tauri.conf.json 读取 productName/version"

case "$(uname -m)" in
  x86_64|amd64)  ARCH=x86_64; DEBARCH=amd64 ;;
  aarch64|arm64) ARCH=aarch64; DEBARCH=arm64 ;;
  *) die "不支持的架构: $(uname -m)" ;;
esac
PKGREL="${NEKOS_PKGREL:-1}"

DEB="src-tauri/target/release/bundle/deb/${PRODUCT}_${VERSION}_${DEBARCH}.deb"

# ---------- 1. 构建 deb (pack.sh 只出 deb) ----------
if [ "$NO_DEB" -eq 1 ]; then
  [ -f "$DEB" ] || die "--no-deb 但找不到已有产物: $DEB"
  log "复用已有 deb: $DEB"
else
  log "NEKOS_BUNDLES=deb ./pack.sh"
  NEKOS_BUNDLES=deb ./pack.sh
fi
[ -f "$DEB" ] || die "deb 未生成: $DEB"

# ---------- 2. 临时目录: PKGBUILD + deb → makepkg ----------
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cp "$DEB" "$TMP/"
cat >"$TMP/PKGBUILD" <<'PKGBUILD'
# Maintainer: nekos developers <https://github.com/gvcgo/nekos>
pkgname=nekos
pkgver=0.1.0
pkgrel=1
pkgdesc="nekos sing-box GUI client"
arch=('x86_64')
url="https://github.com/gvcgo/nekos"
license=('GPL-3.0-or-later')
depends=('webkit2gtk-4.1' 'gtk3' 'libayatana-appindicator' 'librsvg')
source=("nekos_0.1.0_amd64.deb")
sha256sums=('SKIP')

package() {
  cd "$srcdir"
  # .deb 是 ar 容器;bsdtar 先解外层,再解 data.tar.* 到安装根
  bsdtar -xf nekos_0.1.0_amd64.deb
  local data
  data="$(find . -maxdepth 1 -name 'data.tar.*' -print -quit)"
  bsdtar -xf "$data" -C "$pkgdir"
}
PKGBUILD
# 占位符写死后按实际配置回填,避免 heredoc 里做插值
sed -i \
  -e "s/^pkgver=.*/pkgver=${VERSION}/" \
  -e "s/^pkgrel=.*/pkgrel=${PKGREL}/" \
  -e "s/^arch=.*/arch=('${ARCH}')/" \
  -e "s/nekos_0\.1\.0_amd64\.deb/nekos_${VERSION}_${DEBARCH}.deb/g" \
  "$TMP/PKGBUILD"
grep -E "^(pkgname|pkgver|pkgrel|arch|source)" "$TMP/PKGBUILD" | sed 's/^/  /'

# ---------- 3. makepkg ----------
log "makepkg (${TMP})"
( cd "$TMP" && makepkg -f )

PKG="$TMP/${PRODUCT}-${VERSION}-${PKGREL}-${ARCH}.pkg.tar.zst"
[ -f "$PKG" ] || die "makepkg 产物缺失: $PKG"

# ---------- 4. 归位 / 安装 ----------
OUT_DIR="src-tauri/target/release/bundle/arch"
mkdir -p "$OUT_DIR"
mv "$PKG" "$OUT_DIR/"
PKG="$OUT_DIR/${PRODUCT}-${VERSION}-${PKGREL}-${ARCH}.pkg.tar.zst"
log "产物: $PKG"

if [ "$INSTALL" -eq 1 ]; then
  log "sudo pacman -U $PKG"
  sudo pacman -U "$PKG"
fi

log "完成。卸载: pacman -R ${PRODUCT}"
