//! FAT16 のクラスタチェーン追跡 (Human68k HDD 用)
//!
//! 16bit エントリが2バイトずつ並ぶ。
//! **Human68k HDD では BIG-ENDIAN** で格納される (m68k native、BPB と一貫)。
//! XDF (フロッピー) の FAT12 が LE なのと対照的。
//!
//! 終端値: 0xFFF8 - 0xFFFF
//! 不良: 0xFFF7
//!
//! 詳細は `docs/hdd-format.md` 「3.3 FAT エントリのエンディアン」参照。

use crate::bpb::Bpb;
use crate::image::DiskImage;
use anyhow::{anyhow, Result};

pub struct Fat16 {
    raw: Vec<u8>,
}

impl Fat16 {
    pub fn load(image: &dyn DiskImage, bpb: &Bpb) -> Result<Self> {
        // fat12 と同じく物理セクタ単位で正規化して読み込む
        let phys_sec = image.sector_size();
        let logi_sec = bpb.bytes_per_sector as usize;
        if logi_sec % phys_sec != 0 {
            return Err(anyhow!(
                "Logical sector ({}) is not a multiple of physical sector ({})",
                logi_sec,
                phys_sec
            ));
        }
        let phys_per_logi = logi_sec / phys_sec;

        let fat_logi_start = bpb.fat_start() as usize;
        let fat_logi_count = bpb.sectors_per_fat as usize;
        let fat_phys_start = fat_logi_start * phys_per_logi;
        let fat_phys_count = fat_logi_count * phys_per_logi;

        let mut raw = Vec::with_capacity(fat_phys_count * phys_sec);
        for i in 0..fat_phys_count {
            raw.extend_from_slice(image.read_sector(fat_phys_start + i)?);
        }
        Ok(Self { raw })
    }

    /// クラスタ番号 N の値を取得 (16bit BE)
    pub fn entry(&self, n: u16) -> u16 {
        let off = n as usize * 2;
        if off + 1 >= self.raw.len() {
            return 0xFFF8; // 範囲外は終端扱い
        }
        u16::from_be_bytes([self.raw[off], self.raw[off + 1]])
    }

    /// 開始クラスタからのクラスタチェーンをたどる
    pub fn chain(&self, start: u16) -> Vec<u16> {
        let mut out = Vec::new();
        let mut cur = start;
        // 0と1は予約 / 0xFFF8-0xFFFF は終端 / 0xFFF7 は不良
        while cur >= 2 && cur < 0xFFF8 {
            out.push(cur);
            let next = self.entry(cur);
            if next == cur {
                break; // 自己ループ防止
            }
            cur = next;
            if out.len() > 1_000_000 {
                break; // 暴走防止 (大容量HDDでは100Kクラスタ超もありうる)
            }
        }
        out
    }

    /// 内部生バッファを返す (デバッグ・テスト用)
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }
}

/// テスト用: メモリバッファから直接構築する
#[cfg(test)]
impl Fat16 {
    pub fn from_raw(raw: Vec<u8>) -> Self {
        Self { raw }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_read_is_big_endian() {
        // バイト列 [0xF8, 0xFF, 0x00, 0x05, 0x00, 0x06] を BE で読むと
        // entry[0]=0xF8FF, entry[1]=0x0005, entry[2]=0x0006
        let raw = vec![0xF8, 0xFF, 0x00, 0x05, 0x00, 0x06];
        let fat = Fat16::from_raw(raw);
        assert_eq!(fat.entry(0), 0xF8FF);
        assert_eq!(fat.entry(1), 0x0005);
        assert_eq!(fat.entry(2), 0x0006);
    }

    #[test]
    fn chain_follows_sequential_clusters() {
        // cluster 4 → 5 → 6 → 7 → end (0xFFFF)
        // entries 0..4 適当 (使われない)、 entry[4]=5, [5]=6, [6]=7, [7]=0xFFFF
        let mut raw = vec![0u8; 16];
        // BE で書き込む
        raw[8..10].copy_from_slice(&5u16.to_be_bytes());      // entry[4]
        raw[10..12].copy_from_slice(&6u16.to_be_bytes());     // entry[5]
        raw[12..14].copy_from_slice(&7u16.to_be_bytes());     // entry[6]
        raw[14..16].copy_from_slice(&0xFFFFu16.to_be_bytes()); // entry[7] = end
        let fat = Fat16::from_raw(raw);
        let chain = fat.chain(4);
        assert_eq!(chain, vec![4, 5, 6, 7]);
    }

    #[test]
    fn chain_handles_end_marker_range() {
        // 0xFFF8〜0xFFFF はすべて終端扱い
        let mut raw = vec![0u8; 16];
        raw[4..6].copy_from_slice(&3u16.to_be_bytes());        // entry[2] = 3
        raw[6..8].copy_from_slice(&0xFFF8u16.to_be_bytes());   // entry[3] = end (0xFFF8)
        let fat = Fat16::from_raw(raw);
        assert_eq!(fat.chain(2), vec![2, 3]);
    }

    #[test]
    fn chain_breaks_on_self_loop() {
        // entry[2] = 2 (自己参照)
        let mut raw = vec![0u8; 8];
        raw[4..6].copy_from_slice(&2u16.to_be_bytes()); // entry[2] = 2
        let fat = Fat16::from_raw(raw);
        assert_eq!(fat.chain(2), vec![2]);
    }

    #[test]
    fn chain_below_2_returns_empty() {
        let fat = Fat16::from_raw(vec![0u8; 16]);
        assert!(fat.chain(0).is_empty());
        assert!(fat.chain(1).is_empty());
    }
}
