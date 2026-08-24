# x68k-archive container image
#   builder : workspace 全体をビルド
#   runtime : CLI 3種 (xdfls / xdfcp / xdfgrep) + xdf-index + xdf-mcp を同梱

# ---- builder ----
FROM rust:1.98-slim AS builder

WORKDIR /src

# workspace 全体を投入 → 全クレートのバイナリ + examples をビルド
# (依存キャッシュ最適化は cargo-chef 等で後日実装。まずは正確性優先)
COPY . .
RUN cargo build --release --workspace --bins --examples

# ---- runtime ----
FROM debian:bookworm-slim AS runtime

# CA証明書 (xdf-mcp が Anthropic API を叩く際に必要)
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# ファイル操作 CLI
COPY --from=builder /src/target/release/examples/xdfls   /usr/local/bin/xdfls
COPY --from=builder /src/target/release/examples/xdfcp   /usr/local/bin/xdfcp
COPY --from=builder /src/target/release/examples/xdfgrep /usr/local/bin/xdfgrep
# 全文索引 / AI 要約 / 構造化抽出
COPY --from=builder /src/target/release/xdf-index        /usr/local/bin/xdf-index
# MCP サーバ
COPY --from=builder /src/target/release/xdf-mcp          /usr/local/bin/xdf-mcp

# アーカイブを bind mount するデフォルトの場所
WORKDIR /archive

# デフォルトは xdfls --help (使い方が分かるように)
CMD ["xdfls", "--help"]
