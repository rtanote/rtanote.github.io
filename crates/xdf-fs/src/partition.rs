//! X68000 HDD パーティションテーブル
//!
//! HDS (SCSI) / HDF (SASI) で共通のパーティションテーブル構造を扱う。
//! テーブルは物理セクタ 4 に置かれ、`"X68K"` マジックを持つ16Bヘッダ +
//! 16Bパーティションエントリ列で構成される。
//!
//! 多バイト整数は **ビッグエンディアン** (m68k 慣例)。
//! セクタ単位はディスク形式依存:
//! - HDS: 1024 B 論理セクタ
//! - HDF: 256 B 物理セクタ
//!
//! 詳細は `docs/hdd-format.md` を参照。

use anyhow::{bail, Result};

/// `"X68K"` マジック
pub const PARTITION_MAGIC: [u8; 4] = *b"X68K";

/// パーティションテーブルの所在 (両形式とも物理セクタ4)
pub const PARTITION_TABLE_SECTOR: usize = 4;

/// テーブルヘッダのサイズ
pub const HEADER_SIZE: usize = 16;

/// 1エントリのサイズ
pub const ENTRY_SIZE: usize = 16;

/// 1テーブルが持てるエントリの最大数。
/// 物理的には 16 + 15*16 = 256 B でテーブルが埋まる想定。
pub const MAX_ENTRIES: usize = 15;

/// パーティションテーブル全体の最小バッファサイズ
pub const TABLE_BYTES: usize = HEADER_SIZE + MAX_ENTRIES * ENTRY_SIZE;

/// パーティションエントリ
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// ASCII 名 (NUL/空白パディング除去後)
    pub name: String,
    /// 開始セクタ番号 (単位はディスク形式依存)
    pub start_sector: u32,
    /// セクタ数 (同単位)
    pub length_sectors: u32,
}

impl Partition {
    /// 16 B のエントリをパース。先頭バイトが `0x00` なら未使用 (None)。
    pub fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < ENTRY_SIZE || buf[0] == 0x00 {
            return None;
        }
        let name = bytes_to_ascii(&buf[0..8]);
        let start_sector = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let length_sectors = u32::from_be_bytes([buf[12], buf[13], buf[14], buf[15]]);
        Some(Partition {
            name,
            start_sector,
            length_sectors,
        })
    }

    /// 開始バイトオフセット (ディスク先頭から)
    pub fn start_bytes(&self, unit_bytes: usize) -> u64 {
        self.start_sector as u64 * unit_bytes as u64
    }

    /// パーティションのバイト長
    pub fn length_bytes(&self, unit_bytes: usize) -> u64 {
        self.length_sectors as u64 * unit_bytes as u64
    }

    /// `"Human68k"` パーティションかどうか (大文字小文字一致)
    pub fn is_human68k(&self) -> bool {
        self.name == "Human68k"
    }
}

/// パーティションテーブル (16Bヘッダ + エントリ列)
#[derive(Debug, Clone)]
pub struct PartitionTable {
    /// 最終使用セクタ + 1 (推定)
    pub end_used_sector: u32,
    /// ディスク最終セクタ (推定)
    pub last_sector: u32,
    /// ディスク最終セクタ重複 (用途未確認)
    pub last_sector_2: u32,
    /// パーティションエントリ
    pub partitions: Vec<Partition>,
}

impl PartitionTable {
    /// セクタ4から読んだバイト列をパース。`buf` は最低 `HEADER_SIZE` バイト必要。
    /// 通常は 1論理セクタ (1024 B) 渡せば十分。
    pub fn parse(buf: &[u8]) -> Result<Self> {
        if buf.len() < HEADER_SIZE {
            bail!("Partition table buffer too small: {} bytes", buf.len());
        }
        if buf[0..4] != PARTITION_MAGIC {
            bail!(
                "Partition table magic mismatch: expected X68K, found {:02X?}",
                &buf[0..4]
            );
        }
        let end_used_sector = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let last_sector = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let last_sector_2 = u32::from_be_bytes([buf[12], buf[13], buf[14], buf[15]]);

        let mut partitions = Vec::new();
        for i in 0..MAX_ENTRIES {
            let off = HEADER_SIZE + i * ENTRY_SIZE;
            if off + ENTRY_SIZE > buf.len() {
                break;
            }
            let entry = &buf[off..off + ENTRY_SIZE];
            // 全ゼロ = 未使用スロット (以降も無いと仮定して打ち切る)
            if entry.iter().all(|&b| b == 0x00) {
                break;
            }
            // 別の X68K マジック = 隣接する別レコード/バックアップ → 打ち切る
            if entry[0..4] == PARTITION_MAGIC {
                break;
            }
            if let Some(p) = Partition::parse(entry) {
                partitions.push(p);
            }
        }
        Ok(PartitionTable {
            end_used_sector,
            last_sector,
            last_sector_2,
            partitions,
        })
    }

    /// `"Human68k"` パーティションのみ抽出
    pub fn human68k_partitions(&self) -> Vec<&Partition> {
        self.partitions.iter().filter(|p| p.is_human68k()).collect()
    }
}

/// 8バイトの ASCII 名から末尾の NUL/空白を取り除いた文字列を返す
fn bytes_to_ascii(buf: &[u8]) -> String {
    // NUL以降をカット
    let until_nul: Vec<u8> = buf.iter().copied().take_while(|&b| b != 0x00).collect();
    String::from_utf8_lossy(&until_nul).trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// HDS サンプル (`SCSIHDD1.HDS`) の sector 4 (1024B logical) を再現
    fn hds_sample_table_bytes() -> Vec<u8> {
        let mut buf = vec![0u8; TABLE_BYTES];
        buf[0..4].copy_from_slice(&PARTITION_MAGIC);
        buf[4..8].copy_from_slice(&0x000E_0C20u32.to_be_bytes());
        buf[8..12].copy_from_slice(&0x000E_0FFFu32.to_be_bytes());
        buf[12..16].copy_from_slice(&0x000E_0FFFu32.to_be_bytes());
        // partition 0: Human68k, start=32 (1024B units), length=920_576
        buf[16..24].copy_from_slice(b"Human68k");
        buf[24..28].copy_from_slice(&32u32.to_be_bytes());
        buf[28..32].copy_from_slice(&920_576u32.to_be_bytes());
        buf
    }

    /// HDF サンプル (`hd1.hdf`) の sector 4 (256B physical) を再現
    fn hdf_sample_table_bytes() -> Vec<u8> {
        let mut buf = vec![0u8; TABLE_BYTES];
        buf[0..4].copy_from_slice(&PARTITION_MAGIC);
        buf[4..8].copy_from_slice(&0x0002_7930u32.to_be_bytes());
        buf[8..12].copy_from_slice(&0x0002_7930u32.to_be_bytes());
        buf[12..16].copy_from_slice(&0x0002_ACC0u32.to_be_bytes());
        // partition 0: Human68k, start=33 (256B units), length=162_040
        buf[16..24].copy_from_slice(b"Human68k");
        buf[24..28].copy_from_slice(&33u32.to_be_bytes());
        buf[28..32].copy_from_slice(&162_040u32.to_be_bytes());
        buf
    }

    #[test]
    fn parse_hds_header() {
        let buf = hds_sample_table_bytes();
        let pt = PartitionTable::parse(&buf).unwrap();
        assert_eq!(pt.end_used_sector, 0x000E_0C20);
        assert_eq!(pt.last_sector, 0x000E_0FFF);
        assert_eq!(pt.last_sector_2, 0x000E_0FFF);
    }

    #[test]
    fn parse_hds_partition_entry() {
        let buf = hds_sample_table_bytes();
        let pt = PartitionTable::parse(&buf).unwrap();
        assert_eq!(pt.partitions.len(), 1);
        let p = &pt.partitions[0];
        assert_eq!(p.name, "Human68k");
        assert_eq!(p.start_sector, 32);
        assert_eq!(p.length_sectors, 920_576);
        assert!(p.is_human68k());
    }

    #[test]
    fn parse_hdf_table() {
        let buf = hdf_sample_table_bytes();
        let pt = PartitionTable::parse(&buf).unwrap();
        assert_eq!(pt.end_used_sector, 0x0002_7930);
        assert_eq!(pt.partitions.len(), 1);
        let p = &pt.partitions[0];
        assert_eq!(p.start_sector, 33);
        assert_eq!(p.length_sectors, 162_040);
    }

    #[test]
    fn start_and_length_bytes_with_units() {
        let p = Partition {
            name: "Human68k".to_string(),
            start_sector: 32,
            length_sectors: 920_576,
        };
        // HDS unit (1024 B logical)
        assert_eq!(p.start_bytes(1024), 32 * 1024);
        assert_eq!(p.length_bytes(1024), 920_576u64 * 1024);
        // HDF unit (256 B physical)
        assert_eq!(p.start_bytes(256), 32 * 256);
        assert_eq!(p.length_bytes(256), 920_576u64 * 256);
    }

    #[test]
    fn rejects_wrong_magic() {
        let mut buf = vec![0u8; TABLE_BYTES];
        buf[0..4].copy_from_slice(b"XXXX");
        assert!(PartitionTable::parse(&buf).is_err());
    }

    #[test]
    fn empty_first_entry_yields_no_partitions() {
        let mut buf = vec![0u8; TABLE_BYTES];
        buf[0..4].copy_from_slice(&PARTITION_MAGIC);
        // entries領域は全ゼロ
        let pt = PartitionTable::parse(&buf).unwrap();
        assert!(pt.partitions.is_empty());
    }

    #[test]
    fn nested_x68k_magic_breaks_iteration() {
        // 1個目のエントリの後に再度 X68K が来たら以降は無視
        let mut buf = vec![0u8; TABLE_BYTES];
        buf[0..4].copy_from_slice(&PARTITION_MAGIC);
        buf[16..24].copy_from_slice(b"Human68k");
        buf[24..28].copy_from_slice(&32u32.to_be_bytes());
        buf[28..32].copy_from_slice(&100u32.to_be_bytes());
        // 2個目のスロットに X68K (バックアップ的なもの)
        buf[32..36].copy_from_slice(&PARTITION_MAGIC);
        buf[36..40].copy_from_slice(&999u32.to_be_bytes());
        let pt = PartitionTable::parse(&buf).unwrap();
        assert_eq!(pt.partitions.len(), 1);
    }

    #[test]
    fn human68k_filter() {
        let mut buf = vec![0u8; TABLE_BYTES];
        buf[0..4].copy_from_slice(&PARTITION_MAGIC);
        buf[16..24].copy_from_slice(b"Human68k");
        buf[24..28].copy_from_slice(&32u32.to_be_bytes());
        buf[28..32].copy_from_slice(&100u32.to_be_bytes());
        buf[32..40].copy_from_slice(b"Swap    ");
        buf[40..44].copy_from_slice(&132u32.to_be_bytes());
        buf[44..48].copy_from_slice(&50u32.to_be_bytes());
        let pt = PartitionTable::parse(&buf).unwrap();
        assert_eq!(pt.partitions.len(), 2);
        let h = pt.human68k_partitions();
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].name, "Human68k");
    }

    #[test]
    fn ascii_name_strips_trailing_padding() {
        assert_eq!(bytes_to_ascii(b"Human68k"), "Human68k");
        assert_eq!(bytes_to_ascii(b"Swap    "), "Swap");
        assert_eq!(bytes_to_ascii(b"Hum\0junk"), "Hum");
        assert_eq!(bytes_to_ascii(b"  AB    "), "  AB");
    }

    #[test]
    fn buffer_too_small_for_header_errors() {
        let buf = vec![0u8; 8];
        assert!(PartitionTable::parse(&buf).is_err());
    }
}
