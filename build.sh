#!/usr/bin/env bash
# nekos 一键构建 + 调试。
#
# 用法:
#   ./build.sh          构建 Go 内核与前端依赖，然后进入 tauri dev（调试，Ctrl+C 退出）
#   ./build.sh release  构建 release 二进制（src-tauri/target/release/nekos）
#   ./build.sh core     仅重建 Go 内核（core/bin/nekos-core）
#
# 内核构建必须带 sing-box 标签:
#   - with_utls  缺省编译下 uTLS 是 stub，任何带 tls.utls 的节点在初始化出站时报
#                "uTLS is not included in this build, rebuild with -tags with_utls"
#   - with_grpc  同理，grpc 传输层在 !with_grpc 下退化为 lite stub
set -euo pipefail
cd "$(dirname "$0")"

GO_TAGS="with_utls,with_grpc"
CORE_BIN="core/bin/nekos-core"

build_core() {
  echo "==> go build -tags ${GO_TAGS} -o ${CORE_BIN} ./cmd/nekos-core"
  (cd core && go build -tags "${GO_TAGS}" -o bin/nekos-core ./cmd/nekos-core)
  echo "==> 内核就绪: ${CORE_BIN}"
}

need_node() {
  command -v node >/dev/null 2>&1 || { echo "缺少 node/npm，请先安装" >&2; exit 1; }
}

case "${1:-dev}" in
  dev)
    need_node
    build_core
    if [ ! -d node_modules ]; then
      echo "==> npm install"
      npm install
    fi
    echo "==> 启动 tauri dev（调试）…"
    exec npm run tauri dev
    ;;
  release)
    need_node
    build_core
    if [ ! -d node_modules ]; then
      echo "==> npm install"
      npm install
    fi
    echo "==> tauri release 构建（前端由 beforeBuildCommand 一并处理）…"
    exec npm run tauri build
    ;;
  core)
    build_core
    ;;
  *)
    echo "用法: ./build.sh [dev|release|core]（默认 dev）" >&2
    exit 2
    ;;
esac
