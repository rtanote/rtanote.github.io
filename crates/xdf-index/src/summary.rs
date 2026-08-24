//! イメージ単位 / グループ単位 AI 要約のキャッシュ R/W
//!
//! 保存先: `<index_dir>/summaries/<group_id>.json`
//! - 単独 (1 メンバ): group_id == image_id
//! - 複数 (N 枚フロッピー1組): group_id = sha1(member ids sorted joined)[..16]
//!
//! 後方互換: 旧 schema (members フィールド無し) も読める。`members` が空なら
//! 自身の `image_id` がそのまま 1 人グループとして扱われる。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 1 グループ (= 1 〜 N 個のディスクイメージ) 分の AI 要約結果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSummary {
    /// グループ識別子 (単独なら primary member の image_id、複数ならハッシュ)
    pub image_id: String,
    /// プライマリメンバ (単独なら唯一のイメージ、複数ならソート最先頭) のホストパス
    pub image_path: String,
    /// プライマリメンバの形式: "xdf" / "hds" / "hdf"
    pub format: String,
    /// プライマリメンバのバイト数
    pub size: u64,
    /// プライマリメンバの索引化済みファイル数 (LZH メンバー込み)
    pub file_count: usize,
    /// 要約生成時刻 (RFC3339)
    pub summarized_at: String,
    /// 使用モデル名 (再生成判定用)
    pub model: String,
    /// 言語コード ("ja" / "en")
    pub lang: String,
    /// 自然文要約
    pub summary: String,
    /// 用途別カテゴリ (例: ["音楽", "ゲーム", "ドキュメント"])
    #[serde(default)]
    pub categories: Vec<String>,
    /// 含まれる主要トピック (例: ["Z-MUSIC", "FM音源", "X-BASIC"])
    #[serde(default)]
    pub topics: Vec<String>,
    /// 注目すべきファイル (上位3〜5件)
    #[serde(default)]
    pub highlights: Vec<HighlightEntry>,
    /// 入出力 token 数 (コスト追跡用)
    #[serde(default)]
    pub usage: Option<UsageInfo>,
    /// グループのメンバ一覧 (空なら旧 schema = 単独)
    #[serde(default)]
    pub members: Vec<MemberInfo>,
    /// グルーピング由来 ("solo" / "rule:..." / "manual:..." / "default-solo")
    #[serde(default)]
    pub origin: String,
}

/// グループメンバ 1 枚分のメタ情報
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberInfo {
    pub image_id: String,
    pub image_path: String,
    pub format: String,
    pub size: u64,
    pub file_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HighlightEntry {
    pub path: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageInfo {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// USD 概算
    pub estimated_cost_usd: f64,
}

/// 索引ディレクトリ内の summaries ディレクトリ
pub fn summaries_dir(index_dir: &Path) -> PathBuf {
    index_dir.join("summaries")
}

/// `image_id` に対応する JSON のフルパス
pub fn summary_path(index_dir: &Path, image_id: &str) -> PathBuf {
    summaries_dir(index_dir).join(format!("{}.json", image_id))
}

/// JSON を保存 (整形あり)
pub fn save_summary(index_dir: &Path, summary: &ImageSummary) -> Result<()> {
    let dir = summaries_dir(index_dir);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Cannot create summaries dir {:?}", dir))?;
    let path = summary_path(index_dir, &summary.image_id);
    let json = serde_json::to_string_pretty(summary)?;
    std::fs::write(&path, json)
        .with_context(|| format!("Cannot write summary {:?}", path))?;
    Ok(())
}

/// JSON を読み込み。存在しなければ Ok(None)
pub fn load_summary(index_dir: &Path, image_id: &str) -> Result<Option<ImageSummary>> {
    let path = summary_path(index_dir, image_id);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("Cannot read summary {:?}", path))?;
    let s: ImageSummary = serde_json::from_slice(&bytes)
        .with_context(|| format!("Cannot parse summary {:?}", path))?;
    Ok(Some(s))
}

/// 保存済みの全 group_id (= JSON のファイル名 stem) を列挙
pub fn list_summarized_ids(index_dir: &Path) -> Result<Vec<String>> {
    let dir = summaries_dir(index_dir);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                ids.push(stem.to_string());
            }
        }
    }
    ids.sort();
    Ok(ids)
}

/// `image_id` を含むサマリ (単独 or グループ) を検索。
///
/// ロジック:
/// 1. `summaries/<image_id>.json` を直接読みに行く (単独 or グループ ID == image_id の場合)
/// 2. 見つからなければ `summaries/*.json` を全件スキャンし、`members[].image_id` に一致するものを返す
///
/// マルチメンバグループは数百〜千程度を想定しているのでスキャン O(N) で十分。
pub fn find_summary_by_image_id(
    index_dir: &Path,
    image_id: &str,
) -> Result<Option<ImageSummary>> {
    // 直接ヒット
    if let Some(s) = load_summary(index_dir, image_id)? {
        // 単独 (members 空) の場合は ID 一致を確認、グループの場合は members に含まれているか確認
        if s.members.is_empty() {
            if s.image_id == image_id {
                return Ok(Some(s));
            }
        } else if s.members.iter().any(|m| m.image_id == image_id) {
            return Ok(Some(s));
        }
        // ID は一致するが members に含まれない (壊れたキャッシュ) → fallthrough
    }

    // 全件スキャン (member 経由でグループ要約を見つける)
    let dir = summaries_dir(index_dir);
    if !dir.exists() {
        return Ok(None);
    }
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(&p)
            .with_context(|| format!("Cannot read summary {:?}", p))?;
        let s: ImageSummary = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(_) => continue, // 壊れた JSON は skip
        };
        if s.members.iter().any(|m| m.image_id == image_id) {
            return Ok(Some(s));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample() -> ImageSummary {
        ImageSummary {
            image_id: "abc123".to_string(),
            image_path: "/archive/disk.xdf".to_string(),
            format: "xdf".to_string(),
            size: 1_261_568,
            file_count: 100,
            summarized_at: "2026-04-27T00:00:00Z".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            lang: "ja".to_string(),
            summary: "サンプル要約".to_string(),
            categories: vec!["音楽".to_string()],
            topics: vec!["Z-MUSIC".to_string()],
            highlights: vec![HighlightEntry {
                path: "/MUSIC/SAMPLE.ZMS".to_string(),
                note: "曲データ".to_string(),
            }],
            usage: Some(UsageInfo {
                input_tokens: 5000,
                output_tokens: 800,
                estimated_cost_usd: 0.027,
            }),
            members: vec![],
            origin: String::new(),
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let tmp = TempDir::new().unwrap();
        let s = sample();
        save_summary(tmp.path(), &s).unwrap();
        let loaded = load_summary(tmp.path(), "abc123").unwrap().unwrap();
        assert_eq!(loaded.image_id, s.image_id);
        assert_eq!(loaded.summary, s.summary);
        assert_eq!(loaded.topics, s.topics);
        assert_eq!(loaded.highlights.len(), 1);
    }

    #[test]
    fn load_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        let result = load_summary(tmp.path(), "nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn list_summarized_ids_finds_jsons() {
        let tmp = TempDir::new().unwrap();
        save_summary(tmp.path(), &sample()).unwrap();
        let mut s2 = sample();
        s2.image_id = "xyz789".to_string();
        save_summary(tmp.path(), &s2).unwrap();

        let ids = list_summarized_ids(tmp.path()).unwrap();
        assert_eq!(ids, vec!["abc123", "xyz789"]);
    }

    #[test]
    fn find_by_image_id_solo() {
        let tmp = TempDir::new().unwrap();
        save_summary(tmp.path(), &sample()).unwrap();
        let found = find_summary_by_image_id(tmp.path(), "abc123").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().image_id, "abc123");
    }

    #[test]
    fn find_by_image_id_via_members() {
        let tmp = TempDir::new().unwrap();
        let mut s = sample();
        s.image_id = "groupABCDEF".to_string();
        s.members = vec![
            MemberInfo {
                image_id: "memberA".to_string(),
                image_path: "/a.img".to_string(),
                format: "xdf".to_string(),
                size: 100,
                file_count: 10,
            },
            MemberInfo {
                image_id: "memberB".to_string(),
                image_path: "/b.img".to_string(),
                format: "xdf".to_string(),
                size: 200,
                file_count: 20,
            },
        ];
        save_summary(tmp.path(), &s).unwrap();
        let found = find_summary_by_image_id(tmp.path(), "memberB").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().image_id, "groupABCDEF");
    }

    #[test]
    fn find_by_image_id_missing() {
        let tmp = TempDir::new().unwrap();
        save_summary(tmp.path(), &sample()).unwrap();
        assert!(find_summary_by_image_id(tmp.path(), "nope").unwrap().is_none());
    }

    #[test]
    fn legacy_solo_summary_loads() {
        // 旧 schema (members フィールドなし) の JSON を直接書いて読めるか
        let tmp = TempDir::new().unwrap();
        let dir = summaries_dir(tmp.path());
        std::fs::create_dir_all(&dir).unwrap();
        let legacy_json = r#"{
            "image_id": "legacy1",
            "image_path": "/a.img",
            "format": "xdf",
            "size": 100,
            "file_count": 10,
            "summarized_at": "2026-04-27T00:00:00Z",
            "model": "claude-sonnet-4-6",
            "lang": "ja",
            "summary": "old schema"
        }"#;
        std::fs::write(dir.join("legacy1.json"), legacy_json).unwrap();
        let s = load_summary(tmp.path(), "legacy1").unwrap().unwrap();
        assert_eq!(s.summary, "old schema");
        assert!(s.members.is_empty());
        assert!(s.origin.is_empty());

        // image_id 検索でもヒットする
        let f = find_summary_by_image_id(tmp.path(), "legacy1").unwrap();
        assert!(f.is_some());
    }
}
