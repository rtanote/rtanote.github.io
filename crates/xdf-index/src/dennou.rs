//! 電脳倶楽部 (本誌 / 別冊) スキーマプラグイン
//!
//! 対象: `Dennou\d+[ABX]\.img` (本誌) / `Bessatu\d+[ABX]\.xdf` (別冊) 等。
//! 抽出カテゴリ: `music` (A 面の `MUSIC/*.DOC` を1曲1レコード)。
//!
//! 投稿者は MUSIC/*.DOC の本文末尾、または `QS/MOKUJI.DOC` から補完する。
//! いずれも見つからなかった場合は payload に `_warnings` を付ける。

use crate::extract::{ExtractContext, ExtractResult, ExtractedRecord, SchemaPlugin};
use crate::text_norm::{collapse_spaces, decode_sjis_nfkc};
use anyhow::Result;
use regex::Regex;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;
use xdf_fs::direntry::{Attr, DirEntry};

pub const PLUGIN_NAME: &str = "dennou";
pub const PLUGIN_VERSION: u64 = 1;

pub struct DennouPlugin;

impl SchemaPlugin for DennouPlugin {
    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }
    fn version(&self) -> u64 {
        PLUGIN_VERSION
    }
    fn detect(&self, ctx: &ExtractContext<'_>) -> bool {
        detect_dennou(ctx)
    }
    fn extract(&self, ctx: &ExtractContext<'_>) -> Result<ExtractResult> {
        Ok(extract_dennou(ctx))
    }
}

// ---- ファイル名パターン ----

fn issue_from_filename(image_path: &str) -> Option<(String, u64, String)> {
    static MAIN_RE: OnceLock<Regex> = OnceLock::new();
    static EXTRA_RE: OnceLock<Regex> = OnceLock::new();
    let main = MAIN_RE.get_or_init(|| {
        Regex::new(r"(?i)Dennou0*(\d+)([ABX])\.(?:img|xdf)$").unwrap()
    });
    let extra = EXTRA_RE.get_or_init(|| {
        Regex::new(r"(?i)Bessatu0*(\d+)([ABX])\.(?:img|xdf)$").unwrap()
    });
    let stem = Path::new(image_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if let Some(c) = main.captures(stem) {
        let n: u64 = c[1].parse().ok()?;
        let role = c[2].to_ascii_uppercase();
        return Some(("main".into(), n, role));
    }
    if let Some(c) = extra.captures(stem) {
        let n: u64 = c[1].parse().ok()?;
        let role = c[2].to_ascii_uppercase();
        return Some(("extra".into(), n, role));
    }
    None
}

fn issue_from_autoexec(text: &str) -> Option<(String, u64)> {
    // `\電脳倶楽部\NNN` または `\Bessatu\NN` 系を AUTOEXEC.BAT 内で探す
    static MAIN_RE: OnceLock<Regex> = OnceLock::new();
    static EXTRA_RE: OnceLock<Regex> = OnceLock::new();
    let main = MAIN_RE.get_or_init(|| Regex::new(r"電脳倶楽部\\\s*0*(\d{1,4})").unwrap());
    let extra = EXTRA_RE.get_or_init(|| Regex::new(r"(?i)Bessatu\\\s*0*(\d{1,4})").unwrap());
    if let Some(c) = main.captures(text) {
        let n: u64 = c[1].parse().ok()?;
        return Some(("main".into(), n));
    }
    if let Some(c) = extra.captures(text) {
        let n: u64 = c[1].parse().ok()?;
        return Some(("extra".into(), n));
    }
    None
}

/// detect: ファイル名 OR AUTOEXEC.BAT OR QS/MOKUJI.DOC のいずれかが該当
fn detect_dennou(ctx: &ExtractContext<'_>) -> bool {
    if issue_from_filename(ctx.image_path).is_some() {
        return true;
    }
    if let Ok(entry) = ctx.fs.resolve("AUTOEXEC.BAT") {
        if let Ok(bytes) = ctx.fs.read_file(&entry) {
            let text = decode_sjis_nfkc(&bytes);
            if issue_from_autoexec(&text).is_some() {
                return true;
            }
            if text.contains("電脳倶楽部") {
                return true;
            }
        }
    }
    // QS/MOKUJI.DOC または QUICKSTART/MOKUJI.DOC があれば dennou
    if ctx.fs.resolve("QS/MOKUJI.DOC").is_ok() || ctx.fs.resolve("QUICKSTART/MOKUJI.DOC").is_ok()
    {
        return true;
    }
    false
}

// ---- 抽出本体 ----

fn extract_dennou(ctx: &ExtractContext<'_>) -> ExtractResult {
    let mut warnings: Vec<String> = Vec::new();

    // 1) issue_no / issue_kind / role 解決
    let (kind, issue_no, role) = resolve_issue(ctx, &mut warnings).unwrap_or_else(|| {
        warnings.push("Could not determine issue_no/kind".to_string());
        ("main".to_string(), 0, String::new())
    });

    // 2) A 面でなければ楽曲抽出はしない (B/X 面は通常 PCM 補助等)
    if !role.eq_ignore_ascii_case("A") {
        return ExtractResult {
            records: Vec::new(),
            warnings,
        };
    }

    // 3) MOKUJI.DOC を先に読んで投稿者ヒントを集める
    let mokuji_hints = collect_mokuji_hints(ctx);

    // 4) MUSIC/ をリストアップしてステム単位にグループ化
    let stems = collect_music_stems(ctx);
    if stems.is_empty() {
        warnings.push("No MUSIC/ directory or no .DOC files found".to_string());
        return ExtractResult {
            records: Vec::new(),
            warnings,
        };
    }

    let location_facet = format!("/dennou/{}/{:03}", kind, issue_no);

    // 5) 各ステム = 1 レコード
    let mut records: Vec<ExtractedRecord> = Vec::new();
    for stem in stems {
        let doc_path = format!("MUSIC/{}.DOC", stem.name);
        let doc_bytes = match read_file_at(ctx, &doc_path) {
            Ok(b) => b,
            Err(_) => continue, // .DOC 無いステムは曲ではない (G9.BFD 等)
        };
        let doc_text = decode_sjis_nfkc(&doc_bytes);
        let mut rec_warnings: Vec<String> = Vec::new();
        let header = parse_doc_header(&doc_text, &mut rec_warnings);
        let submitter = parse_submitter(&doc_text)
            .or_else(|| mokuji_hints.get(&stem.name.to_ascii_uppercase()).cloned().and_then(|h| h.submitter))
            .or_else(|| {
                rec_warnings.push("submitter not found".to_string());
                None
            });

        let mut payload = serde_json::Map::new();
        payload.insert("issue_no".into(), serde_json::json!(issue_no));
        payload.insert("issue_kind".into(), serde_json::json!(kind));
        if let Some(t) = &header.title {
            payload.insert("title".into(), serde_json::json!(t));
        } else {
            rec_warnings.push("title not found".to_string());
        }
        if let Some(v) = &header.original_artist {
            payload.insert("original_artist".into(), serde_json::json!(v));
        }
        if let Some(v) = &header.composer {
            payload.insert("composer".into(), serde_json::json!(v));
        }
        if let Some(v) = &header.arranger {
            payload.insert("arranger".into(), serde_json::json!(v));
        }
        if let Some(v) = &header.engine {
            payload.insert("engine".into(), serde_json::json!(v));
        }
        if let Some(v) = &header.duration {
            payload.insert("duration".into(), serde_json::json!(v));
        }
        if let Some(s) = &submitter {
            payload.insert("submitter".into(), serde_json::json!(s.name));
            payload.insert("submitter_pref".into(), serde_json::json!(s.pref));
        }

        // ファイル対応 (DOC/ZMS/ZPD など)
        let mut files = serde_json::Map::new();
        for (kind_key, ext_upper) in [("doc", "DOC"), ("score", "ZMS"), ("voice", "ZPD")] {
            let p = format!("MUSIC/{}.{}", stem.name, ext_upper);
            if read_file_at(ctx, &p).is_ok() {
                files.insert(kind_key.into(), serde_json::json!(p));
            }
        }
        if !files.is_empty() {
            payload.insert("files".into(), serde_json::Value::Object(files));
        }

        // MOKUJI 由来の追加情報
        if let Some(hint) = mokuji_hints.get(&stem.name.to_ascii_uppercase()) {
            if let Some(j) = &hint.jasrac_note {
                payload.insert("jasrac_note".into(), serde_json::json!(j));
            }
        }

        if !rec_warnings.is_empty() {
            payload.insert(
                "_warnings".into(),
                serde_json::Value::Array(
                    rec_warnings
                        .iter()
                        .map(|w| serde_json::json!(w))
                        .collect(),
                ),
            );
        }

        let unique_id = format!(
            "{}:{}:{}",
            PLUGIN_NAME,
            ctx.image_id_short(),
            doc_path
        );

        records.push(ExtractedRecord {
            unique_id,
            category: "music".into(),
            source_path: doc_path,
            image_path: ctx.image_path.into(),
            image_id: ctx.image_id.into(),
            partition: ctx.partition,
            issue_no,
            issue_kind: kind.clone(),
            issue_role: role.clone(),
            location_facet: location_facet.clone(),
            payload: serde_json::Value::Object(payload),
            schema_plugin: PLUGIN_NAME.into(),
            schema_version: PLUGIN_VERSION,
        });
    }

    ExtractResult { records, warnings }
}

fn resolve_issue(
    ctx: &ExtractContext<'_>,
    warnings: &mut Vec<String>,
) -> Option<(String, u64, String)> {
    // 優先: ファイル名
    if let Some(t) = issue_from_filename(ctx.image_path) {
        return Some(t);
    }
    // フォールバック: AUTOEXEC.BAT (役は不明として "")
    if let Ok(entry) = ctx.fs.resolve("AUTOEXEC.BAT") {
        if let Ok(bytes) = ctx.fs.read_file(&entry) {
            let text = decode_sjis_nfkc(&bytes);
            if let Some((kind, n)) = issue_from_autoexec(&text) {
                warnings.push("issue_role unknown (resolved from AUTOEXEC.BAT)".into());
                return Some((kind, n, String::new()));
            }
        }
    }
    None
}

struct DocHeader {
    title: Option<String>,
    original_artist: Option<String>,
    composer: Option<String>,
    arranger: Option<String>,
    engine: Option<String>,
    duration: Option<String>,
}

fn parse_doc_header(text: &str, _warnings: &mut Vec<String>) -> DocHeader {
    // 1) ♪♪♪ 区切り行を見つけ、最初と次の区切り行の間を「ヘッダブロック」とする。
    let lines: Vec<&str> = text.lines().collect();
    let sep_indices: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_separator_line(l))
        .map(|(i, _)| i)
        .collect();
    let (start, end) = match sep_indices.as_slice() {
        [s, e, ..] => (*s + 1, *e),
        [s] => (*s + 1, lines.len().min(*s + 30)),
        _ => (0, lines.len().min(40)),
    };
    let header_lines: Vec<String> = lines[start..end]
        .iter()
        .map(|l| collapse_spaces(l))
        .filter(|l| !l.is_empty())
        .collect();

    let mut title: Option<String> = None;
    let mut original_artist: Option<String> = None;
    let mut composer: Option<String> = None;
    let mut arranger: Option<String> = None;
    let mut engine: Option<String> = None;
    let mut duration: Option<String> = None;

    let composer_re = composer_regex();
    let arranger_re = arranger_regex();
    let engine_re = engine_regex();
    let duration_re = duration_regex();

    let mut title_candidates: Vec<String> = Vec::new();
    for line in &header_lines {
        // メタ行を先に検出
        if composer.is_none() {
            if let Some(c) = composer_re.captures(line) {
                composer = Some(clean_field(&c[1]));
                continue;
            }
        }
        if arranger.is_none() {
            if let Some(c) = arranger_re.captures(line) {
                arranger = Some(clean_field(&c[1]));
                continue;
            }
        }
        if engine.is_none() {
            if let Some(c) = engine_re.captures(line) {
                let name = c.get(1).map(|m| m.as_str()).unwrap_or("");
                let ver = c.get(2).map(|m| m.as_str()).unwrap_or("");
                let s = if ver.is_empty() {
                    name.to_string()
                } else {
                    format!("{} Ver {}", name, ver)
                };
                engine = Some(s);
                continue;
            }
        }
        if duration.is_none() {
            if let Some(c) = duration_re.captures(line) {
                duration = Some(clean_field(&c[1]));
                continue;
            }
        }
        // 上記いずれにも一致しない非空行 → タイトル候補
        if !line.contains("演奏時間")
            && !line.to_ascii_uppercase().contains("MUSIC BY")
            && !line.to_ascii_uppercase().contains("ARRANGE BY")
            && !line.to_ascii_uppercase().contains("ARRENGE BY")
            && !line.contains("作曲")
            && !line.contains("編曲")
        {
            title_candidates.push(line.clone());
        }
    }

    // タイトル: 候補先頭。original_artist: 2 番目があれば。
    let mut iter = title_candidates.into_iter();
    if let Some(t) = iter.next() {
        title = Some(t);
    }
    if let Some(a) = iter.next() {
        original_artist = Some(a);
    }

    DocHeader {
        title,
        original_artist,
        composer,
        arranger,
        engine,
        duration,
    }
}

fn is_separator_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let count = trimmed.chars().filter(|c| *c == '♪').count();
    count >= 5 && trimmed.chars().all(|c| c == '♪' || c.is_whitespace())
}

fn clean_field(s: &str) -> String {
    let s = s.trim();
    // 末尾の括弧書き ( … ) を削除
    let s = if let Some(idx) = s.find('(') {
        s[..idx].trim_end()
    } else {
        s
    };
    let s = if let Some(idx) = s.find('（') {
        s[..idx].trim_end()
    } else {
        s
    };
    s.to_string()
}

fn composer_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // "MUSIC by ..." または "作曲：..." または "作曲 ..."
    R.get_or_init(|| {
        Regex::new(r"(?i)(?:MUSIC\s+by\s+|作曲\s*[:：]?\s*)(.+?)\s*$").unwrap()
    })
}

fn arranger_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // "68ARRANGE by ..." / "ARRENGE by ..." / "ARRENGE" の N 位置揺れに耐える: ARR[AE]N?GE
    // 「編曲：...」も拾う
    R.get_or_init(|| {
        Regex::new(r"(?i)(?:(?:68)?ARR[AE]N?GE\s+by\s+|編曲\s*[:：]?\s*)(.+?)\s*$").unwrap()
    })
}

fn engine_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    // ハイフン部分は ASCII '-', U+2212 (SJIS 817C, NFKC でも変わらない), U+2010 を許容。
    // "Z-MUSIC Ver 2.06" / "Z−MUSIC" / "MXDRV" / "MUSIC LALF"
    R.get_or_init(|| {
        Regex::new(
            r"(?i)\b(Z[-\u{2212}\u{2010}]?MUSIC|MXDRV|MUSIC\s+LALF|MUSPRO|YM2151)\b(?:\s*Ver\.?\s*([0-9.]+))?",
        )
        .unwrap()
    })
}

fn duration_regex() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"演奏時間\s*[:：]?\s*(.+?)\s*$").unwrap())
}

#[derive(Debug, Clone)]
struct Submitter {
    pref: String,
    name: String,
}

fn parse_submitter(text: &str) -> Option<Submitter> {
    static R: OnceLock<Regex> = OnceLock::new();
    let re = R.get_or_init(|| {
        // 例: "茨城県の山田 太郎さんの投稿です"
        // pref: 1〜4 文字の漢字 + 都道府県 (句読点は含めない)
        //       「、東京都」のような前置点があっても pref 部分にはマッチしない。
        // name: さん の直前までを最短で
        Regex::new(r"(\p{Han}{1,4}(?:都|道|府|県))の([^\n]{1,30}?)\s*さん(?:の投稿)?").unwrap()
    });
    let c = re.captures(text)?;
    Some(Submitter {
        pref: c[1].trim().to_string(),
        name: c[2].trim().to_string(),
    })
}

// ---- MUSIC/ ステム列挙 ----

#[derive(Debug)]
struct MusicStem {
    /// 大文字統一済み (例: "SONGA")
    name: String,
}

fn collect_music_stems(ctx: &ExtractContext<'_>) -> Vec<MusicStem> {
    let entries = match ctx.fs.list_path("MUSIC") {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut stems: BTreeMap<String, ()> = BTreeMap::new();
    for e in &entries {
        if e.attr.contains(Attr::DIRECTORY) || e.attr.contains(Attr::VOLUME) {
            continue;
        }
        if e.name == "." || e.name == ".." {
            continue;
        }
        if e.ext.eq_ignore_ascii_case("DOC") {
            // DOC が存在するステムだけを楽曲とみなす
            stems.insert(e.name.to_ascii_uppercase(), ());
        }
    }
    stems.into_iter().map(|(n, _)| MusicStem { name: n }).collect()
}

fn read_file_at(ctx: &ExtractContext<'_>, path: &str) -> Result<Vec<u8>> {
    let entry: DirEntry = ctx.fs.resolve(path)?;
    Ok(ctx.fs.read_file(&entry)?)
}

// ---- MOKUJI.DOC 補助 ----

#[derive(Debug, Clone, Default)]
struct MokujiHint {
    submitter: Option<Submitter>,
    jasrac_note: Option<String>,
}

/// MOKUJI.DOC を読んで `STEM` (大文字) → ヒント のマップを返す。
/// 見つからなかったり読めなかったら空マップ。
fn collect_mokuji_hints(ctx: &ExtractContext<'_>) -> BTreeMap<String, MokujiHint> {
    let candidates = ["QS/MOKUJI.DOC", "QUICKSTART/MOKUJI.DOC"];
    for cand in candidates {
        if let Ok(bytes) = read_file_at(ctx, cand) {
            let text = decode_sjis_nfkc(&bytes);
            return parse_mokuji(&text);
        }
    }
    BTreeMap::new()
}

fn parse_mokuji(text: &str) -> BTreeMap<String, MokujiHint> {
    // ◎演奏を聞く ... TYPE=OPM:A:\MUSIC\SONGA.ZMS のようなマーカー周辺を1ブロックとみなす。
    // ブロック先頭にタイトル、末尾付近に「(都道府県)の(名前)さんの投稿です」「JASRAC R-... 」がある。
    static STEM_RE: OnceLock<Regex> = OnceLock::new();
    let stem_re = STEM_RE.get_or_init(|| {
        Regex::new(r"(?i)A:\\MUSIC\\([A-Za-z0-9_]+)\.ZMS").unwrap()
    });
    static JASRAC_RE: OnceLock<Regex> = OnceLock::new();
    let jasrac_re = JASRAC_RE
        .get_or_init(|| Regex::new(r"(?:JASRAC|日本音楽著作権協会)[^\n]{0,80}").unwrap());

    let mut out: BTreeMap<String, MokujiHint> = BTreeMap::new();

    // ZMS ファイル名を含む行を起点に「次の ZMS 行 まで」を1ブロックにする
    let lines: Vec<&str> = text.lines().collect();
    let stem_positions: Vec<(usize, String)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| stem_re.captures(l).map(|c| (i, c[1].to_ascii_uppercase())))
        .collect();
    let total = lines.len();
    for (idx, (start_line, stem)) in stem_positions.iter().enumerate() {
        let end_line = stem_positions
            .get(idx + 1)
            .map(|(j, _)| *j)
            .unwrap_or(total);
        let block: String = lines[*start_line..end_line].join("\n");
        let mut hint = MokujiHint::default();
        if let Some(s) = parse_submitter(&block) {
            hint.submitter = Some(s);
        }
        if let Some(m) = jasrac_re.find(&block) {
            hint.jasrac_note = Some(collapse_spaces(m.as_str()));
        }
        out.entry(stem.clone()).or_insert(hint);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_from_filename_main() {
        let r = issue_from_filename("/archive/Dennou094A.img").unwrap();
        assert_eq!(r.0, "main");
        assert_eq!(r.1, 94);
        assert_eq!(r.2, "A");
    }

    #[test]
    fn issue_from_filename_extra() {
        let r = issue_from_filename("/archive/Bessatu14A.xdf").unwrap();
        assert_eq!(r.0, "extra");
        assert_eq!(r.1, 14);
        assert_eq!(r.2, "A");
    }

    #[test]
    fn issue_from_filename_zero_padded() {
        let r = issue_from_filename("/archive/Dennou074A.img").unwrap();
        assert_eq!(r.1, 74);
    }

    #[test]
    fn issue_from_filename_b_side_excluded_from_music() {
        // B 面でも識別はできる (ただし extract_dennou は B/X をスキップする)
        let r = issue_from_filename("/archive/Dennou094B.img").unwrap();
        assert_eq!(r.2, "B");
    }

    #[test]
    fn separator_detection() {
        assert!(is_separator_line("♪♪♪♪♪♪♪♪♪♪"));
        assert!(is_separator_line("  ♪♪♪♪♪♪♪♪♪♪  "));
        assert!(!is_separator_line("♪ Title ♪"));
        assert!(!is_separator_line(""));
        assert!(!is_separator_line("MUSIC by 山田 太郎"));
    }

    #[test]
    fn parse_doc_header_typical() {
        // NFKC 後を想定したテキスト (全角は半角化される)。
        // 内容はすべて架空 — 実在の楽曲・人物とは無関係。
        let sample = "
♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪

   　　  SAMPLE SONG TITLE

         Sample Artist

         MUSIC by 山田 太郎

         68ARRENGE by Hanako

         Z-MUSIC Ver 2.06

         演奏時間 3分 19杪

♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪♪
";
        let mut warnings = Vec::new();
        let h = parse_doc_header(sample, &mut warnings);
        assert_eq!(h.title.as_deref(), Some("SAMPLE SONG TITLE"));
        assert_eq!(h.original_artist.as_deref(), Some("Sample Artist"));
        assert_eq!(h.composer.as_deref(), Some("山田 太郎"));
        assert_eq!(h.arranger.as_deref(), Some("Hanako"));
        assert!(h.engine.as_deref().unwrap().contains("Z-MUSIC"));
        assert!(h.duration.as_deref().unwrap().contains("3"));
    }

    #[test]
    fn submitter_pref_name() {
        let sample = "本曲は、茨城県の山田 太郎さんの投稿です。";
        let s = parse_submitter(sample).unwrap();
        assert_eq!(s.pref, "茨城県");
        assert_eq!(s.name, "山田 太郎");
    }

    #[test]
    fn submitter_alt_form() {
        // "投稿" を含まないが「さん」までで切り出せる
        let sample = "山口県の田中花子さんの投稿です";
        let s = parse_submitter(sample).unwrap();
        assert_eq!(s.pref, "山口県");
        assert_eq!(s.name, "田中花子");
    }

    #[test]
    fn parse_mokuji_yields_stem_to_submitter() {
        // 簡略化した MOKUJI.DOC 風サンプル (内容はすべて架空)
        let sample = "
%V%W SAMPLE SONG TITLE
◎演奏を聞く                 TYPE=OPM:A:\\MUSIC\\SONGA.ZMS
◎説明                       TYPE=DOC:A:\\MUSIC\\SONGA.DOC
作曲：山田 太郎             (頒布版：日本音楽著作権協会許諾 R-XXXXXXX)
茨城県の田中花子さんの投稿です。

%V%W ANOTHER SONG
◎演奏を聞く                 TYPE=OPM:A:\\MUSIC\\SONGB.ZMS
東京都の佐藤 太郎さんの投稿です。
";
        let m = parse_mokuji(sample);
        assert_eq!(m.get("SONGA").and_then(|h| h.submitter.as_ref()).map(|s| s.name.clone()), Some("田中花子".to_string()));
        assert_eq!(m.get("SONGB").and_then(|h| h.submitter.as_ref()).map(|s| s.pref.clone()), Some("東京都".to_string()));
        assert!(m.get("SONGA").and_then(|h| h.jasrac_note.as_ref()).is_some());
    }

    #[test]
    fn clean_field_strips_trailing_paren() {
        assert_eq!(clean_field("山田 太郎 (作詞:○○)"), "山田 太郎");
        assert_eq!(clean_field("山田 太郎（メイン）"), "山田 太郎");
        assert_eq!(clean_field("  Hanako  "), "Hanako");
    }
}
