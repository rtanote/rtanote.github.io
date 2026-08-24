# X68000 HDD イメージフォーマット (HDS / HDF)

X68000 のハードディスクイメージは2系統存在する:

- **HDS** = **SCSI** HDD イメージ (1990年代以降の主流、大容量)
- **HDF** = **SASI** HDD イメージ (初期X68000内蔵HDD用)

両者ともディスク全体のベタダンプ (raw image)。物理セクタサイズが異なる:

| 形式 | バス | 物理セクタ | 典型容量 |
|---|---|---|---|
| HDS | SCSI | 512 B | 数百MB〜数GB |
| HDF | SASI | 256 B | 10〜80 MB |

本ドキュメントは実サンプル (`tests/data/SCSIHDD1.HDS` 900MB, `tests/data/hd1.hdf` 40MB)
を実機解析した結果に基づく。**未検証部分は明記**。

---

## 1. ディスクヘッダ (sector 0)

両形式で異なる。

### HDS (SCSI) — 識別ヘッダ + ジオメトリ

セクタ 0 (先頭 512 B) は識別情報のみ。実行コードは含まない。

| オフセット | サイズ | 内容 | サンプル値 |
|---|---|---|---|
| 0x00 | 8 B | マジック ASCII | `"X68SCSI1"` |
| 0x08 | 8 B | ジオメトリ/設定 | `02 00 00 1C 1F FF 01 00` (詳細未解明) |
| 0x10 | 36 B | 識別文字列 (ASCII null終端) | `"Human68K SCSI-DISK by Keisoku Giken"` |
| 0x34〜 | -- | パディング (0x00) | -- |

検出方法: 先頭 8 バイトが `"X68SCSI1"` なら HDS と確定。

### HDF (SASI) — IPL ブートコードを直接含む

セクタ 0 (先頭 256 B) は m68k IPL コード。識別ヘッダはない。

| オフセット | サイズ | 内容 | サンプル |
|---|---|---|---|
| 0x00 | 4 B | m68k 命令 (BRA等) | `60 00 00 CA` |
| 0x04 | -- | IPL コード | -- |
| 0x40〜 | -- | エスケープシーケンス + 日本語IPLメニュー | `"X68000 HARD DISK IPL MENU"` |

検出方法: マジックなし。HDS マジック不在 + 拡張子 `.hdf` + ファイルサイズが 256 で割り切れる、で判定するしかない。
(より堅牢にするなら、後述のパーティションテーブル位置で X68K マジックを確認)

---

## 2. パーティションテーブル (sector 4)

**両形式共通の構造**。所在は物理セクタ 4 (HDS: 0x800, HDF: 0x400)。

### 2.1 ヘッダ (16 B)

| オフセット | サイズ | 型 | 内容 |
|---|---|---|---|
| 0x00 | 4 B | ASCII | マジック `"X68K"` |
| 0x04 | 4 B | u32 BE | 最終使用セクタ + 1 (パーティション末尾) |
| 0x08 | 4 B | u32 BE | ディスク総セクタ数 - 1 (最終LBA) |
| 0x0C | 4 B | u32 BE | 同上 (用途未確認、おそらく予備) |

### 2.2 パーティションエントリ (16 B 単位、複数)

ヘッダの直後 (offset 0x10) から最大 15 個まで連続して並ぶ。

| オフセット | サイズ | 型 | 内容 |
|---|---|---|---|
| 0x00 | 8 B | ASCII | パーティション名 (ヌル/空白パディング)。例: `"Human68k"` |
| 0x08 | 4 B | u32 BE | 開始セクタ (単位は **物理セクタサイズ依存**) |
| 0x0C | 4 B | u32 BE | パーティション長 (セクタ数、同単位) |

**先頭バイトが 0x00 のエントリは「未使用」**として扱う (空きスロット)。

### 2.3 値の単位 (重要)

実サンプルから判明した規則:

| 形式 | 単位 |
|---|---|
| **HDS** | **1024 B 論理セクタ** (= 物理 2セクタ分) |
| **HDF** | **256 B 物理セクタ** |

理由は推測: HDF (SASI) は物理 256B のままで自然に扱える容量域 (~80MB) のため物理単位、HDS は容量が大きく物理セクタだと u32 で 2TB 上限になるため論理単位を採用したものと思われる (要確認)。

実装上は: パーティション開始バイト = `start * unit`, 長さバイト = `length * unit`。

### 2.4 サンプル解析結果

#### HDS sample (`SCSIHDD1.HDS`, 900 MB = 921,600 × 1024 B)

```
offset 0x800:
  X68K magic
  end = 0x000E0C20 = 920,608  (in 1024B sectors)
  last = 0x000E0FFF = 921,599
  last = 0x000E0FFF = 921,599

offset 0x810: partition entry
  name = "Human68k"
  start = 0x00000020 = 32       (= byte offset 0x8000)
  length = 0x000E0C00 = 920,576 (= 942,669,824 bytes ≈ 900 MB)
```

#### HDF sample (`hd1.hdf`, 40 MB ≈ 162,096 × 256 B)

```
offset 0x400:
  X68K magic
  end = 0x00027930 = 162,096    (≈ ディスク総セクタ数)
  last = 0x00027930 = 162,096
  last = 0x0002ACC0 = 175,296   (??? 物理セクタを超える値、要確認)

offset 0x410: partition entry
  name = "Human68k"
  start = 0x00000021 = 33       (= byte offset 0x2100)
  length = 0x000278F8 = 162,040 (= 41,482,240 bytes ≈ 40 MB)
```

---

## 3. パーティション内 BPB (Human68k HDD BPB)

**重要**: XDF (フロッピー) の MS-DOS 互換 BPB とは**異なるレイアウト**。

### 3.1 ヘッダ部分 (0x00 - 0x11, 18 B)

| オフセット | サイズ | 内容 |
|---|---|---|
| 0x00 | 2 B | m68k BRA.S 命令 (`60 xx`) — IPLコードへのジャンプ |
| 0x02 | 16 B | OEM ID 文字列 (空白パディング) |

サンプル OEM:
- HDS: `"SHARP/KG    1.00"` (Sharp製 + Keisoku Giken製 SCSI BIOS)
- HDF: `"Hudson soft 2.00"` (Hudson製 SASI BIOS)

### 3.2 BPB フィールド (0x12 〜)

実サンプル解析 + FAT 配置検証で確定したレイアウト:

| オフセット | サイズ | 型 | 内容 | HDS値 | HDF値 |
|---|---|---|---|---|---|
| 0x12 | 2 B | u16 **BE** | bytes_per_sector (論理) | 1024 | 1024 |
| 0x14 | 1 B | u8 | sectors_per_cluster | 16 | 1 |
| 0x15 | 1 B | u8 | num_fats | 2 | 2 |
| 0x16 | 2 B | u16 BE | reserved_sectors | 1 | 1 |
| 0x18 | 2 B | u16 BE | root_entries | 512 | 512 |
| 0x1A | 2 B | u16 BE | total_sectors_16 (0 if >65535) | 0 | 40,510 |
| 0x1C | 1 B | u8 | media descriptor | 0xF7 | 0xF8 |
| 0x1D | 1 B | u8 | sectors_per_fat (※u8、MS-DOS は u16) | 114 | 80 |
| 0x1E | 4 B | u32 BE | total_sectors_32 (when _16 == 0) | 920,576 | (非該当) |

> ⚠️ MS-DOS BPB との主な相違点:
> - OEM が **16 B** (MS-DOS は 8 B)
> - 多バイト数値が **BE** (MS-DOS は LE)
> - **`num_fats` と `reserved_sectors` の位置・型が逆転**: MS-DOS は `reserved(LE u16) → num_fats(u8)` だが、Human68k HDD は `num_fats(u8) → reserved(BE u16)`
> - sectors_per_fat が **u8** (MS-DOS は u16)
> - total_sectors_32 の位置が **0x1E** (MS-DOS は 0x20)

検証: HDS で FAT1 が partition_start + 1024 (= 1 logical sector) に存在し
(`F7 FF FF FF` = FAT16 entry[0] media, entry[1] clean)、
FAT2 がそこから sectors_per_fat=114 セクタ後にあることを実測で確認済み。

### 3.3 FAT エントリのエンディアン

**FAT16 エントリは BE u16** (m68k native、Human68k HDD では BPB と一貫)。

- HDS: entry[0] = `F7 FF` (= BE 0xF7FF)、media 0xF7 が**上位バイト**に入る
- HDF: entry[0] = `F8 FF` (= BE 0xF8FF)、media 0xF8 が上位バイト
- entry[1] = `FF FF` = 0xFFFF (clean、palindrome)

> MS-DOS 標準の FAT16 は entry[0] = `0xFFXX` (LE で media が低位) だが、
> Human68k HDD は **media が高位バイト**。Sharp/Hudson 独自慣例。

検証: HDF FAT のクラスタチェーン領域 (offset 0x14〜0x1F) を BE で読むと
`00 0b 00 0c 00 0d 00 0e 00 0f 00 10` → 11, 12, 13, 14, 15, 16 と
連続クラスタポインタになる。LE で読むと 2816, 3072, 3328, ... となり不自然。

> ⚠️ XDF (フロッピー) の **FAT12 は MS-DOS互換の LE**。HDD の FAT16 だけが BE。
> 同じ Human68k でもメディアによって違う点に注意。

### 3.4 ファイルシステム種別

両サンプルともクラスタ数 > 4084 のため **FAT16** (FAT12 ではない)。

- HDS: 920,576 / 16 = 57,536 クラスタ
- HDF: 40,510 / 1 = 40,510 クラスタ

→ Phase 2 では `fat12.rs` と並行して `fat16.rs` (またはジェネリックな `fat.rs`) が必要。

---

## 4. 実装上の判断ポイント

### 4.1 形式自動判別 (`image::open_any`)

```
1. ファイル先頭 8 B を読む
2. "X68SCSI1" → HDS確定 (sector size = 512)
3. それ以外で sector 4 (offset 0x800) に "X68K" → HDS の可能性高い
4. それ以外で sector 4 (offset 0x400) に "X68K" → HDF
5. ファイル先頭の BPB が valid → XDF
6. それ以外 → エラー
```

### 4.2 BPB パーサの分岐

XDF と HDD で BPB レイアウトが異なるため、`Bpb` 構造体は同じでも parser を分ける:

- `Bpb::parse_xdf(&[u8])` — 既存実装、MS-DOS互換
- `Bpb::parse_hdd(&[u8])` — 新規、Human68k HDD用 (BE, 16B OEM, u8 sectors_per_fat 等)

または、`DiskImage` 種別から自動的に正しい parser を選ぶラッパを `Filesystem::open` に追加。

### 4.3 FAT12 / FAT16 の選択

クラスタ数で判定 (MS-DOS と同じルール):
- < 4085 → FAT12
- 4085 〜 65524 → FAT16
- ≥ 65525 → FAT32 (X68000では使われないので対応不要)

`Filesystem::open` で BPB から計算したクラスタ数を見て、適切な FAT 実装をロード。

### 4.4 物理 / 論理セクタの吸収

既存の `phys_per_logi` 仕組みでカバーできる:

- HDS: 物理 512、論理 1024 → `phys_per_logi = 2`
- HDF: 物理 256、論理 1024 → `phys_per_logi = 4`
- XDF: 物理 512、論理 1024 → `phys_per_logi = 2`

---

## 5. 未確認事項 (今後の実機/資料突き合わせ対象)

- HDS sector 0 のジオメトリ 8 B の意味
- HDF パーティションテーブルヘッダ 0x0C の値が物理セクタ数を超える理由
- パーティションエントリ単位がなぜ HDS/HDF で違うのか (公式ドキュメント要)
- BPB の `reserved_sectors = 512` が実際にそれだけ予約されているか
- 複数パーティション持つ HDS/HDF サンプルでの動作
- 名前 `"Human68k"` 以外のパーティション種別 (例: スワップ領域)

---

## 6. 参考にすべき公知資料 (今後)

- 電脳倶楽部 内 FORMAT.X / SUSIE.X 等のディスク管理ツール添付ドキュメント
- XM6 TypeG / XM6i ソースコード (HDD読み込み部分)
- X68000 LIBRARY (シャープ純正マニュアル) のHDD関連章
- 「Inside X68000」「X68000 環境ハンドブック」等のサードパーティ書籍

これらと実サンプル解析結果を突き合わせて、本ドキュメントを正式仕様に格上げする。
