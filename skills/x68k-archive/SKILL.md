---
name: x68k-archive
description: |
  Use this skill when the user asks about X68000 (Sharp's 1987 Japanese personal computer) era software,
  music, games, sprites, or retro programming techniques. The skill bridges X68000 archives to modern
  platforms (Logic Pro, Unity, Unreal Engine, modern audio/graphics). Triggers include: 「X68000」「Z-MUSIC」
  「X-BASIC」「FM音源」「電脳倶楽部」「Oh!X」「スプライト」「温故知新」「retro coding」 / "X68k", "Sharp X68000",
  "MML", "FM synth chip", "sprite hardware", "old Japanese computer". Calls into the x68k-archive MCP
  server to search/read indexed disk images.
---

# x68k-archive Skill

X68000 時代のディスクイメージ群を温故知新ナレッジベースとして活用するための作法。
背後で `x68k-archive` MCP サーバが動作しており、7 ツールが使える状態を前提とする。

## 1. 利用可能なツール (前提)

| ツール | 用途 |
|---|---|
| `archive_search(query, limit?, ext?)` | 全文索引クエリ。`ext: ["ZMS","DOC"]` で拡張子絞り込み可 |
| `archive_read(image_path, partition?, file_path, max_bytes?)` | ファイル取得 (UTF-8 / hex)。LZH内部は `outer.lzh!/inner.zms` 記法 |
| `archive_list(image_path, partition?, dir_path?)` | ディレクトリ一覧 |
| `archive_summarize(image_path)` | イメージ単位のキャッシュ要約 (categories / topics / highlights / summary) |
| `archive_metadata_query(category?, issue_no?, submitter_contains?, ...)` | 構造化メタデータへの確定的クエリ。「何号の誰の投稿か」のような列挙に使う (全文検索より正確) |
| `archive_view_image(image_path, partition?, file_path, max_width?)` | **X68000 PIC 画像を PNG に変換してインライン表示**。LZH内部もOK、`max_width` で縮小可 |
| `archive_extract(image_path, paths[], partition?, flatten?)` | **複数ファイル/ディレクトリを ZIP にまとめて出力**。glob (`*.X` `IKAP/**`) 対応、バイト無加工、100 MB 上限。返値の `download_url` (HTTPS) をユーザに提示するか、`output_path` を後段の bash で利用 |

接続が無い場合 (例: 設定未完了) は、その旨をユーザに知らせて [docs/claude-desktop-setup.md](https://github.com/rtanote/x68k-archive/blob/main/docs/claude-desktop-setup.md) を参照するよう案内する。

## 2. 適用すべきユーザー意図

このスキルが効くのは以下のような問いかけ:

- **トピック調査**: 「X68000 でスプライトを使った作例ある?」「Z-MUSIC の音色データ集めて」
- **個別文書の参照**: 「`xsprite.doc` の関数一覧見せて」「Z-MUSIC のサンプル `SAMPLE.DOC` 読んで」
- **アーカイブ全体の把握**: 「このディスクの中身教えて」「Oh!X 1994年10月号には何が入ってる?」
- **現代への橋渡し** (本スキルの真価): 「X68000 の手法を Unity の C# で再現するとどうなる?」「Z-MUSIC の音色を Logic Pro の Operator で組むには?」

## 3. 標準ワークフロー

### パターン A: トピック検索 → 上位ヒット要約

```
1. archive_search({query: "<topic>", limit: 10})
2. ヒット上位 3〜5 件を archive_read で取得 (max_bytes: 4096-16384)
3. 各ファイルから関連箇所を抽出して、出典付きで要約
```

例 (`スプライト` 検索):
```
[1] /archive/SCSIHDD1.HDS:0:OhX_appendix_DISK/19941001MOMIJI/MOMIJI1/xbasfnc/xsprite/xsprite.doc
    XSPRITE.FNC は X-BASIC でハードウェアスプライトを操作する関数群を提供。
    主要関数: sp_xinit() (初期化)、sp_loc(s,x,y) (位置設定)、
    sp_hit(s) (接触判定)、sp_slidev(s,vx,vy,t) (ベクトル移動) ...
```

### パターン B: イメージ全体把握

```
1. archive_summarize({image_path: "/archive/..."})
2. summary / categories / topics / highlights をユーザに提示
3. 興味があれば archive_search でディスク内 (image_id 絞り込み) を深掘り
```

要約が無い (キャッシュ未生成) と返ってきた場合は、
「`xdf-index summarize` CLI で先にバッチ生成が必要」とユーザに案内する。

### パターン C: 拡張子フィルタでの収集

```
特定形式だけ集めたいケース (MML/ソース/ドキュメント横断):
1. archive_search({query: "", ext: ["ZMS","MML"], limit: 50})
2. ファイル一覧をテーマ別にグルーピングして提示
```

### パターン D: ディレクトリ探索

```
ユーザが構造把握したい場合:
1. archive_list({image_path: "...", dir_path: "/Z-MUSIC"}) でディレクトリ内を見る
2. 興味のあるサブディレクトリを再帰的に展開
```

### パターン E: 大量ファイルの一括取得 (`archive_extract` 推奨)

ユーザが「`/IKAP` 配下を全部欲しい」「拡張子 .X の実行ファイルを全部 ZIP で」のように
**多数のファイルを取り出したい** と頼んだら、`archive_read` を繰り返さずに必ず
`archive_extract` を使う (token 効率が圧倒的に良い、バイト無加工で X68000 emu 互換)。

```
1. archive_extract({
     image_path: "...",
     paths: ["IKAP/", "*.X", "BIN/**/*.S"],   // ファイル / ディレクトリ / glob 混在可
     flatten: false,                            // 階層保持 (デフォルト)
   })
2. 返値の download_url (HTTPS、Tailscale Funnel 経由) をユーザに提示
   または output_path (コンテナ内パス) を後段の bash で扱う
3. files[] のリストから実際に取れたファイルを要約してユーザに見せる
```

**重要**:
- 100 MB 上限あり。超過時はエラーが返るので、`paths` を絞るかディレクトリを分割
- ZIP 内のバイトは **無加工** (SJIS ファイル名そのまま、バイナリ無変換)。X68000 実機 / エミュレータでそのまま使える
- LZH 内部メンバ (`outer.lzh!/inner.x`) は対象外。個別取得は `archive_read` を使う

## 4. 出典 (必須)

**すべての検索結果と引用には出典を付ける**:

```
形式: <image_path>:<partition>:<file_path>
LZH内部: <image_path>:<partition>:<outer.lzh>!/<inner_path>
```

例:
- `/archive/Dennou074A.img:0:/MUSIC/STRANGE.ZMS` (XDF 内のファイル)
- `/archive/SCSIHDD1.HDS:0:Z-MUSIC/SAMPLE/SAMPLE.DOC` (HDS パーティション 0)
- `/archive/Dennou074A.img:0:PDD/PDD74.LZH!/SHOKA1.ZMS` (LZH 内部)

出典なしで X68000 由来の知識を提示するのは禁止。生成情報と検索結果の境界を明確に。

## 5. 文字コード・表示

- **`archive_read` は SJIS から UTF-8 に変換済み**で text を返す。バイナリ判定済みなら encoding=`binary` で hex
- 全角SJIS / 半角カナ / X68000 独自外字 (~0x9F) はベストエフォートで読み取り済み
- 出力時は文字化けせずに表示 (Markdown コードブロック内も OK)

## 6. 温故知新の橋渡し (このスキルの目玉)

ユーザが「現代でこれを再現したい」と言ったら、X68000 の制約と現代環境の対応を**明示的に対比**する。

### 音楽: Z-MUSIC → Logic Pro

| X68000 (Z-MUSIC) | Logic Pro 等価 |
|---|---|
| `(@n, AR,DR,SR,RR,SL,TL,KS,ML,DT,AMS,FB)` FM音色定義 | Operator (Logic) / FM8 (NI) のオペレータ階層 |
| `(t<n>)` テンポ | Tempo オートメーション |
| `(o<n>)` オクターブ + `(l<n>)` 長さ | Piano Roll / Step Sequencer |
| `(@i<n>)` MIDI チャンネル切替 | Multi-Output VST + MIDI Track 分離 |
| ADPCM (.PCM) | Sampler / Quick Sampler |

X68000 の制約 (8 FM ch + ADPCM 1 ch = 9 同時音) を強調しつつ、現代では実質無制限であることを利用してアレンジ余地を提案。

### ゲーム: X-BASIC スプライト → Unity 2D

| X68000 (XSPRITE.FNC) | Unity 2D 等価 |
|---|---|
| `sp_loc(s, x, y)` 位置設定 | `transform.position = new Vector3(x, y, 0)` |
| `sp_hit(s)` 接触判定 | `Collider2D.OverlapPoint` / `Physics2D.OverlapBox` |
| `sp_slidev(s, vx, vy, t)` ベクトル移動 | `Rigidbody2D.linearVelocity` または `Animator` |
| `sp_xinit()` 初期化 | `MonoBehaviour.Start()` |
| 32x32px ハードウェアスプライト | `SpriteRenderer` (任意サイズ) |
| 接触判定グループ (`sp_hgadd`) | `LayerMask` + Physics2D |

現代版は「画面合成は GPU が無制限」であることを伝え、スプライト数の制約 (X68000 は 128 個まで) からの解放をアピール。

### システム: m68k アセンブリ → 現代 CPU / WebAssembly

m68k アセンブリの読解は教育的価値あり (シンプルな ISA、レジスタ豊富)。
現代対応物として:
- 学習目的: ARM64 アセンブリ、WebAssembly Text format
- 思想の継承: SDL2 / SFML での低レベルゲーム開発、wgpu / WebGPU でハードウェア寄りの操作

### グラフィック: PIC → モダン画像形式

`.PIC` (X68000 native image format) は **`archive_view_image` ツールで PNG にデコード**できる:

```
archive_view_image({image_path: "/archive/Dennou074A.img", file_path: "TTL1.PIC", max_width: 512})
→ Content::text (metadata) + Content::image (PNG inline)
```

**画像をユーザに見せる手順**:

1. `archive_view_image` を呼ぶ (大きい画像なら `max_width: 512` を指定)
2. 自分の応答に **「ツール結果の "Archive view image ⌄" の行をクリックして展開すると画像が見られます」と明示**する
3. metadata から読み取れる情報 (画像名・色数・年代等) を1〜3行のキャプションで添える

> **Claude Desktop の UI 仕様**: tool result はデフォルトで折りたたまれ、画像も collapsed UI 内に隠れる。
> ユーザに展開を促す文言を入れないと、見落とされる。
> 1クリックの展開操作が現状で最も手早い表示方法 (markdown データURL も file URL も Claude Desktop が描画しない)。

その上で当時のグラフィック表現を解説する流れ:
- 色数 (15bit = 32K色) や解像度 (768x512 が標準) の意義
- グラデーション・ディザ等の表現技法
- 現代の PNG/WebP/AVIF と比較した制約と工夫

**サイズが大きい PIC** (768x512 等) は `max_width: 512` を指定推奨。
Content::image のトークン消費を抑えられる。

**まだ未対応**:
- `.MAG` (Maki-chan-Graphic) 形式 — 同様にインライン表示できるよう対応予定だが、現状は `archive_read` でバイナリ hex として返るだけ
- `.PI` (PI-DOS 用形式) — 同上

## 7. 応答フォーマット (推奨)

```markdown
## 検索結果

### [スコア順1位] <ファイル名>
**出典**: `image:partition:file_path`
**サイズ**: NNNN bytes / **更新日**: YYYY-MM-DD
**抜粋**:
> (本文の関連箇所、3〜5行)

**内容**: (Claudeによる要約)

---

### [次のヒット]
...

## まとめ
- (横断的な発見の整理)
- (現代環境でどう活かせそうか — ユーザが温故知新の文脈で聞いているなら)
```

## 8. 禁忌・注意事項

- **アーカイブの内容を二次配布しない**: 雑誌付属ディスク (電脳倶楽部、Oh!X) は著作物。ユーザが取り出した内容を「ネット公開してみては」と提案しない
- **作者・開発者・楽曲タイトル等の知的成果は尊重**して引用 (出典必須はその一環)
- **古いコードを「動く」と保証しない**: 「X68000 実機 or XM6 等エミュレータで動作する想定」と断る
- **検索ヒットが無いとき**: 推測で答えず「アーカイブ内には見つかりませんでした」と返す。一般知識による回答が必要なら明示的に区別する

## 9. 索引メタ情報

索引対象拡張子 (本文索引):
`.DOC` `.TXT` `.ZMS` `.MDD` `.BAS` `.X` `.S` `.C` `.H` `.MAC` `.INC` `.ASM` `.BAT` `.MD` `.INI` `.CFG`

LZH 内部メンバーも上記拡張子に該当すれば索引対象 (深度2まで)。
バイナリは検索対象外だがファイル名・サイズ・mtime はメタ索引に存在 (拡張子フィルタで列挙可能)。
