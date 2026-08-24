#!/usr/bin/env bash
#
# compose-rebuild.sh — docker compose 構成 (HTTP 常駐サーバ) の停止→索引再構築→起動を一括実行
#
# 使い方:
#   ./compose-rebuild.sh                    # アーカイブ=.env の ARCHIVE_DIR (省略時 ./tests/data)
#   ./compose-rebuild.sh /path/to/archive   # アーカイブを上書き
#   SKIP_INDEX=1 ./compose-rebuild.sh       # 索引再構築をスキップ (既存索引で再起動だけ)
#
# 環境変数:
#   ARCHIVE_DIR   .env で設定済みなら不要。コマンド引数 > 環境変数 > .env の順で優先
#   SKIP_INDEX    1 を設定すると索引化を skip (compose down → up のみ)

set -euo pipefail

# プロジェクトルートに移動
cd "$(dirname "$0")"

# Git Bash (MSYS) なら docker のパス変換を抑制 (compose run の引数 /archive /index が壊れるのを防ぐ)
if [ -n "${MSYSTEM:-}" ] || [ -n "${MINGW_CHOST:-}" ]; then
    export MSYS_NO_PATHCONV=1
fi

# 1. 既存コンテナ停止 (named volume は保持される)
echo "→ docker compose down"
docker compose down

# 2. 引数があれば .env の ARCHIVE_DIR を一時的に上書き
COMPOSE_ARGS=()
if [ -n "${1:-}" ]; then
    export ARCHIVE_DIR="$1"
    echo "  ARCHIVE_DIR=${ARCHIVE_DIR} (CLI 引数で上書き)"
fi

# 3. 索引を named volume 内に再構築
if [ "${SKIP_INDEX:-0}" = "1" ]; then
    echo "→ index rebuild skipped (SKIP_INDEX=1)"
else
    echo "→ docker compose run --rm xdf-mcp xdf-index build /archive --out /index --fresh"
    docker compose run --rm xdf-mcp xdf-index build /archive --out /index --fresh
fi

# 4. サーバ起動
echo "→ docker compose up -d"
docker compose up -d

# 5. 起動ログを確認
sleep 2
echo
echo "=== xdf-mcp logs (last 5 lines) ==="
docker compose logs xdf-mcp --tail 5

echo
echo "✓ done"
echo "  HTTP: http://localhost:${HTTP_PORT:-8765}/mcp"
echo "  停止: docker compose down  (索引は保持)"
echo "  完全削除: docker compose down -v  (索引も削除、要注意)"
