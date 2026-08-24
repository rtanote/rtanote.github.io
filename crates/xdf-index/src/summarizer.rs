//! AI 要約バッチの中心ロジック
//!
//! - 索引対象イメージを列挙 → グルーピング設定で束ねる
//! - 各グループごとに「ファイル一覧 + 主要テキスト先頭」をプロンプト化 (複数枚なら連結)
//! - Anthropic API 呼び出し → 構造化 JSON で受け取る
//! - `<index>/summaries/<group_id>.json` にキャッシュ

use crate::anthropic::{estimate_cost, AnthropicClient, MODEL_SONNET};
use crate::builder::{compute_image_id, discover_images};
use crate::grouping::{resolve_groups, GroupConfig, ResolvedGroup};
use crate::summary::{
    find_summary_by_image_id, load_summary, save_summary, HighlightEntry, ImageSummary,
    MemberInfo, UsageInfo,
};
use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use xdf_fs::bpb::Bpb;
use xdf_fs::direntry::Attr;
use xdf_fs::fs::Filesystem;
use xdf_fs::image::{DiskImage, OpenedImage};
use xdf_fs::walker;

/// 要約オプション
pub struct SummarizeOpts {
    pub model: String,
    pub lang: String,
    /// 入力に含める「主要テキスト」の最大バイト数 (1 メンバあたり)
    pub max_text_bytes: usize,
    /// 入力に含めるファイル一覧の最大件数 (1 メンバあたり)
    pub max_listing: usize,
    /// 既存サマリを上書き再生成する
    pub force: bool,
    /// この USD を超えたら以降のグループを skip (None なら無制限)
    pub max_cost_usd: Option<f64>,
    /// dry-run: API 呼ばず、入力プロンプトだけ表示して終了
    pub dry_run: bool,
    /// API 呼び出し間の sleep 秒数。Anthropic Tier 1 のレート制限
    /// (30K input TPM 程度) に引っかかる場合は 25-30 秒推奨。0 で無効。
    pub rate_sleep_secs: u64,
}

impl Default for SummarizeOpts {
    fn default() -> Self {
        Self {
            model: MODEL_SONNET.to_string(),
            lang: "ja".to_string(),
            max_text_bytes: 16 * 1024,
            max_listing: 200,
            force: false,
            max_cost_usd: None,
            dry_run: false,
            rate_sleep_secs: 0,
        }
    }
}

#[derive(Debug, Default)]
pub struct SummarizeStats {
    pub processed: usize,
    pub skipped_existing: usize,
    pub failed: usize,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cost_usd: f64,
}

/// archive_dir を再帰スキャン → グルーピング → 各グループを要約
///
/// `group_cfg` が `None` なら全イメージが単独 (旧挙動互換)。
pub fn summarize_archive<A: AsRef<Path>, I: AsRef<Path>>(
    archive_dir: A,
    index_dir: I,
    opts: SummarizeOpts,
    group_cfg: Option<&GroupConfig>,
) -> Result<SummarizeStats> {
    let archive_dir = archive_dir.as_ref();
    let index_dir = index_dir.as_ref();

    let client = if opts.dry_run {
        None
    } else {
        Some(AnthropicClient::from_env()?)
    };

    let mut stats = SummarizeStats::default();

    let images = discover_images(archive_dir)?;
    let empty_cfg = GroupConfig::empty();
    let cfg = group_cfg.unwrap_or(&empty_cfg);
    let groups = resolve_groups(&images, cfg, |p| compute_image_id(p))?;

    for group in groups {
        // 既存キャッシュ判定: group_id 直接ヒット OR 任意メンバが既存サマリに含まれる
        if !opts.force {
            if load_summary(index_dir, &group.id)?.is_some() {
                stats.skipped_existing += 1;
                continue;
            }
            // メンバが他のグループに含まれているケースも skip
            let mut covered = false;
            for m in &group.members {
                let id = compute_image_id(m)?;
                if find_summary_by_image_id(index_dir, &id)?.is_some() {
                    covered = true;
                    break;
                }
            }
            if covered {
                stats.skipped_existing += 1;
                continue;
            }
        }

        // コスト上限チェック
        if let Some(limit) = opts.max_cost_usd {
            if stats.total_cost_usd >= limit {
                eprintln!(
                    "max-cost-usd reached ({:.4} >= {:.4}), stopping",
                    stats.total_cost_usd, limit
                );
                break;
            }
        }

        match summarize_one_group(&client, &group, &opts) {
            Ok(summary) => {
                if let Some(usage) = &summary.usage {
                    stats.total_input_tokens += usage.input_tokens;
                    stats.total_output_tokens += usage.output_tokens;
                    stats.total_cost_usd += usage.estimated_cost_usd;
                }
                if !opts.dry_run {
                    save_summary(index_dir, &summary)?;
                }
                stats.processed += 1;
                eprintln!(
                    "[{}] {} ({} member{}, ~${:.4})",
                    summary.image_id,
                    summary.image_path,
                    summary.members.len().max(1),
                    if summary.members.len() <= 1 { "" } else { "s" },
                    summary
                        .usage
                        .as_ref()
                        .map(|u| u.estimated_cost_usd)
                        .unwrap_or(0.0)
                );
            }
            Err(e) => {
                stats.failed += 1;
                eprintln!("FAILED {}: {}", group.members[0].display(), e);
            }
        }

        // レート制限対策の sleep (dry-run 時は不要)
        if !opts.dry_run && opts.rate_sleep_secs > 0 {
            std::thread::sleep(std::time::Duration::from_secs(opts.rate_sleep_secs));
        }
    }
    Ok(stats)
}

/// 1 グループを要約 (API 呼び出し1回)
fn summarize_one_group(
    client: &Option<AnthropicClient>,
    group: &ResolvedGroup,
    opts: &SummarizeOpts,
) -> Result<ImageSummary> {
    // 全メンバのスナップショット収集
    let mut snapshots: Vec<ImageSnapshot> = Vec::with_capacity(group.members.len());
    let mut member_infos: Vec<MemberInfo> = Vec::with_capacity(group.members.len());
    for m in &group.members {
        let snap = collect_image_snapshot(m, opts.max_text_bytes, opts.max_listing)?;
        let id = compute_image_id(m)?;
        member_infos.push(MemberInfo {
            image_id: id,
            image_path: m.to_string_lossy().to_string(),
            format: snap.format.clone(),
            size: snap.total_size,
            file_count: snap.file_count,
        });
        snapshots.push(snap);
    }

    let prompt = build_group_prompt(&group.members, &snapshots, &opts.lang);
    let primary = &snapshots[0];
    let primary_path = group.members[0].to_string_lossy().to_string();

    if opts.dry_run {
        let title = if group.members.len() == 1 {
            format!("DRY RUN: {} (solo)", primary_path)
        } else {
            format!(
                "DRY RUN: {} (group of {}: {})",
                group.id,
                group.members.len(),
                group
                    .members
                    .iter()
                    .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        println!("=== {} ===", title);
        println!("--- system ---");
        println!("{}", system_prompt(&opts.lang, group.members.len()));
        println!("--- user (first 1500 chars) ---");
        let preview: String = prompt.chars().take(1500).collect();
        println!("{}", preview);
        println!("=== END ===\n");

        return Ok(ImageSummary {
            image_id: group.id.clone(),
            image_path: primary_path,
            format: primary.format.clone(),
            size: primary.total_size,
            file_count: primary.file_count,
            summarized_at: Utc::now().to_rfc3339(),
            model: opts.model.clone(),
            lang: opts.lang.clone(),
            summary: "[dry-run]".to_string(),
            categories: vec![],
            topics: vec![],
            highlights: vec![],
            usage: None,
            members: member_infos,
            origin: group.origin.clone(),
        });
    }

    let client = client
        .as_ref()
        .ok_or_else(|| anyhow!("API client missing (programmer error)"))?;
    let result = client.complete(
        &opts.model,
        &system_prompt(&opts.lang, group.members.len()),
        &prompt,
        2000,
    )?;

    let parsed = parse_summary_json(&result.text).with_context(|| {
        format!(
            "Cannot parse model output as JSON. Raw output:\n{}",
            result.text.chars().take(500).collect::<String>()
        )
    })?;

    let cost = estimate_cost(&opts.model, result.input_tokens, result.output_tokens);
    Ok(ImageSummary {
        image_id: group.id.clone(),
        image_path: primary_path,
        format: primary.format.clone(),
        size: primary.total_size,
        file_count: primary.file_count,
        summarized_at: Utc::now().to_rfc3339(),
        model: opts.model.clone(),
        lang: opts.lang.clone(),
        summary: parsed.summary,
        categories: parsed.categories,
        topics: parsed.topics,
        highlights: parsed.highlights,
        usage: Some(UsageInfo {
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            estimated_cost_usd: cost,
        }),
        members: member_infos,
        origin: group.origin.clone(),
    })
}

/// イメージから「サマリ用入力素材」を抽出
struct ImageSnapshot {
    format: String,
    total_size: u64,
    file_count: usize,
    /// パス + サイズ
    listing: Vec<(String, u64)>,
    volume_label: Option<String>,
    /// 主要テキスト (例: 一番大きい .DOC) のパスと先頭バイト
    main_text: Option<(String, String)>,
}

fn collect_image_snapshot(
    img_path: &Path,
    max_text_bytes: usize,
    max_listing: usize,
) -> Result<ImageSnapshot> {
    let metadata = std::fs::metadata(img_path)?;
    let total_size = metadata.len();
    let opened = OpenedImage::open(img_path)?;
    let format = opened.format_name().to_ascii_lowercase();

    let mut listing: Vec<(String, u64)> = Vec::new();
    let mut volume_label: Option<String> = None;
    let mut text_candidates: Vec<(String, u64, Vec<u8>)> = Vec::new();

    let mut walk_fs = |fs: &Filesystem, prefix: &str| -> Result<()> {
        // ボリュームラベルを取得
        if volume_label.is_none() {
            let root = fs.read_root_dir()?;
            for e in root.iter() {
                if e.attr.contains(Attr::VOLUME) {
                    volume_label = Some(e.display_name());
                    break;
                }
            }
        }
        let mut error: Option<anyhow::Error> = None;
        walker::walk(fs, |item| {
            if item.entry.attr.contains(Attr::VOLUME)
                || item.entry.attr.contains(Attr::DIRECTORY)
            {
                return true;
            }
            let path = if prefix.is_empty() {
                item.path.clone()
            } else {
                format!("{}{}", prefix, item.path)
            };
            listing.push((path.clone(), item.entry.size as u64));

            // テキスト候補 (.DOC / .TXT) を収集
            let ext_upper = item.entry.ext.to_ascii_uppercase();
            if (ext_upper == "DOC" || ext_upper == "TXT")
                && item.entry.size > 0
                && (item.entry.size as usize) <= max_text_bytes * 4
            {
                if let Ok(bytes) = fs.read_file(item.entry) {
                    text_candidates.push((path, item.entry.size as u64, bytes));
                }
            }
            true
        })
        .map_err(|e| {
            error = Some(e.into());
        })
        .ok();
        if let Some(e) = error {
            return Err(e);
        }
        Ok(())
    };

    match opened {
        OpenedImage::Floppy(img) => {
            let fs = Filesystem::open(&img)?;
            walk_fs(&fs, "")?;
        }
        OpenedImage::Hdd(hdd) => {
            for idx in 0..hdd.partitions().len() {
                let part = match hdd.partition(idx) {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                let boot = match part.read_sector(0) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                let bpb = match Bpb::parse_hdd(boot) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                let fs = match Filesystem::open_with_bpb(&part, bpb) {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                let prefix = format!("[part{}]", idx);
                walk_fs(&fs, &prefix)?;
            }
        }
    }

    let file_count = listing.len();
    if listing.len() > max_listing {
        listing.truncate(max_listing);
    }

    // 一番大きい text 候補を main_text に
    text_candidates.sort_by(|a, b| b.1.cmp(&a.1));
    let main_text = text_candidates.into_iter().next().map(|(path, _, bytes)| {
        let text = encoding_rs::SHIFT_JIS.decode(&bytes).0.into_owned();
        let truncated: String = text.chars().take(max_text_bytes / 2).collect();
        (path, truncated)
    });

    Ok(ImageSnapshot {
        format,
        total_size,
        file_count,
        listing,
        volume_label,
        main_text,
    })
}

fn system_prompt(lang: &str, member_count: usize) -> String {
    let multi_disk_note_ja = if member_count > 1 {
        format!(
            "\n\n**重要**: これは {} 枚組の関連ディスクセット (例: 雑誌1号分の A 面 / B 面 / X 面) です。\
             個別の面ごとではなく、セット全体としての概要・特徴・代表的なファイルを記述してください。\
             highlights のパスにはどのディスクのファイルか分かるよう接頭辞を付けてもOK。",
            member_count
        )
    } else {
        String::new()
    };
    let multi_disk_note_en = if member_count > 1 {
        format!(
            "\n\n**Important**: This is a {}-disk related set (e.g. side A/B/X of one magazine issue). \
             Summarize the SET as a whole, not individual disks. \
             You may prefix highlight paths with the disk name to disambiguate.",
            member_count
        )
    } else {
        String::new()
    };

    if lang == "en" {
        format!(
            r#"You are an archivist who analyzes X68000 (Sharp's 1987 personal computer) disk image contents.

You will be shown a file listing and excerpts from one disk image. Produce a structured summary as JSON.

Output ONLY a JSON object with these keys (no surrounding markdown):
{{
  "summary": "Natural-language description in 2-4 sentences. Mention dominant content type (music data / source code / executables / documents / etc.), notable software or topics, and approximate era if guessable.",
  "categories": ["array of high-level category labels, e.g. 'music', 'game', 'graphics', 'system', 'document'"],
  "topics": ["array of specific subjects, e.g. 'Z-MUSIC', 'X-BASIC', 'FM-OPM', 'sprite']", 'PIC viewer'],
  "highlights": [
    {{"path": "/exact/path/to/file", "note": "why this file is interesting"}},
    ... up to 5 entries
  ]
}}

Be specific. Use information from the file listing and main text.{}"#,
            multi_disk_note_en
        )
    } else {
        format!(
            r#"あなたは X68000 (シャープが 1987 年に発売したパーソナルコンピュータ) のディスクイメージの中身を解析するアーキビストです。

ディスク 1 個のファイル一覧と主要テキストの抜粋が与えられます。これをもとに構造化された要約を JSON で生成してください。

JSON のキーは以下に固定 (前後にコードブロック等は付けない):
{{
  "summary": "ディスク全体の概要を 2〜4 文で。主な内容種別 (音楽データ / ソースコード / 実行ファイル / ドキュメント等)、特徴的なソフトウェアやトピック、推測できる年代があれば言及。",
  "categories": ["音楽", "ゲーム", "グラフィック", "システム", "ドキュメント" 等の高位カテゴリ。複数可"],
  "topics": ["Z-MUSIC", "X-BASIC", "FM音源", "スプライト" 等の具体的なトピック。複数可"],
  "highlights": [
    {{"path": "/正確なパス/file", "note": "そのファイルが注目に値する理由"}},
    ... 最大 5 件
  ]
}}

具体的に記述してください。ファイル一覧と主要テキストから読み取れる情報を活用すること。{}"#,
            multi_disk_note_ja
        )
    }
}

/// 1 メンバ分のテキストブロック
fn render_member_block(
    paths: &Path,
    snap: &ImageSnapshot,
    lang: &str,
    label: Option<&str>,
) -> String {
    let mut s = String::new();
    if let Some(lbl) = label {
        if lang == "en" {
            s.push_str(&format!("=== Disk: {} ({}) ===\n", lbl, paths.display()));
        } else {
            s.push_str(&format!("=== ディスク: {} ({}) ===\n", lbl, paths.display()));
        }
    }
    if lang == "en" {
        s.push_str(&format!(
            "Image format: {}\nTotal bytes: {}\nFile count (after LZH expansion): {}\n",
            snap.format.to_uppercase(),
            snap.total_size,
            snap.file_count
        ));
        if let Some(l) = &snap.volume_label {
            s.push_str(&format!("Volume label: {}\n", l));
        }
        s.push_str(&format!("\nFile listing (first {}):\n", snap.listing.len()));
    } else {
        s.push_str(&format!(
            "イメージ形式: {}\n総バイト数: {}\nファイル数 (LZH展開後): {}\n",
            snap.format.to_uppercase(),
            snap.total_size,
            snap.file_count
        ));
        if let Some(l) = &snap.volume_label {
            s.push_str(&format!("ボリュームラベル: {}\n", l));
        }
        s.push_str(&format!(
            "\nファイル一覧 (先頭{}件):\n",
            snap.listing.len()
        ));
    }
    for (path, size) in &snap.listing {
        s.push_str(&format!("  {} ({} B)\n", path, size));
    }
    if let Some((path, text)) = &snap.main_text {
        if lang == "en" {
            s.push_str(&format!(
                "\nMain text content from {}:\n---\n{}\n---\n",
                path, text
            ));
        } else {
            s.push_str(&format!(
                "\n主要テキストの抜粋 ({}):\n---\n{}\n---\n",
                path, text
            ));
        }
    }
    s
}

fn build_group_prompt(
    paths: &[PathBuf],
    snapshots: &[ImageSnapshot],
    lang: &str,
) -> String {
    if snapshots.len() == 1 {
        return render_member_block(&paths[0], &snapshots[0], lang, None);
    }
    let mut out = String::new();
    if lang == "en" {
        out.push_str(&format!(
            "This is a multi-disk set ({} disks total).\n\n",
            snapshots.len()
        ));
    } else {
        out.push_str(&format!(
            "これは {} 枚組のディスクセットです。各ディスクの内容を以下に列挙します。\n\n",
            snapshots.len()
        ));
    }
    for (i, (p, snap)) in paths.iter().zip(snapshots.iter()).enumerate() {
        let label = format!("{}", i + 1);
        out.push_str(&render_member_block(p, snap, lang, Some(&label)));
        out.push('\n');
    }
    out
}

/// モデル出力 JSON のパース
#[derive(Debug, Deserialize)]
struct ParsedSummary {
    summary: String,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    highlights: Vec<HighlightEntry>,
}

fn parse_summary_json(text: &str) -> Result<ParsedSummary> {
    // JSONの前後にmarkdownブロックがあれば除去
    let trimmed = text.trim();
    let cleaned = if trimmed.starts_with("```") {
        // ```json ... ``` または ``` ... ``` を剥がす
        let after_first = trimmed
            .splitn(2, '\n')
            .nth(1)
            .unwrap_or(trimmed);
        let before_last = after_first.rsplit_once("```").map(|(s, _)| s).unwrap_or(after_first);
        before_last.trim()
    } else {
        trimmed
    };
    let parsed: ParsedSummary = serde_json::from_str(cleaned)
        .or_else(|_| {
            // モデルがゆるい JSON を返した場合のサルベージ: 最初の `{` から最後の `}` まで
            let start = cleaned.find('{').unwrap_or(0);
            let end = cleaned.rfind('}').map(|i| i + 1).unwrap_or(cleaned.len());
            serde_json::from_str(&cleaned[start..end])
        })
        .map_err(|e| anyhow!("JSON parse failed: {}", e))?;
    if parsed.summary.trim().is_empty() {
        bail!("Parsed summary is empty");
    }
    Ok(parsed)
}

/// dry-run-show-groups: 実 API なしでグルーピング結果だけを表示
pub fn show_groups<A: AsRef<Path>>(
    archive_dir: A,
    cfg: &GroupConfig,
) -> Result<Vec<ResolvedGroup>> {
    let images = discover_images(archive_dir.as_ref())?;
    let groups = resolve_groups(&images, cfg, |p| compute_image_id(p))?;
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clean_json() {
        let raw = r#"{"summary":"foo","categories":["bar"],"topics":["baz"],"highlights":[]}"#;
        let p = parse_summary_json(raw).unwrap();
        assert_eq!(p.summary, "foo");
        assert_eq!(p.categories, vec!["bar"]);
    }

    #[test]
    fn parse_with_markdown_fence() {
        let raw = "```json\n{\"summary\":\"hi\"}\n```";
        let p = parse_summary_json(raw).unwrap();
        assert_eq!(p.summary, "hi");
    }

    #[test]
    fn parse_empty_summary_errors() {
        let raw = r#"{"summary":""}"#;
        assert!(parse_summary_json(raw).is_err());
    }

    #[test]
    fn system_prompt_mentions_multi_disk() {
        let p = system_prompt("ja", 3);
        assert!(p.contains("3 枚組"), "should mention multi-disk note: {}", p);
        let p1 = system_prompt("ja", 1);
        assert!(!p1.contains("枚組"));
    }
}
