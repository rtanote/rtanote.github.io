//! tantivy スキーマ定義
//!
//! ドキュメント1件は次のいずれか:
//! - **ファイル本文ドキュメント** (既存): 1 image 内の 1 ファイル。`category` は空文字列。
//! - **構造化メタデータドキュメント** (Phase 5b): 雑誌等から抽出した1レコード
//!   (例: `category="music"` の楽曲1件)。`unique_id`, `payload` JSON, `location_facet` を持つ。
//!
//! 両者は同一スキーマ・同一インデクス内で共存する。`archive_search` は body + file_name を
//! クエリパーサに渡しているため、構造化ドキュメントは（payload が TEXT 索引対象でも）
//! 既存検索の挙動を変えない。
//!
//! 設計詳細は `docs/additional_spec/design.md`「Tantivy スキーマ拡張」参照。

use tantivy::schema::{
    FacetOptions, JsonObjectOptions, Schema, TextFieldIndexing, FAST, INDEXED, STORED, STRING, TEXT,
};

/// 索引のフィールド集合をまとめた構造体
pub struct ArchiveSchema {
    pub schema: Schema,
    pub image_path: tantivy::schema::Field,
    pub image_id: tantivy::schema::Field,
    pub partition: tantivy::schema::Field,
    pub file_path: tantivy::schema::Field,
    pub file_name: tantivy::schema::Field,
    pub ext: tantivy::schema::Field,
    pub size: tantivy::schema::Field,
    pub mtime: tantivy::schema::Field,
    pub attr: tantivy::schema::Field,
    pub body: tantivy::schema::Field,
    pub body_excerpt: tantivy::schema::Field,

    // --- Phase 5b: 構造化メタデータ ---
    /// 例: "music" / "gallery" / "tool" / "" (= 構造化レコードでないファイル本文)
    pub category: tantivy::schema::Field,
    /// 号番号 (例: 94)
    pub issue_no: tantivy::schema::Field,
    /// "main" (本誌) / "extra" (別冊) / "" (該当なし)
    pub issue_kind: tantivy::schema::Field,
    /// "A" / "B" / "X" (多面ディスク識別)、または ""
    pub issue_role: tantivy::schema::Field,
    /// 再抽出用の delete_term キー (例: "dennou:9dc9b31f:MUSIC/SONGA.DOC")
    pub unique_id: tantivy::schema::Field,
    /// 抽出に使ったプラグイン名 (例: "dennou")
    pub schema_plugin: tantivy::schema::Field,
    /// プラグインのバージョン (上げると再抽出対象になる)
    pub schema_version: tantivy::schema::Field,
    /// カテゴリ依存の構造化データ (JSON)
    pub payload: tantivy::schema::Field,
    /// 階層ファセット (例: "/dennou/main/094")
    pub location_facet: tantivy::schema::Field,
}

impl ArchiveSchema {
    pub fn build() -> Self {
        let mut sb = Schema::builder();
        // メタフィールド
        let image_path = sb.add_text_field("image_path", STRING | STORED);
        let image_id = sb.add_text_field("image_id", STRING | STORED);
        let partition = sb.add_u64_field("partition", INDEXED | STORED | FAST);
        let file_path = sb.add_text_field("file_path", STRING | STORED);
        let file_name = sb.add_text_field("file_name", TEXT | STORED);
        let ext = sb.add_text_field("ext", STRING | STORED | FAST);
        let size = sb.add_u64_field("size", INDEXED | STORED | FAST);
        let mtime = sb.add_i64_field("mtime", INDEXED | STORED | FAST);
        let attr = sb.add_text_field("attr", STRING | STORED);
        // 本文 (テキスト系のみ)
        let body = sb.add_text_field("body", TEXT);
        let body_excerpt = sb.add_text_field("body_excerpt", STORED);

        // --- 構造化メタデータ (Phase 5b) ---
        let category = sb.add_text_field("category", STRING | STORED | FAST);
        let issue_no = sb.add_u64_field("issue_no", INDEXED | STORED | FAST);
        let issue_kind = sb.add_text_field("issue_kind", STRING | STORED | FAST);
        let issue_role = sb.add_text_field("issue_role", STRING | STORED | FAST);
        let unique_id = sb.add_text_field("unique_id", STRING | STORED);
        let schema_plugin = sb.add_text_field("schema_plugin", STRING | STORED | FAST);
        let schema_version = sb.add_u64_field("schema_version", STORED | FAST);
        // payload: JSON。デフォルトトークナイザで indexed (TEXT) かつ stored
        let payload_opts = JsonObjectOptions::default()
            .set_stored()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_index_option(tantivy::schema::IndexRecordOption::WithFreqsAndPositions)
                    .set_tokenizer("default"),
            );
        let payload = sb.add_json_field("payload", payload_opts);
        let location_facet = sb.add_facet_field("location_facet", FacetOptions::default());

        Self {
            schema: sb.build(),
            image_path,
            image_id,
            partition,
            file_path,
            file_name,
            ext,
            size,
            mtime,
            attr,
            body,
            body_excerpt,
            category,
            issue_no,
            issue_kind,
            issue_role,
            unique_id,
            schema_plugin,
            schema_version,
            payload,
            location_facet,
        }
    }

    /// 既存索引のスキーマに構造化フィールドが含まれているかを判定。
    /// 旧 (Phase 4 まで) で作られた索引には含まれず、抽出は失敗する。
    pub fn has_structured_fields(s: &Schema) -> bool {
        s.get_field("category").is_ok()
            && s.get_field("issue_no").is_ok()
            && s.get_field("payload").is_ok()
            && s.get_field("unique_id").is_ok()
            && s.get_field("location_facet").is_ok()
    }
}

/// 本文索引対象とする拡張子のデフォルト集合 (大文字統一)
pub const DEFAULT_TEXT_EXTENSIONS: &[&str] = &[
    "DOC", "TXT", "ZMS", "MDD", "BAS", "X", "S", "C", "H",
    "MAC", "INC", "ASM", "BAT", "MD", "INI", "CFG",
];

/// 拡張子 (ASCII大文字に正規化済み) がテキスト系か
pub fn is_text_extension(ext: &str) -> bool {
    DEFAULT_TEXT_EXTENSIONS.contains(&ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_builds_and_has_all_fields() {
        let s = ArchiveSchema::build();
        // 各フィールドが取得できることだけ確認 (panic しない)
        let _ = s.schema.get_field("image_id").unwrap();
        let _ = s.schema.get_field("partition").unwrap();
        let _ = s.schema.get_field("file_path").unwrap();
        let _ = s.schema.get_field("body").unwrap();
        let _ = s.schema.get_field("body_excerpt").unwrap();
        // 構造化フィールド (Phase 5b)
        let _ = s.schema.get_field("category").unwrap();
        let _ = s.schema.get_field("issue_no").unwrap();
        let _ = s.schema.get_field("payload").unwrap();
        let _ = s.schema.get_field("location_facet").unwrap();
        let _ = s.schema.get_field("unique_id").unwrap();
        assert!(ArchiveSchema::has_structured_fields(&s.schema));
    }

    #[test]
    fn text_extensions_recognized() {
        assert!(is_text_extension("DOC"));
        assert!(is_text_extension("ZMS"));
        assert!(is_text_extension("BAS"));
        assert!(!is_text_extension("X8")); // 実行ファイルは除外
        assert!(!is_text_extension("PIC")); // 画像は除外
    }
}
