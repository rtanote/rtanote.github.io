# Claude Desktop で xdf-mcp を使う

xdf-mcp は MCP (Model Context Protocol) のサーバとして動作し、Claude Desktop / Claude Code から
自然言語で X68000 アーカイブを検索・参照できるようにします。

## 前提

1. Claude Desktop がインストール済み (macOS / Windows)
2. `xdf-mcp` バイナリがビルド済み (Docker または cargo)
3. tantivy 索引が作成済み (`xdf-index build` で)

> **著作権**: 索引対象は**自分が正当に取得したディスクイメージ**に限ること。
> 索引と AI 要約は原本の逐語テキストを含むため、公開・再配布しないこと。
> 詳細は [README の「著作権について」](../README.md#️-著作権について) を参照。

## 1. 索引を準備

```bash
# Docker の場合
docker run --rm \
  -v /path/to/archive:/archive:ro \
  -v $HOME/.xdf-fs/index:/index \
  xdf-fs xdf-index build /archive --out /index

# ネイティブビルドの場合
cargo run --release -p xdf-index --bin xdf-index -- \
  build /path/to/archive --out ~/.xdf-fs/index
```

`/path/to/archive` はXDF/HDS/HDFファイルを置いたディレクトリ。再帰的にスキャンされます。

## 2. Claude Desktop の設定

### 設定ファイルの場所

| OS | パス |
|---|---|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |

### ネイティブバイナリで起動 (推奨)

```jsonc
{
  "mcpServers": {
    "x68k-archive": {
      "command": "/usr/local/bin/xdf-mcp",
      "args": ["--index", "/Users/yourname/.xdf-fs/index"]
    }
  }
}
```

Windows パスの場合: `"C:\\Users\\yourname\\.xdf-fs\\index"` のようにバックスラッシュをエスケープ。

### Docker コンテナで起動 (stdio)

```jsonc
{
  "mcpServers": {
    "x68k-archive": {
      "command": "docker",
      "args": [
        "run", "--rm", "-i",
        "-v", "/Users/yourname/archive:/archive:ro",
        "-v", "/Users/yourname/.xdf-fs/index:/index",
        "xdf-fs",
        "xdf-mcp", "--index", "/index"
      ]
    }
  }
}
```

> Docker 経由は stdio が会話開始ごとにコンテナ起動するため初動が遅い。下記 HTTP モードを推奨。

### HTTP モード (常駐サーバ)

`docker compose up -d` で常駐させ、HTTP 経由で接続する方法。1度立ち上げれば再起動不要、複数クライアントから同時接続可能。

#### サーバ起動

```bash
# .env を編集
cp .env.example .env
# ANTHROPIC_API_KEY=... と ARCHIVE_DIR=... を設定 (ARCHIVE_DIR は省略時 ./tests/data)

# 索引が無ければ事前構築
docker compose run --rm xdf-mcp xdf-index build /archive --out /index --fresh

# サーバ起動
docker compose up -d

# 状態確認
docker compose logs xdf-mcp | tail -5
# → "xdf-mcp HTTP listening on http://0.0.0.0:8765/mcp"
```

#### Claude Desktop 設定 (HTTP)

```jsonc
{
  "mcpServers": {
    "x68k-archive": {
      "url": "http://localhost:8765/mcp"
    }
  }
}
```

> Claude Desktop の HTTP MCP サポートはバージョンによって対応状況が異なります。
> 接続できない場合は stdio 設定に戻すか、最新版の Claude Desktop を入手してください。

## 3. 動作確認

Claude Desktop を再起動 → 新しい会話を開始 → 設定アイコン (右上) で `x68k-archive` MCP サーバが
"Connected" になっていることを確認。

## 4. 提供される 7 ツール

### `archive_search` — 全文検索 (拡張子フィルタ対応)
```
input:  { query: string, limit?: number, ext?: string[] }
output: 上位ヒットの image_path:partition:file_path + 抜粋
```

例 (Claudeへのプロンプト):
> "X68000 アーカイブから、スプライトのスクロール処理をしている作例を探して"
> "ZMS ファイルだけ列挙して" (ext: ["ZMS"] で絞り込み)

### `archive_read` — ファイル内容取得 (LZH 内部対応)
```
input:  { image_path, partition?, file_path, max_bytes? }
output: UTF-8 (テキスト) または hex (バイナリ)
```

`file_path` に `outer.lzh!/inner.zms` 記法を含めれば LZH 内部メンバーも取得可能。

例:
> "上で見つけた xsprite.doc の中身を読んで、関数一覧を整理して"
> "PDD/PDD74.LZH の中の SHOKA1.ZMS を読んで"

### `archive_list` — ディレクトリ一覧
```
input:  { image_path, partition?, dir_path? }
output: ディレクトリエントリの一覧 (kind / size / mtime / attr)
```

例:
> "/archive/SCSIHDD1.HDS:0:/Z-MUSIC/SAMPLE/ の中身は?"

### `archive_summarize` — AI 要約取得 (キャッシュ参照)
```
input:  { image_path: string }
output: { summary, categories[], topics[], highlights[], usage }
```

要約は **事前生成** が必要 (CLI):
```bash
docker run --rm --env-file .env \
  -v <archive>:/archive:ro -v <index>:/index \
  xdf-fs xdf-index summarize /archive --index /index
```

例:
> "/archive/SCSIHDD1.HDS の概要を教えて"  
> → Claude が archive_summarize を呼び、キャッシュされた要約 (このディスクには Z-MUSIC ライブラリと Oh!X 付録が...) を返す

### `archive_view_image` — PIC 画像の表示
```
input:  { image_path, partition?, file_path, max_width? }
output: { format, width, height, bit_depth, mode, comment, source } + PNG inline
```

X68000 PIC 形式の画像を PNG に変換。4/8 bit パレット + 15/16 bit RGB 全モード対応 (png2pic 経由)。
LZH 内部の PIC (`outer.lzh!/inner.pic`) もOK。

> **表示方法**: Claude Desktop は MCP の image content を **デフォルトで折りたたみ UI に格納**する。
> 表示するには ツール結果の **「Archive view image ⌄」行をクリックして展開**する必要がある。
> markdown データURL も `file://` リンクも Claude Desktop が意図的に描画しない仕様のため、
> 「展開して見る」が現状最も手早い操作 (1クリック)。

例:
> "Dennou074A.img の TTL1.PIC を見せて"  
> → Claude が archive_view_image を呼ぶ → ユーザは結果行を1クリックで展開 → 「電脳倶楽部」タイトルロゴ表示

> "1992年あたりの PIC を 5 枚ランダムに見せて、当時のグラフィック表現の特徴を教えて"  
> → search で .PIC 列挙 → 各を view_image で取得 → 各結果を順次展開して見比べ

### `archive_metadata_query` — 構造化メタデータクエリ

```
input:  { category?: string, issue_no?: number, issue_no_min?: number,
          issue_no_max?: number, submitter_contains?: string,
          title_contains?: string, limit?: number, format?: "json" | "csv" }
output: 条件に一致するレコードの一覧 (payload 付き)
```

全文検索と違い**確定的なリスト**が返るため、「何号に誰が投稿したか」のような
列挙に向く。事前に `xdf-index extract` で構造化抽出を済ませておく必要がある。

例:
> "50号から100号までの音楽投稿を投稿者ごとに一覧にして"
> "この投稿者の曲を全部挙げて"

### `archive_extract` — 複数ファイルを ZIP でまとめて取得

```
input:  { image_path: string, paths: string[], partition?: number, flatten?: boolean }
output: { output_path, download_url, file_count, total_bytes }
```

glob (`*.X`, `IKAP/**`) に対応。バイト無加工、100 MB 上限。
`XDF_MCP_PUBLIC_URL` を設定していれば HTTPS の `download_url` が返る
(未設定ならコンテナ内の `output_path` のみ)。

> ⚠️ 取り出した内容は著作物である。再配布しないこと。

例:
> "このディスクの MUSIC ディレクトリを丸ごと ZIP にして"

## 5. 想定ユースケース

- **温故知新ナレッジベース**: X68000時代の作例をヒントに、Logic Pro / Unity / Unreal Engine で
  現代版を実装する際の参考にする
- **Z-MUSIC 楽曲調査**: 「`CMD` 命令を使っている曲」「FM音源音色データ」を一括検索
- **X-BASIC ゲーム作例**: 「スプライト管理」「衝突判定」のサンプルコードを取得
- **Oh!X 付録ディスク横断**: 月号別にアーカイブされた付録ディスクから記事関連コードを横断参照

## 6. トラブルシュート

### MCP サーバが "Connected" にならない

- Claude Desktop のログを確認:
  - macOS: `~/Library/Logs/Claude/`
  - Windows: `%APPDATA%\Claude\logs\`
- `xdf-mcp --index <dir>` を手動で起動して動くか確認 (stdio で待機状態になる)
- 索引ディレクトリのパスがアクセス可能か (`xdf-index status --index <dir>` で確認)

### 検索結果が空

- `xdf-index status --index <dir>` で documents 数を確認 (0 ならビルド失敗)
- クエリを単純な単語にする (例: `Z-MUSIC` ではなく `Z` か `MUSIC` 単体)
- `xdf-index search "..." --index <dir>` で CLI から検索できるか確認

### 文字化け

- 索引化時に SJIS → UTF-8 変換済みなので、Claude が表示する文字は UTF-8。
- Claude Desktop の表示フォントが日本語対応か確認。

## 7. 今後の予定機能

- ⏳ `archive_find_examples` — 自然言語トピック → 関連する**作例コード**の候補を返す
  (`archive_search` との差別化は「実行可能なサンプルに絞る」点)
- ⏳ HDS のディレクトリ粒度要約 (HDS 1個=1サマリでは粗い場合に追加)
- ⏳ 電脳倶楽部以外のスキーマプラグイン (Oh!X 等)
