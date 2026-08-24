# x68k-archive Skill

X68000 アーカイブナレッジベース (xdf-fs / xdf-mcp) を Claude が自然に使うためのスキル定義。

## 何をするスキルか

- ユーザが X68000 / Z-MUSIC / X-BASIC / スプライト等の話題を出したとき、自動的にこのスキルが選ばれる
- 配下にある [`SKILL.md`](SKILL.md) が Claude のコンテキストに投入され、`x68k-archive` MCP サーバの 7 ツール (search / read / list / summarize / view_image / metadata_query / extract) を適切に使う作法が伝わる
- **温故知新の橋渡し** (X68000 → Logic Pro / Unity / Unreal) のマッピングをスキル内に持つ

## 前提条件

1. **MCP サーバ稼働中**: `x68k-archive` が Claude Desktop か Claude Code に接続済み (緑バッジ)
   → 未設定なら [docs/claude-desktop-setup.md](../../docs/claude-desktop-setup.md) を先に
2. **索引構築済み**: `xdf-index build` が走り済み
3. **(任意) 要約キャッシュ生成済み**: `xdf-index summarize` — `archive_summarize` ツールを使う場合

## インストール

スキルは **Claude Code** と **Claude Desktop** の両方で利用可能。配置場所は環境ごとに異なる。

### Claude Code (VS Code 拡張・CLI)

ユーザレベルにインストール (全プロジェクトで使える):

```bash
mkdir -p ~/.claude/skills
cp -r skills/x68k-archive ~/.claude/skills/
```

または、このプロジェクト内でのみ有効にする (プロジェクトレベル):

```bash
mkdir -p .claude/skills
cp -r skills/x68k-archive .claude/skills/
```

PowerShell (Windows) の場合:

```powershell
New-Item -ItemType Directory -Path "$env:USERPROFILE\.claude\skills" -Force
Copy-Item -Path "skills\x68k-archive" -Destination "$env:USERPROFILE\.claude\skills\" -Recurse
```

### Claude Desktop (GUI アプリ)

Claude Desktop の Skill サポートは 2026 年現在発展中。最新状況は Anthropic 公式
[Skill ドキュメント](https://docs.anthropic.com/en/docs/build-with-claude/skills) を参照。

現状の暫定配置 (将来変わる可能性あり):

- macOS: `~/Library/Application Support/Claude/skills/x68k-archive/`
- Windows: `%LOCALAPPDATA%\Packages\Claude_<HASH>\LocalCache\Roaming\Claude\skills\x68k-archive\`

(MSIX 形式の Claude Desktop では `LocalCache\Roaming\Claude\` 配下が `%APPDATA%\Claude\` の実体)

## 動作確認

Claude Code か Claude Desktop で新しい会話を開き、以下のような質問を投げると、Claude がスキルをロードして MCP ツールを自然に呼ぶはず:

> 「X68000 アーカイブから、スプライトの接触判定をしている作例を3件探して、Unity の Physics2D で書くとどうなるか比較して」

期待される動作:
1. Claude が `archive_search({query: "接触判定 sprite", limit: 5})` を呼ぶ (or 同様のクエリ)
2. ヒット上位 3 件を `archive_read` で取得
3. **温故知新マッピング** (XSPRITE.FNC vs Physics2D) を提示
4. すべての引用に出典 `image:partition:file_path` 付き

期待動作にならない場合 (ツール未呼出 / 出典なし) は、SKILL.md の「適用すべきユーザー意図」「出典 (必須)」セクションを強化する。

## 更新

`SKILL.md` を編集 → Claude Code/Desktop を **完全再起動** (バックグラウンドプロセスも終了) で反映。

スキル定義はテキストなので、ナレッジベースの実利用フィードバック ("ここがイマイチ" "もっとこう答えてほしい") を直接書き込んで改善できる。

## トラブルシュート

### Claude がスキルを呼ばない / MCP ツールを使ってくれない
- フロントマター `description` に**ユーザの実際の問いかけに含まれそうなキーワード**を追加
  (今は `「X68000」「Z-MUSIC」` 等を列挙済みだが、漏れがあれば追加)
- スキルが「自動選択」されない場合、ユーザに **明示的に呼ぶ** ように依頼:
  > 「`x68k-archive` スキルを使って、〇〇を調べて」

### 出典が抜ける
SKILL.md の「出典 (必須)」セクションを最上位に移動するか、**禁忌セクション**に「出典なしで X68000 由来の知識を提示しない」と強調。

### 温故知新マッピングが不正確 / 増やしたい
SKILL.md の「6. 温故知新の橋渡し」表を編集。User の実利用 (Logic Pro / Unity / Unreal) に近い具体例ほど効く。
