//! FAT12のチェーン追跡ロジックの単体テスト
//! (実イメージなしで動く)

use xdf_fs::bpb::Bpb;
use xdf_fs::image::DiskImage;

/// メモリ上の偽イメージ
struct FakeImage {
    data: Vec<u8>,
}

impl DiskImage for FakeImage {
    fn sector_size(&self) -> usize { 512 }
    fn sector_count(&self) -> usize { self.data.len() / 512 }
    fn read_sector(&self, lba: usize) -> anyhow::Result<&[u8]> {
        Ok(&self.data[lba*512..(lba+1)*512])
    }
}

#[test]
fn bpb_parse_basic() {
    let mut boot = vec![0u8; 512];
    boot[0x0B] = 0x00; boot[0x0C] = 0x04; // bytes_per_sector = 1024
    boot[0x0D] = 1; // sectors_per_cluster
    boot[0x0E] = 1; boot[0x0F] = 0; // reserved = 1
    boot[0x10] = 2; // num_fats
    boot[0x11] = 0xC0; boot[0x12] = 0; // root_entries = 192
    boot[0x13] = 0xD0; boot[0x14] = 0x04; // total_sectors = 1232
    boot[0x16] = 2; boot[0x17] = 0; // sectors_per_fat = 2
    boot[0x18] = 8; boot[0x19] = 0; // sectors_per_track = 8
    boot[0x1A] = 2; boot[0x1B] = 0; // num_heads = 2

    let bpb = Bpb::parse(&boot).unwrap();
    assert_eq!(bpb.bytes_per_sector, 1024);
    assert_eq!(bpb.sectors_per_cluster, 1);
    assert_eq!(bpb.reserved_sectors, 1);
    assert_eq!(bpb.num_fats, 2);
    assert_eq!(bpb.root_entries, 192);
    assert_eq!(bpb.total_sectors, 1232);
    assert_eq!(bpb.sectors_per_fat, 2);

    // 領域計算
    assert_eq!(bpb.fat_start(), 1);
    assert_eq!(bpb.root_dir_start(), 5); // 1 + 2*2
    assert_eq!(bpb.root_dir_sectors(), 6); // 192*32/1024 = 6
    assert_eq!(bpb.data_start(), 11);
    assert_eq!(bpb.cluster_to_sector(2), 11);
    assert_eq!(bpb.cluster_to_sector(3), 12);
}

#[test]
fn direntry_parse_basic() {
    use xdf_fs::direntry::{DirEntry, Attr};
    let mut e = vec![0u8; 32];
    // "HELLO   " + "X  " 通常エントリ
    e[0..8].copy_from_slice(b"HELLO   ");
    e[8..11].copy_from_slice(b"X  ");
    e[0x0B] = 0x20; // ARCHIVE
    e[0x1A] = 2; // start_cluster = 2
    e[0x1C] = 100; e[0x1D] = 0; e[0x1E] = 0; e[0x1F] = 0; // size=100
    let parsed = DirEntry::parse(&e).unwrap();
    assert_eq!(parsed.name, "HELLO");
    assert_eq!(parsed.ext, "X");
    assert_eq!(parsed.start_cluster, 2);
    assert_eq!(parsed.size, 100);
    assert_eq!(parsed.attr, Attr::ARCHIVE);
    assert_eq!(parsed.display_name(), "HELLO.X");
}

#[test]
fn direntry_parse_long_name_with_extension_area() {
    // 8+9 バイトの主名 (Human68k拡張領域使用)
    use xdf_fs::direntry::DirEntry;
    let mut e = vec![0u8; 32];
    e[0..8].copy_from_slice(b"LONGNAME");
    e[0x08..0x0B].copy_from_slice(b"DAT");
    // 0x0C-0x14 (9バイト) の拡張領域に "EXTRABYTE"
    e[0x0C..0x15].copy_from_slice(b"EXTRABYTE");
    e[0x0B] = 0x20;
    let parsed = DirEntry::parse(&e).unwrap();
    assert_eq!(parsed.name, "LONGNAMEEXTRABYTE");
    assert_eq!(parsed.ext, "DAT");
}

#[test]
fn direntry_parse_deleted() {
    use xdf_fs::direntry::{DirEntry, EntryKind};
    let mut e = vec![0u8; 32];
    e[0..8].copy_from_slice(b"\xE5ELETED ");
    e[0x08..0x0B].copy_from_slice(b"BAK");
    let parsed = DirEntry::parse(&e).unwrap();
    assert_eq!(parsed.kind, EntryKind::Deleted);
    assert_eq!(parsed.name, "?ELETED"); // 先頭が ? に置換
}

#[test]
fn direntry_parse_empty() {
    use xdf_fs::direntry::{DirEntry, EntryKind};
    let e = vec![0u8; 32];
    let parsed = DirEntry::parse(&e).unwrap();
    assert_eq!(parsed.kind, EntryKind::Empty);
}

#[test]
fn direntry_date_time_decoding() {
    use xdf_fs::direntry::DirEntry;
    let mut e = vec![0u8; 32];
    e[0..8].copy_from_slice(b"FOO     ");
    // date = 1998-06-11 → ((1998-1980) << 9) | (6 << 5) | 11 = 0x24CB
    e[0x18] = 0xCB; e[0x19] = 0x24;
    // time = 12:00:00 → (12 << 11) | (0 << 5) | 0 = 0x6000
    e[0x16] = 0x00; e[0x17] = 0x60;
    let parsed = DirEntry::parse(&e).unwrap();
    assert_eq!(parsed.date(), (1998, 6, 11));
    assert_eq!(parsed.time(), (12, 0, 0));
}

#[test]
fn fat12_chain_simple() {
    // ダミーBPB: bps=512, spc=1, reserved=1, num_fats=1, sectors_per_fat=1
    // → FAT領域 = セクタ1 のみ
    let mut data = vec![0u8; 512 * 20];
    // ブートセクタ
    data[0x0B] = 0; data[0x0C] = 2; // bytes_per_sector = 512
    data[0x0D] = 1;
    data[0x0E] = 1; data[0x0F] = 0;
    data[0x10] = 1;
    data[0x11] = 16; data[0x12] = 0;
    data[0x13] = 20; data[0x14] = 0;
    data[0x16] = 1; data[0x17] = 0;

    // FAT12: クラスタ2 → 3 → 4 → 終端 (0xFFF)
    // entry 0,1: 予約 / entry 2 = 3, entry 3 = 4, entry 4 = 0xFFF
    let fat = &mut data[1*512..2*512];
    // 0,1 (予約)
    fat[0] = 0xF8; fat[1] = 0xFF; fat[2] = 0xFF;
    // entry 2 = 0x003, entry 3 = 0x004 (詰めると 03 40 00)
    fat[3] = 0x03; fat[4] = 0x40; fat[5] = 0x00;
    // entry 4 = 0xFFF, entry 5 = 0x000 (FF 0F 00)
    fat[6] = 0xFF; fat[7] = 0x0F; fat[8] = 0x00;

    let img = FakeImage { data };
    let bpb = Bpb::parse(img.read_sector(0).unwrap()).unwrap();
    let fat = xdf_fs::fat12::Fat12::load(&img, &bpb).unwrap();
    assert_eq!(fat.entry(2), 3);
    assert_eq!(fat.entry(3), 4);
    assert_eq!(fat.entry(4), 0xFFF);
    assert_eq!(fat.chain(2), vec![2, 3, 4]);
}

#[test]
fn export_sanitize_filename() {
    use xdf_fs::export::sanitize_filename;
    assert_eq!(sanitize_filename("HELLO.X"), "HELLO.X");
    assert_eq!(sanitize_filename("a/b\\c:d*e?f"), "a_b_c_d_e_f");
    assert_eq!(sanitize_filename("trailing."), "trailing");
    assert_eq!(sanitize_filename("trailing  "), "trailing");
    assert_eq!(sanitize_filename(""), "_");
    // 全角は通す
    assert_eq!(sanitize_filename("愛する人よ"), "愛する人よ");
}

#[test]
fn export_naive_to_epoch_known_dates() {
    use xdf_fs::direntry::DirEntry;
    use xdf_fs::export::entry_to_filetime;
    let mut e = vec![0u8; 32];
    e[0..8].copy_from_slice(b"FOO     ");
    // 1998-06-11 12:00:00
    e[0x18] = 0xCB; e[0x19] = 0x24;
    e[0x16] = 0x00; e[0x17] = 0x60;
    let parsed = DirEntry::parse(&e).unwrap();
    let ft = entry_to_filetime(&parsed).unwrap();
    // 1998-06-11 12:00 UTC = 897_566_400
    // (Howard Hinnant 'days from civil' アルゴリズムの正しさだけ確認)
    assert_eq!(ft.unix_seconds(), 897_566_400);
}

// ---- Human68k HDD BPB parser tests (Phase 2, T-2.2) ----

/// HDS サンプル (`SCSIHDD1.HDS`) のパーティション先頭 BPB (実 dump from 0x8000)
fn hds_sample_bpb_bytes() -> Vec<u8> {
    vec![
        0x60, 0x24, // jump (BRA.S)
        // OEM (16B): "SHARP/KG    1.00"
        0x53, 0x48, 0x41, 0x52, 0x50, 0x2F, 0x4B, 0x47, 0x20, 0x20, 0x20, 0x20, 0x31, 0x2E, 0x30, 0x30,
        // BPB:
        0x04, 0x00, // 0x12: bytes_per_sector (BE) = 1024
        0x10,       // 0x14: sectors_per_cluster = 16
        0x02,       // 0x15: num_fats = 2
        0x00, 0x01, // 0x16: reserved_sectors (BE) = 1
        0x02, 0x00, // 0x18: root_entries (BE) = 512
        0x00, 0x00, // 0x1A: total_sectors_16 (BE) = 0 (use 32)
        0xF7,       // 0x1C: media
        0x72,       // 0x1D: sectors_per_fat (u8) = 114
        0x00, 0x0E, 0x0C, 0x00, // 0x1E: total_sectors_32 (BE) = 920_576
    ]
}

/// HDF サンプル (`hd1.hdf`) のパーティション先頭 BPB (実 dump from 0x2100)
fn hdf_sample_bpb_bytes() -> Vec<u8> {
    vec![
        0x60, 0x20, // jump (BRA.S)
        // OEM (16B): "Hudson soft 2.00"
        0x48, 0x75, 0x64, 0x73, 0x6F, 0x6E, 0x20, 0x73, 0x6F, 0x66, 0x74, 0x20, 0x32, 0x2E, 0x30, 0x30,
        // BPB:
        0x04, 0x00, // 0x12: bytes_per_sector (BE) = 1024
        0x01,       // 0x14: sectors_per_cluster = 1
        0x02,       // 0x15: num_fats = 2
        0x00, 0x01, // 0x16: reserved_sectors (BE) = 1
        0x02, 0x00, // 0x18: root_entries (BE) = 512
        0x9E, 0x3E, // 0x1A: total_sectors_16 (BE) = 40_510
        0xF8,       // 0x1C: media
        0x50,       // 0x1D: sectors_per_fat (u8) = 80
        0x00, 0x00, 0x00, 0x21, // 0x1E: 未使用 (total_sectors_16 != 0)
    ]
}

#[test]
fn bpb_parse_hdd_hds() {
    use xdf_fs::bpb::Bpb;
    let buf = hds_sample_bpb_bytes();
    let bpb = Bpb::parse_hdd(&buf).unwrap();
    assert_eq!(bpb.bytes_per_sector, 1024);
    assert_eq!(bpb.sectors_per_cluster, 16);
    assert_eq!(bpb.num_fats, 2);
    assert_eq!(bpb.reserved_sectors, 1);
    assert_eq!(bpb.root_entries, 512);
    // total_sectors_16 == 0 → total_sectors_32 を使う
    assert_eq!(bpb.total_sectors, 920_576);
    assert_eq!(bpb.sectors_per_fat, 114);
    // HDD では sectors_per_track / num_heads は意味なし (0)
    assert_eq!(bpb.sectors_per_track, 0);
    assert_eq!(bpb.num_heads, 0);
}

#[test]
fn bpb_parse_hdd_hdf() {
    use xdf_fs::bpb::Bpb;
    let buf = hdf_sample_bpb_bytes();
    let bpb = Bpb::parse_hdd(&buf).unwrap();
    assert_eq!(bpb.bytes_per_sector, 1024);
    assert_eq!(bpb.sectors_per_cluster, 1);
    assert_eq!(bpb.num_fats, 2);
    assert_eq!(bpb.reserved_sectors, 1);
    assert_eq!(bpb.root_entries, 512);
    // total_sectors_16 != 0 → そちらを使う (total_sectors_32 は無視される)
    assert_eq!(bpb.total_sectors, 40_510);
    assert_eq!(bpb.sectors_per_fat, 80);
}

#[test]
fn bpb_parse_hdd_rejects_short_buffer() {
    use xdf_fs::bpb::Bpb;
    let buf = vec![0u8; 16];
    assert!(Bpb::parse_hdd(&buf).is_err());
}

#[test]
fn bpb_parse_hdd_rejects_invalid_bps() {
    use xdf_fs::bpb::Bpb;
    let mut buf = hds_sample_bpb_bytes();
    // bytes_per_sector を不正値に: BE 0x0003 = 3
    buf[0x12] = 0x00;
    buf[0x13] = 0x03;
    assert!(Bpb::parse_hdd(&buf).is_err());
}

#[test]
fn bpb_parse_hdd_rejects_zero_fats() {
    use xdf_fs::bpb::Bpb;
    let mut buf = hds_sample_bpb_bytes();
    buf[0x15] = 0; // num_fats を 0 に
    assert!(Bpb::parse_hdd(&buf).is_err());
}

#[test]
fn bpb_parse_hdd_region_calc_works() {
    use xdf_fs::bpb::Bpb;
    let buf = hdf_sample_bpb_bytes();
    let bpb = Bpb::parse_hdd(&buf).unwrap();
    // 既存の領域計算メソッドが新パーサ後も使えることを確認
    assert_eq!(bpb.fat_start(), 1); // reserved_sectors = 1
    assert_eq!(bpb.root_dir_start(), 1 + 2 * 80); // + num_fats(2) * sectors_per_fat(80)
    // root_dir_sectors = 512 entries * 32 / 1024 bps = 16
    assert_eq!(bpb.root_dir_sectors(), 16);
    assert_eq!(bpb.data_start(), 1 + 2 * 80 + 16);
}

#[test]
fn parse_floppy_falls_back_to_default_for_hudson_dim() {
    use xdf_fs::bpb::Bpb;
    // Hudson DIM-style boot sector: 2-byte BRA + 16-byte OEM "Hudson soft 2.00"
    // followed by garbage that the standard BPB parser would reject.
    // 実サンプル (Dennou083B.img の先頭 32 バイト) を再現:
    let boot: Vec<u8> = vec![
        0x60, 0x1c, // BRA.S 0x1C
        0x48, 0x75, 0x64, 0x73, 0x6f, 0x6e, 0x20, 0x73,
        0x6f, 0x66, 0x74, 0x20, 0x32, 0x2e, 0x30, 0x30, // "Hudson soft 2.00"
        0x04, 0x00, 0x01, 0x02, 0x00, 0x01, 0x00, 0xc0,
        0x04, 0xd0, 0xfe, 0x02, 0x4f, 0xfa,
    ]
    .into_iter()
    .chain(std::iter::repeat(0u8).take(512 - 32))
    .collect();

    // 標準 parse は失敗 (0x0B-0x0C = "ft" → 29798 が bytes_per_sector になる)
    assert!(Bpb::parse(&boot).is_err());
    assert!(Bpb::looks_like_hudson_dim(&boot));

    // parse_floppy は default_2hd_1232k() で fallback して成功
    let bpb = Bpb::parse_floppy(&boot).unwrap();
    assert_eq!(bpb.bytes_per_sector, 1024);
    assert_eq!(bpb.num_fats, 2);
    assert_eq!(bpb.total_sectors, 1232);
    // root_dir_start = 1 + 2 * 2 = 5、これに 1024 を掛けると 0x1400 (実イメージで確認済み)
    assert_eq!(bpb.root_dir_start() * (bpb.bytes_per_sector as u32), 0x1400);
}

#[test]
fn looks_like_hudson_dim_negative_cases() {
    use xdf_fs::bpb::Bpb;
    // 標準 XDF (X68IPL30) は Hudson と誤判定しない
    let mut std_xdf = vec![0u8; 512];
    std_xdf[0..16].copy_from_slice(&[
        0x60, 0x3c, 0x90, 0x58, 0x36, 0x38, 0x49, 0x50,
        0x4c, 0x33, 0x30, 0x00, 0x04, 0x01, 0x01, 0x00,
    ]);
    assert!(!Bpb::looks_like_hudson_dim(&std_xdf));
    // 短すぎるバッファは false
    assert!(!Bpb::looks_like_hudson_dim(&[0u8; 4]));
}

#[test]
fn parse_floppy_passes_through_standard_xdf() {
    use xdf_fs::bpb::Bpb;
    // 標準 XDF はそのまま Bpb::parse に流れて成功する
    let mut boot = vec![0u8; 512];
    boot[0..16].copy_from_slice(&[
        0x60, 0x3c, 0x90, 0x58, 0x36, 0x38, 0x49, 0x50,
        0x4c, 0x33, 0x30, 0x00, 0x04, 0x01, 0x01, 0x00,
    ]);
    boot[0x10] = 2;
    boot[0x11] = 0xC0;
    boot[0x12] = 0;
    boot[0x13] = 0xD0;
    boot[0x14] = 0x04;
    boot[0x16] = 2;
    boot[0x17] = 0;
    boot[0x18] = 8;
    boot[0x19] = 0;
    boot[0x1A] = 2;
    boot[0x1B] = 0;
    let bpb = Bpb::parse_floppy(&boot).unwrap();
    assert_eq!(bpb.bytes_per_sector, 1024);
    assert_eq!(bpb.total_sectors, 1232);
}
