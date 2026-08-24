#!/usr/bin/env bash
#
# rebuild.sh — xdf-fs イメージの再ビルド + 索引の全消し再構築
#
# 使い方:
#   ./rebuild.sh                       # アーカイブ=tests/data、索引=~/.xdf-fs/index
#   ./rebuild.sh /path/to/archive      # アーカイブを別ディレクトリに切替
#   SKIP_BUILD=1 ./rebuild.sh          # Docker イメージ再ビルドはスキップ (索引のみ)
#   INDEX_DIR=~/foo ./rebuild.sh       # 索引保存先を上書き
#
# 環境変数:
#   ARCHIVE_DIR   アーカイブディレクトリ (デフォルト: tests/data)
#   INDEX_DIR     索引ディレクトリ (デフォルト: ~/.xdf-fs/index)
#   IMAGE_TAG     Docker イメージのタグ (デフォルト: xdf-fs)
#   SKIP_BUILD    1 を設定すると docker build を skip
#   SKIP_INDEX    1 を設定すると索引化を skip (build のみ)

set -euo pipefail

# プロジェクトルートに移動
cd "$(dirname "$0")"

ARCHIVE_DIR="${1:-${ARCHIVE_DIR:-$(pwd)/tests/data}}"
INDEX_DIR="${INDEX_DIR:-$HOME/.xdf-fs/index}"
IMAGE_TAG="${IMAGE_TAG:-xdf-fs}"

# ---- 1. Docker イメージ再ビルド ----
if [ "${SKIP_BUILD:-0}" = "1" ]; then
    echo "→ docker build skipped (SKIP_BUILD=1)"
else
    echo "→ docker build -t ${IMAGE_TAG} . (1〜2分)"
    docker build -t "${IMAGE_TAG}" .
fi

# ---- 2. 索引ディレクトリ確保 ----
mkdir -p "${INDEX_DIR}"

# ---- 3. 索引の全消し再構築 ----
if [ "${SKIP_INDEX:-0}" = "1" ]; then
    echo "→ index rebuild skipped (SKIP_INDEX=1)"
    exit 0
fi

# Git Bash (MSYS) の場合、Docker 引数のパス変換を抑制
PATH_PREFIX=""
if [ -n "${MSYSTEM:-}" ] || [ -n "${MINGW_CHOST:-}" ]; then
    export MSYS_NO_PATHCONV=1
fi

echo "→ index rebuild (--fresh)"
echo "    archive: ${ARCHIVE_DIR}"
echo "    index:   ${INDEX_DIR}"
echo

docker run --rm \
    -v "${ARCHIVE_DIR}:/archive:ro" \
    -v "${INDEX_DIR}:/index" \
    "${IMAGE_TAG}" \
    xdf-index build /archive --out /index --fresh

echo
echo "✓ done"
echo
echo "次のステップ: Claude Desktop を再起動して新しい索引を反映"
