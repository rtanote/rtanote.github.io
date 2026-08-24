//! 実イメージ (XDF フロッピー + HDS/HDF) を使った統合テスト
//!
//! これらのテストは著作権物のディスクイメージを必要とするため、すべて
//! `#[ignore]` が付いている。通常の `cargo test` では実行されない。
//!
//! 実行するには tests/data/ にイメージを配置したうえで:
//!   cargo test --workspace -- --include-ignored
//!
//! データが無い状態で `--include-ignored` を付けた場合は、
//! 必要なファイル名を挙げて明確に失敗する (黙って成功しない)。

use std::path::PathBuf;
use xdf_fs::bpb::Bpb;
use xdf_fs::direntry::Attr;
use xdf_fs::fat::FatKind;
use xdf_fs::fs::Filesystem;
use xdf_fs::hdd::HddImage;
use xdf_fs::image::{DiskImage, XdfImage};
use xdf_fs::partition::{PartitionTable, PARTITION_TABLE_SECTOR};

/// ワークスペース直下の tests/data/ ディレクトリを返す。
/// workspace 化に伴い CARGO_MANIFEST_DIR は crates/xdf-fs/ を指すので
/// 2階層上 (`../../tests/data`) を参照する。
fn workspace_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/data")
}

/// tests/data/<name> を返す。無ければテストを失敗させる。
///
/// 呼び出し側のテストは `#[ignore]` 済みなので、`--include-ignored` を
/// 明示した場合にのみここへ到達する。その状況でデータが無いのは
/// 「黙って skip」ではなく明確なエラーとして扱う。
fn data_path(name: &str) -> PathBuf {
    let p = workspace_data_dir().join(name);
    assert!(
        p.exists(),
        "テストフィクスチャがありません: {}
         著作権物のため配布していません。README の「テストデータ配置」を参照して          tests/data/ に配置してください。",
        p.display()
    );
    p
}

fn test_image_path() -> PathBuf {
    data_path("Dennou074A.img")
}

/// パーティションテーブルが置かれている物理セクタの内容を読み出す
fn read_partition_table_bytes(path: &std::path::Path, phys_sector_size: usize) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    let off = phys_sector_size * PARTITION_TABLE_SECTOR;
    // 1論理セクタ分 (1024B) を読む。物理セクタが小さい形式 (HDF=256B) では
    // 複数の物理セクタにまたがるが、実際のテーブル本体は先頭 256 B 程度。
    bytes[off..off + 1024].to_vec()
}

#[test]
#[ignore = "requires tests/data/Dennou074A.img (著作権物のため非配布)"]
fn open_and_read_bpb() {
    let path = test_image_path();
    let img = XdfImage::open(&path).unwrap();
    let fs = Filesystem::open(&img).unwrap();
    assert_eq!(fs.bpb.bytes_per_sector, 1024);
    assert_eq!(fs.bpb.sectors_per_cluster, 1);
    assert_eq!(fs.bpb.num_fats, 2);
    assert_eq!(fs.bpb.root_entries, 192);
    assert_eq!(fs.bpb.total_sectors, 1232);
    assert_eq!(fs.phys_per_logi, 2);
}

#[test]
#[ignore = "requires tests/data/Dennou074A.img (著作権物のため非配布)"]
fn root_directory_has_expected_entries() {
    let path = test_image_path();
    let img = XdfImage::open(&path).unwrap();
    let fs = Filesystem::open(&img).unwrap();
    let root = fs.read_root_dir().unwrap();
    let names: Vec<String> = root.iter().map(|e| e.display_name()).collect();
    assert!(names.iter().any(|n| n == "MUSIC"));
    assert!(names.iter().any(|n| n == "BIN"));
    assert!(names.iter().any(|n| n == "COMMAND.X"));
    assert!(names.iter().any(|n| n == "HUMAN.SYS"));
}

#[test]
#[ignore = "requires tests/data/Dennou074A.img (著作権物のため非配布)"]
fn volume_label_decodes_full_width_sjis() {
    let path = test_image_path();
    let img = XdfImage::open(&path).unwrap();
    let fs = Filesystem::open(&img).unwrap();
    let root = fs.read_root_dir().unwrap();
    let vol = root
        .iter()
        .find(|e| e.attr.contains(Attr::VOLUME))
        .expect("volume label entry should exist");
    // 期待値: "電脳倶楽部７４" (全角)
    assert_eq!(vol.display_name(), "電脳倶楽部７４");
}

#[test]
#[ignore = "requires tests/data/Dennou074A.img (著作権物のため非配布)"]
fn resolve_nested_path() {
    let path = test_image_path();
    let img = XdfImage::open(&path).unwrap();
    let fs = Filesystem::open(&img).unwrap();
    let zms = fs.resolve("MUSIC/STRANGE.ZMS").unwrap();
    assert_eq!(zms.size, 11151);
}

#[test]
#[ignore = "requires tests/data/Dennou074A.img (著作権物のため非配布)"]
fn case_insensitive_path_resolution() {
    let path = test_image_path();
    let img = XdfImage::open(&path).unwrap();
    let fs = Filesystem::open(&img).unwrap();
    // 元は MUSIC/STRANGE.ZMS だが小文字でも引ける
    let zms = fs.resolve("music/strange.zms").unwrap();
    assert_eq!(zms.size, 11151);
}

#[test]
#[ignore = "requires tests/data/Dennou074A.img (著作権物のため非配布)"]
fn read_file_content() {
    let path = test_image_path();
    let img = XdfImage::open(&path).unwrap();
    let fs = Filesystem::open(&img).unwrap();
    let zms = fs.resolve("MUSIC/STRANGE.ZMS").unwrap();
    let data = fs.read_file(&zms).unwrap();
    assert_eq!(data.len(), 11151);
    // ZMS ファイルは先頭付近に ".comment" or ".COMMENT" を含む (Z-MUSIC源コード規約)
    let head = &data[..data.len().min(64)];
    let head_lower = head.iter().map(|b| b.to_ascii_lowercase()).collect::<Vec<_>>();
    assert!(
        head_lower.windows(8).any(|w| w == b".comment"),
        "ZMS source should contain .comment near start, got: {:?}",
        std::str::from_utf8(head).ok(),
    );
}

#[test]
#[ignore = "requires tests/data/Dennou074A.img (著作権物のため非配布)"]
fn list_subdirectory() {
    let path = test_image_path();
    let img = XdfImage::open(&path).unwrap();
    let fs = Filesystem::open(&img).unwrap();
    let entries = fs.list_path("MUSIC").unwrap();
    let names: Vec<String> = entries.iter().map(|e| e.display_name()).collect();
    assert!(names.iter().any(|n| n == "STRANGE.ZMS"));
    assert!(names.iter().any(|n| n == "STRANGE.DOC"));
    assert!(names.iter().any(|n| n == "STRANGE.ZPD"));
}

#[test]
#[ignore = "requires tests/data/Dennou074A.img (著作権物のため非配布)"]
fn walker_visits_all_files() {
    let path = test_image_path();
    let img = XdfImage::open(&path).unwrap();
    let fs = Filesystem::open(&img).unwrap();
    let mut paths = Vec::new();
    xdf_fs::walker::walk(&fs, |item| {
        paths.push(item.path.clone());
        true
    }).unwrap();
    assert!(paths.iter().any(|p| p == "MUSIC/STRANGE.ZMS"));
    // BIN 内に zmusic 系ファイル (大文字小文字混在しうる)
    assert!(paths.iter().any(|p| p.to_ascii_uppercase() == "BIN/ZMUSIC.X"));
    // 多層: IKAP配下のサブディレクトリ
    assert!(paths.iter().any(|p| p.starts_with("IKAP/")));
}

// ---- HDS / HDF パーティションテーブル統合テスト (Phase 2) ----

#[test]
#[ignore = "requires tests/data/SCSIHDD1.HDS (著作権物のため非配布)"]
fn hds_partition_table_parses() {
    let path = data_path("SCSIHDD1.HDS");
    let buf = read_partition_table_bytes(&path, 512); // HDS: 512B physical
    let pt = PartitionTable::parse(&buf).unwrap();
    assert_eq!(pt.partitions.len(), 1, "expected single Human68k partition");
    let p = &pt.partitions[0];
    assert_eq!(p.name, "Human68k");
    assert_eq!(p.start_sector, 32);
    assert_eq!(p.length_sectors, 920_576);
    // 1024 B unit でディスク容量と整合
    let total_bytes = p.start_bytes(1024) + p.length_bytes(1024);
    assert!(total_bytes <= std::fs::metadata(&path).unwrap().len());
}

#[test]
#[ignore = "requires tests/data/hd1.hdf (著作権物のため非配布)"]
fn hdf_partition_table_parses() {
    let path = data_path("hd1.hdf");
    let buf = read_partition_table_bytes(&path, 256); // HDF: 256B physical
    let pt = PartitionTable::parse(&buf).unwrap();
    assert_eq!(pt.partitions.len(), 1, "expected single Human68k partition");
    let p = &pt.partitions[0];
    assert_eq!(p.name, "Human68k");
    assert_eq!(p.start_sector, 33);
    assert_eq!(p.length_sectors, 162_040);
    // 256 B unit でディスク容量と整合
    let total_bytes = p.start_bytes(256) + p.length_bytes(256);
    assert!(total_bytes <= std::fs::metadata(&path).unwrap().len());
}

#[test]
#[ignore = "requires tests/data/SCSIHDD1.HDS (著作権物のため非配布)"]
fn hds_partition_bpb_parses() {
    let path = data_path("SCSIHDD1.HDS");
    let bytes = std::fs::read(&path).unwrap();
    let table_buf = &bytes[512 * PARTITION_TABLE_SECTOR..512 * PARTITION_TABLE_SECTOR + 1024];
    let pt = PartitionTable::parse(table_buf).unwrap();
    let p = &pt.partitions[0];
    // パーティション開始位置 = start_sector * 1024 (HDS は 1024B 単位)
    let bpb_off = p.start_bytes(1024) as usize;
    let bpb_buf = &bytes[bpb_off..bpb_off + 64];
    let bpb = Bpb::parse_hdd(bpb_buf).unwrap();
    assert_eq!(bpb.bytes_per_sector, 1024);
    assert_eq!(bpb.sectors_per_cluster, 16);
    assert_eq!(bpb.num_fats, 2);
    assert_eq!(bpb.reserved_sectors, 1);
    assert_eq!(bpb.total_sectors, 920_576);
    assert_eq!(bpb.sectors_per_fat, 114);
}

#[test]
#[ignore = "requires tests/data/hd1.hdf (著作権物のため非配布)"]
fn hdf_partition_bpb_parses() {
    let path = data_path("hd1.hdf");
    let bytes = std::fs::read(&path).unwrap();
    let table_buf = &bytes[256 * PARTITION_TABLE_SECTOR..256 * PARTITION_TABLE_SECTOR + 1024];
    let pt = PartitionTable::parse(table_buf).unwrap();
    let p = &pt.partitions[0];
    // パーティション開始位置 = start_sector * 256 (HDF は 256B 単位)
    let bpb_off = p.start_bytes(256) as usize;
    let bpb_buf = &bytes[bpb_off..bpb_off + 64];
    let bpb = Bpb::parse_hdd(bpb_buf).unwrap();
    assert_eq!(bpb.bytes_per_sector, 1024);
    assert_eq!(bpb.sectors_per_cluster, 1);
    assert_eq!(bpb.num_fats, 2);
    assert_eq!(bpb.reserved_sectors, 1);
    assert_eq!(bpb.total_sectors, 40_510);
    assert_eq!(bpb.sectors_per_fat, 80);
}

// ---- HddImage end-to-end (T-2.4) ----

#[test]
#[ignore = "requires tests/data/SCSIHDD1.HDS (著作権物のため非配布)"]
fn hds_open_and_list_partitions() {
    let path = data_path("SCSIHDD1.HDS");
    let hdd = HddImage::open_hds(&path).unwrap();
    assert_eq!(hdd.phys_sec_size(), 512);
    let parts = hdd.partitions();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].name, "Human68k");
    // start = 32 * 1024B / 512B = 64 物理セクタ
    assert_eq!(parts[0].phys_start, 64);
    // length = 920_576 * 1024B / 512B = 1_841_152 物理セクタ
    assert_eq!(parts[0].phys_count, 1_841_152);
}

#[test]
#[ignore = "requires tests/data/hd1.hdf (著作権物のため非配布)"]
fn hdf_open_and_list_partitions() {
    let path = data_path("hd1.hdf");
    let hdd = HddImage::open_hdf(&path).unwrap();
    assert_eq!(hdd.phys_sec_size(), 256);
    let parts = hdd.partitions();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].name, "Human68k");
    // HDF: 単位=256B = phys_sec_size、なので倍数1
    assert_eq!(parts[0].phys_start, 33);
    assert_eq!(parts[0].phys_count, 162_040);
}

#[test]
#[ignore = "requires tests/data/SCSIHDD1.HDS (著作権物のため非配布)"]
fn hds_partition_image_reads_correct_bpb() {
    let path = data_path("SCSIHDD1.HDS");
    let hdd = HddImage::open_hds(&path).unwrap();
    let part = hdd.partition(0).unwrap();
    // sector 0 of partition は HDD BPB を含む
    let boot = part.read_sector(0).unwrap();
    let bpb = Bpb::parse_hdd(boot).unwrap();
    assert_eq!(bpb.bytes_per_sector, 1024);
    assert_eq!(bpb.sectors_per_cluster, 16);
    assert_eq!(bpb.total_sectors, 920_576);
}

#[test]
#[ignore = "requires tests/data/SCSIHDD1.HDS (著作権物のため非配布)"]
fn hds_filesystem_mounts_and_lists_root() {
    let path = data_path("SCSIHDD1.HDS");
    let hdd = HddImage::open_hds(&path).unwrap();
    let part = hdd.partition(0).unwrap();
    let boot = part.read_sector(0).unwrap();
    let bpb = Bpb::parse_hdd(boot).unwrap();
    let fs = Filesystem::open_with_bpb(&part, bpb).unwrap();
    // 大容量ディスク → FAT16 自動選択
    assert_eq!(fs.fat.kind(), FatKind::Fat16);
    let root = fs.read_root_dir().unwrap();
    // Human68k FS なので何かしらルートエントリは存在するはず
    // (この HDS の中身に依存するので件数だけ確認)
    println!("HDS root: {} entries", root.len());
    assert!(!root.is_empty(), "expected at least one root entry");
}

#[test]
#[ignore = "requires tests/data/hd1.hdf (著作権物のため非配布)"]
fn hdf_filesystem_mounts_and_lists_root() {
    let path = data_path("hd1.hdf");
    let hdd = HddImage::open_hdf(&path).unwrap();
    let part = hdd.partition(0).unwrap();
    let boot = part.read_sector(0).unwrap();
    let bpb = Bpb::parse_hdd(boot).unwrap();
    let fs = Filesystem::open_with_bpb(&part, bpb).unwrap();
    assert_eq!(fs.fat.kind(), FatKind::Fat16);
    let root = fs.read_root_dir().unwrap();
    println!("HDF root: {} entries", root.len());
    assert!(!root.is_empty(), "expected at least one root entry");
    // ボリュームラベル(あれば)を確認
    let vol = root.iter().find(|e| e.attr.contains(Attr::VOLUME));
    if let Some(v) = vol {
        println!("HDF volume label: {:?}", v.display_name());
    }
}
