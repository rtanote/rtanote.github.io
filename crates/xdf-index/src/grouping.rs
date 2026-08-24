//! フロッピーディスク等の複数イメージを「1つの論理アーカイブ」としてまとめる仕組み
//!
//! 用途: `Dennou074A.img` `Dennou074B.img` `Dennou074X.img` のような同一号の
//! 複数枚フロッピーを 1 サマリにまとめる (コスト削減 + セマンティクスの向上)。
//!
//! 設定ファイル `xdf-grouping.toml` で挙動をカスタマイズ可能。
//!
//! ```toml
//! solo_extensions = ["hds", "hdf", "dim", "d88"]
//!
//! [[rule]]
//! name = "電脳倶楽部 (vol番号 + 面記号 A/B/X)"
//! pattern = '^(Dennou\d+)[A-Z]\.(?:img|xdf)$'
//!
//! [[rule]]
//! name = "汎用フロッピー (末尾1文字を面記号として剥ぐ)"
//! pattern = '^(.+?)[A-Da-dXx]?\.(?:xdf|img|2hd)$'
//!
//! [[manual_group]]
//! id = "Special_Anniversary_Set"
//! members = ["DennouSpecial1.img", "DennouSpecial2.img"]
//! ```
//!
//! 解決アルゴリズム (各 image_path について):
//!   1. 拡張子が `solo_extensions` に該当 → 単独
//!   2. ファイル名 (basename) が `manual_group` の members に含まれる → そのグループ
//!   3. `rule` を上から順にテスト → 最初にマッチしたルールの capture[1] を group_key
//!   4. どれにも該当しない → 単独

use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `xdf-grouping.toml` の生表現
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GroupConfigRaw {
    #[serde(default)]
    pub solo_extensions: Vec<String>,
    #[serde(default, rename = "rule")]
    pub rules: Vec<RuleRaw>,
    #[serde(default, rename = "manual_group")]
    pub manual_groups: Vec<ManualGroupRaw>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuleRaw {
    #[serde(default)]
    pub name: String,
    pub pattern: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManualGroupRaw {
    pub id: String,
    pub members: Vec<String>,
}

/// コンパイル済み設定 (ホットパスで再利用)
#[derive(Debug)]
pub struct GroupConfig {
    pub solo_extensions: Vec<String>,
    pub rules: Vec<CompiledRule>,
    pub manual_groups: Vec<ManualGroupRaw>,
}

#[derive(Debug)]
pub struct CompiledRule {
    pub name: String,
    pub regex: Regex,
}

impl GroupConfig {
    /// 規則ゼロのデフォルト (= 全イメージが単独)
    pub fn empty() -> Self {
        Self {
            solo_extensions: vec![],
            rules: vec![],
            manual_groups: vec![],
        }
    }

    /// TOML ファイルから読み込み + 正規表現コンパイル
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read grouping config {:?}", path))?;
        Self::from_toml(&bytes)
    }

    /// TOML 文字列から構築
    pub fn from_toml(s: &str) -> Result<Self> {
        let raw: GroupConfigRaw =
            toml::from_str(s).context("Cannot parse grouping config as TOML")?;
        let mut rules = Vec::with_capacity(raw.rules.len());
        for r in raw.rules {
            let regex = Regex::new(&r.pattern)
                .with_context(|| format!("Invalid regex in rule {:?}: {}", r.name, r.pattern))?;
            if regex.captures_len() < 2 {
                return Err(anyhow!(
                    "Rule {:?} has no capture group (need at least one '(...)')",
                    r.name
                ));
            }
            rules.push(CompiledRule {
                name: r.name,
                regex,
            });
        }
        Ok(Self {
            solo_extensions: raw
                .solo_extensions
                .into_iter()
                .map(|e| e.to_ascii_lowercase())
                .collect(),
            rules,
            manual_groups: raw.manual_groups,
        })
    }
}

/// 解決済みグループ (1〜N 枚のディスクイメージを束ねる)
#[derive(Debug, Clone)]
pub struct ResolvedGroup {
    /// グループ識別子
    /// - 単独: そのイメージの image_id
    /// - 複数: メンバ image_id をソート連結 → sha1[..16]
    /// - manual: ユーザ指定 ID
    pub id: String,
    /// マッチした規則名 (デバッグ用、`solo` / `manual:<id>` / `rule:<name>` / `default-solo`)
    pub origin: String,
    /// メンバ (1 枚 or 複数枚)
    pub members: Vec<PathBuf>,
}

impl ResolvedGroup {
    pub fn is_solo(&self) -> bool {
        self.members.len() == 1
    }
}

/// イメージ群を設定にしたがってグルーピング。順序は安定 (group key の昇順)。
///
/// `image_id_resolver` は image_path → image_id に変換するクロージャ。
/// テストでは固定値、本番では `compute_image_id` を渡す。
pub fn resolve_groups<F>(
    images: &[PathBuf],
    cfg: &GroupConfig,
    image_id_resolver: F,
) -> Result<Vec<ResolvedGroup>>
where
    F: Fn(&Path) -> Result<String>,
{
    // basename → image_path の逆引き (manual_group のメンバ照合用)
    let mut by_basename: BTreeMap<String, &PathBuf> = BTreeMap::new();
    for img in images {
        if let Some(name) = img.file_name().and_then(|s| s.to_str()) {
            by_basename.insert(name.to_string(), img);
        }
    }

    // どのイメージがどのグループに属するかの引き当て
    // group_key → (origin, Vec<PathBuf>)
    let mut buckets: BTreeMap<String, (String, Vec<PathBuf>)> = BTreeMap::new();
    let mut consumed: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    // (1) manual_group を最優先で確定
    for mg in &cfg.manual_groups {
        let mut members = Vec::new();
        for name in &mg.members {
            if let Some(path) = by_basename.get(name) {
                members.push((*path).clone());
                consumed.insert((*path).clone());
            }
        }
        if !members.is_empty() {
            members.sort();
            let key = format!("manual:{}", mg.id);
            buckets.insert(key, (format!("manual:{}", mg.id), members));
        }
    }

    // (2) 残りを solo / rule で振り分け
    for img in images {
        if consumed.contains(img) {
            continue;
        }
        let name = img
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let ext = img
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if cfg.solo_extensions.iter().any(|e| e == &ext) {
            // solo 確定 → image_id ベースのキー (重複しないよう "solo:" 接頭)
            let key = format!("solo:{}", img.display());
            buckets.insert(key, ("solo".to_string(), vec![img.clone()]));
            continue;
        }

        let mut matched = false;
        for rule in &cfg.rules {
            if let Some(caps) = rule.regex.captures(name) {
                if let Some(group_key) = caps.get(1).map(|m| m.as_str().to_string()) {
                    let bucket_key = format!("rule:{}", group_key);
                    let entry = buckets
                        .entry(bucket_key)
                        .or_insert_with(|| (format!("rule:{}", rule.name), Vec::new()));
                    entry.1.push(img.clone());
                    matched = true;
                    break;
                }
            }
        }

        if !matched {
            let key = format!("default-solo:{}", img.display());
            buckets.insert(
                key,
                ("default-solo".to_string(), vec![img.clone()]),
            );
        }
    }

    // (3) 各 bucket を ResolvedGroup に変換 (member sort, group_id 計算)
    let mut groups: Vec<ResolvedGroup> = Vec::with_capacity(buckets.len());
    for (_key, (origin, mut members)) in buckets {
        members.sort();
        // 単独メンバ: image_id がそのままグループID
        // 複数メンバ: 各 image_id をソート連結 → sha1[..16]
        let id = if members.len() == 1 {
            image_id_resolver(&members[0])?
        } else {
            let mut ids = Vec::with_capacity(members.len());
            for m in &members {
                ids.push(image_id_resolver(m)?);
            }
            ids.sort();
            let joined = ids.join(":");
            let mut hasher = Sha1::new();
            hasher.update(joined.as_bytes());
            let digest = hasher.finalize();
            digest
                .iter()
                .take(8)
                .fold(String::with_capacity(16), |mut acc, b| {
                    acc.push_str(&format!("{:02x}", b));
                    acc
                })
        };
        groups.push(ResolvedGroup {
            id,
            origin,
            members,
        });
    }

    // 安定順序: メンバの先頭パス昇順
    groups.sort_by(|a, b| a.members[0].cmp(&b.members[0]));
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_id(path: &Path) -> Result<String> {
        // ファイル名そのまま (テスト用)
        Ok(path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string())
    }

    #[test]
    fn empty_config_makes_everything_solo() {
        let cfg = GroupConfig::empty();
        let images = vec![
            PathBuf::from("/a/Dennou074A.img"),
            PathBuf::from("/a/Dennou074B.img"),
            PathBuf::from("/a/SCSIHDD1.HDS"),
        ];
        let groups = resolve_groups(&images, &cfg, fake_id).unwrap();
        assert_eq!(groups.len(), 3);
        assert!(groups.iter().all(|g| g.is_solo()));
    }

    #[test]
    fn solo_extensions_isolate_hdds() {
        let cfg = GroupConfig::from_toml(
            r#"
            solo_extensions = ["hds", "hdf"]
            [[rule]]
            name = "any-floppy"
            pattern = '^(.+?)[A-Z]?\.(?:img|xdf)$'
            "#,
        )
        .unwrap();
        let images = vec![
            PathBuf::from("/a/Dennou074A.img"),
            PathBuf::from("/a/Dennou074B.img"),
            PathBuf::from("/a/SCSIHDD1.HDS"),
            PathBuf::from("/a/sub.HDF"),
        ];
        let groups = resolve_groups(&images, &cfg, fake_id).unwrap();
        // Dennou074{A,B}: 1 group; SCSIHDD1: solo; sub: solo
        assert_eq!(groups.len(), 3);
        let dennou = groups
            .iter()
            .find(|g| g.members.len() == 2)
            .expect("Dennou group should have 2 members");
        assert_eq!(
            dennou
                .members
                .iter()
                .map(|m| m.file_name().unwrap().to_str().unwrap().to_string())
                .collect::<Vec<_>>(),
            vec!["Dennou074A.img", "Dennou074B.img"]
        );
    }

    #[test]
    fn dennou_specific_rule_with_x_suffix() {
        // ユーザの実例: Dennou074A.img / Dennou074B.img / Dennou074X.img
        let cfg = GroupConfig::from_toml(
            r#"
            [[rule]]
            name = "dennou"
            pattern = '^(Dennou\d+)[A-Z]\.(?:img|xdf)$'
            "#,
        )
        .unwrap();
        let images = vec![
            PathBuf::from("/a/Dennou074A.img"),
            PathBuf::from("/a/Dennou074B.img"),
            PathBuf::from("/a/Dennou074X.img"),
        ];
        let groups = resolve_groups(&images, &cfg, fake_id).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 3);
    }

    #[test]
    fn manual_group_overrides_rule() {
        let cfg = GroupConfig::from_toml(
            r#"
            [[rule]]
            name = "any"
            pattern = '^(.+?)[A-Z]?\.img$'
            [[manual_group]]
            id = "anniversary"
            members = ["Special1.img", "Special2.img"]
            "#,
        )
        .unwrap();
        let images = vec![
            PathBuf::from("/a/Special1.img"),
            PathBuf::from("/a/Special2.img"),
            PathBuf::from("/a/Other.img"),
        ];
        let groups = resolve_groups(&images, &cfg, fake_id).unwrap();
        // manual: Special{1,2} (2 members), default-solo: Other (1 member)
        assert_eq!(groups.len(), 2);
        let manual = groups.iter().find(|g| g.origin.starts_with("manual:")).unwrap();
        assert_eq!(manual.members.len(), 2);
    }

    #[test]
    fn no_match_falls_back_to_solo() {
        let cfg = GroupConfig::from_toml(
            r#"
            [[rule]]
            name = "dennou-only"
            pattern = '^(Dennou\d+)[A-Z]\.img$'
            "#,
        )
        .unwrap();
        let images = vec![PathBuf::from("/a/RandomFloppy.xdf")];
        let groups = resolve_groups(&images, &cfg, fake_id).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].origin, "default-solo");
    }

    #[test]
    fn group_id_stable_under_member_reorder() {
        let cfg = GroupConfig::from_toml(
            r#"
            [[rule]]
            name = "any"
            pattern = '^(.+?)[A-Z]\.img$'
            "#,
        )
        .unwrap();
        let images_a = vec![
            PathBuf::from("/a/FooA.img"),
            PathBuf::from("/a/FooB.img"),
        ];
        let images_b = vec![
            PathBuf::from("/a/FooB.img"),
            PathBuf::from("/a/FooA.img"),
        ];
        let g_a = resolve_groups(&images_a, &cfg, fake_id).unwrap();
        let g_b = resolve_groups(&images_b, &cfg, fake_id).unwrap();
        assert_eq!(g_a[0].id, g_b[0].id);
    }

    #[test]
    fn rule_without_capture_is_rejected() {
        let err = GroupConfig::from_toml(
            r#"
            [[rule]]
            name = "bad"
            pattern = '^Dennou\d+\.img$'
            "#,
        );
        assert!(err.is_err());
    }
}
