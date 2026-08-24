//! xdf-index 統合テスト (build → search → 増分の一気通貫)
//!
//! これらのテストは著作権物のディスクイメージを必要とするため、すべて
//! `#[ignore]` が付いている。通常の `cargo test` では実行されない。
//!
//! 実行するには tests/data/ にイメージを配置したうえで:
//!   cargo test --workspace -- --include-ignored

use std::path::PathBuf;
use tempfile::TempDir;
use xdf_index::builder::{build_index, BuildOpts};
use xdf_index::searcher::Searcher;

fn workspace_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data")
}

/// tests/data/ にイメージが1つ以上あることを要求する。
///
/// 呼び出し側は `#[ignore]` 済みなので、`--include-ignored` を明示した
/// 場合にのみここへ到達する。データが無いのは明確なエラーとして扱う。
fn require_samples() {
    let d = workspace_data_dir();
    let any = d.join("Dennou074A.img").exists()
        || d.join("SCSIHDD1.HDS").exists()
        || d.join("hd1.hdf").exists();
    assert!(
        any,
        "テストフィクスチャがありません: {}
         著作権物のため配布していません。README の「テストデータ配置」を参照して          tests/data/ に配置してください。",
        d.display()
    );
}

/// 特定のイメージを要求する。
fn require_sample(name: &str) {
    let p = workspace_data_dir().join(name);
    assert!(
        p.exists(),
        "テストフィクスチャがありません: {}
         著作権物のため配布していません。README の「テストデータ配置」を参照して          tests/data/ に配置してください。",
        p.display()
    );
}

#[test]
#[ignore = "requires tests/data/ のディスクイメージ (著作権物のため非配布)"]
fn build_then_search_round_trip() {
    require_samples();
    let archive = workspace_data_dir();
    let tmp = TempDir::new().unwrap();
    let stats = build_index(&archive, tmp.path(), BuildOpts::default()).unwrap();
    assert!(stats.images_indexed >= 1, "at least 1 image should be indexed");
    assert!(stats.files_indexed > 0, "at least 1 file should be indexed");

    let searcher = Searcher::open(tmp.path()).unwrap();
    assert_eq!(searcher.doc_count() as usize, stats.files_indexed);

    // ファイル名検索 (file_name フィールドが対象)
    let hits = searcher.search("STRANGE", 10).unwrap();
    if archive.join("Dennou074A.img").exists() {
        assert!(
            hits.iter().any(|h| h.file_name.contains("STRANGE")),
            "expected to find STRANGE.* from Dennou074A"
        );
    }
}

#[test]
#[ignore = "requires tests/data/ のディスクイメージ (著作権物のため非配布)"]
fn incremental_build_skips_existing() {
    require_samples();
    let archive = workspace_data_dir();
    let tmp = TempDir::new().unwrap();

    // 1回目: 全イメージ index 化
    let first = build_index(&archive, tmp.path(), BuildOpts::default()).unwrap();
    assert!(first.images_indexed >= 1);

    // 2回目: 同じディレクトリで再ビルド (増分) → すべて skip されるはず
    let second = build_index(&archive, tmp.path(), BuildOpts::default()).unwrap();
    assert_eq!(second.images_indexed, 0);
    assert_eq!(second.images_skipped, first.images_indexed);
}

#[test]
#[ignore = "requires tests/data/ のディスクイメージ (著作権物のため非配布)"]
fn fresh_rebuild_reindexes_all() {
    require_samples();
    let archive = workspace_data_dir();
    let tmp = TempDir::new().unwrap();

    let first = build_index(&archive, tmp.path(), BuildOpts::default()).unwrap();

    let stats = build_index(
        &archive,
        tmp.path(),
        BuildOpts {
            fresh: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.images_indexed, first.images_indexed);
    assert_eq!(stats.images_skipped, 0);
}

#[test]
#[ignore = "requires tests/data/SCSIHDD1.HDS (著作権物のため非配布)"]
fn search_returns_excerpt_and_path() {
    require_sample("SCSIHDD1.HDS");
    let archive = workspace_data_dir();
    let tmp = TempDir::new().unwrap();
    build_index(&archive, tmp.path(), BuildOpts::default()).unwrap();

    let searcher = Searcher::open(tmp.path()).unwrap();
    // HDS の Z-MUSIC ディレクトリ配下に SAMPLE.DOC が複数あることを確認済み
    let hits = searcher.search("SAMPLE", 5).unwrap();
    assert!(!hits.is_empty(), "expected hits for 'SAMPLE'");
    let first = &hits[0];
    assert!(!first.image_path.is_empty());
    assert!(!first.file_path.is_empty());
    // 出典は image_id:partition:path の3点組
    assert_eq!(first.image_id.len(), 16, "image_id is sha1[..16]");
}

#[test]
#[ignore = "requires tests/data/SCSIHDD1.HDS (著作権物のため非配布)"]
fn fat16_partition_files_appear_in_index() {
    require_sample("SCSIHDD1.HDS");
    let archive = workspace_data_dir();
    let tmp = TempDir::new().unwrap();
    build_index(&archive, tmp.path(), BuildOpts::default()).unwrap();

    let searcher = Searcher::open(tmp.path()).unwrap();
    // Oh!X HDS には Z-MUSIC ディレクトリが存在
    let hits = searcher.search("Z-MUSIC", 20).unwrap();
    let from_hds = hits
        .iter()
        .any(|h| h.image_path.ends_with("SCSIHDD1.HDS") && h.partition == 0);
    assert!(from_hds, "expected at least one hit from HDS partition 0");
}
