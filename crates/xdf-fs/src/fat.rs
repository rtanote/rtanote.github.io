//! FAT12 / FAT16 統合インターフェース
//!
//! `Bpb` のクラスタ数で FAT12 / FAT16 を自動判別し、
//! 共通 API (`chain`) で扱えるようにする。
//!
//! 判別境界 (MS-DOS 標準):
//! - クラスタ数 < 4085 → FAT12
//! - クラスタ数 >= 4085 → FAT16
//! - クラスタ数 >= 65525 → FAT32 (本プロジェクトでは未対応)

use crate::bpb::Bpb;
use crate::fat12::Fat12;
use crate::fat16::Fat16;
use crate::image::DiskImage;
use anyhow::{anyhow, Result};

/// FAT12 / FAT16 共通インターフェース
pub enum Fat {
    Fat12(Fat12),
    Fat16(Fat16),
}

impl Fat {
    /// BPBからFATを読み込む。クラスタ数でFAT12 / FAT16を自動判別。
    pub fn load(image: &dyn DiskImage, bpb: &Bpb) -> Result<Self> {
        let cc = cluster_count(bpb);
        if cc >= 65525 {
            return Err(anyhow!(
                "Cluster count {} requires FAT32 which is not supported",
                cc
            ));
        }
        if cc < 4085 {
            Ok(Fat::Fat12(Fat12::load(image, bpb)?))
        } else {
            Ok(Fat::Fat16(Fat16::load(image, bpb)?))
        }
    }

    /// 開始クラスタからのチェーンを追跡
    pub fn chain(&self, start: u16) -> Vec<u16> {
        match self {
            Fat::Fat12(f) => f.chain(start),
            Fat::Fat16(f) => f.chain(start),
        }
    }

    /// このFATの種別 (デバッグ・ログ用)
    pub fn kind(&self) -> FatKind {
        match self {
            Fat::Fat12(_) => FatKind::Fat12,
            Fat::Fat16(_) => FatKind::Fat16,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatKind {
    Fat12,
    Fat16,
}

/// データ領域に存在するクラスタ数を計算
pub fn cluster_count(bpb: &Bpb) -> u32 {
    let data_sectors = bpb.total_sectors.saturating_sub(bpb.data_start());
    data_sectors / bpb.sectors_per_cluster.max(1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用: メモリ上のディスクイメージ
    struct FakeImage {
        data: Vec<u8>,
        sec_size: usize,
    }
    impl DiskImage for FakeImage {
        fn sector_size(&self) -> usize {
            self.sec_size
        }
        fn sector_count(&self) -> usize {
            self.data.len() / self.sec_size
        }
        fn read_sector(&self, lba: usize) -> Result<&[u8]> {
            let off = lba * self.sec_size;
            Ok(&self.data[off..off + self.sec_size])
        }
    }

    fn xdf_like_bpb() -> Bpb {
        // XDF (1232KB) クラス: 約 1199 クラスタ → FAT12
        Bpb {
            bytes_per_sector: 1024,
            sectors_per_cluster: 1,
            reserved_sectors: 1,
            num_fats: 2,
            root_entries: 192,
            total_sectors: 1232,
            sectors_per_fat: 2,
            sectors_per_track: 8,
            num_heads: 2,
        }
    }

    fn hdd_like_bpb() -> Bpb {
        // HDF クラス: 40,510 / 1 = 40,510 クラスタ → FAT16
        Bpb {
            bytes_per_sector: 1024,
            sectors_per_cluster: 1,
            reserved_sectors: 1,
            num_fats: 2,
            root_entries: 512,
            total_sectors: 40_510,
            sectors_per_fat: 80,
            sectors_per_track: 0,
            num_heads: 0,
        }
    }

    #[test]
    fn cluster_count_xdf() {
        // total=1232, data_start=11, spc=1 → 1221
        let cc = cluster_count(&xdf_like_bpb());
        assert!(cc < 4085, "XDF should have < 4085 clusters, got {}", cc);
    }

    #[test]
    fn cluster_count_hdd() {
        let cc = cluster_count(&hdd_like_bpb());
        assert!(cc >= 4085 && cc < 65525, "HDD should be FAT16 range, got {}", cc);
    }

    #[test]
    fn fat_load_picks_fat12_for_xdf() {
        let bpb = xdf_like_bpb();
        // FAT12 用にダミーイメージを構築 (FAT12 が読み込める最小限)
        let mut data = vec![0u8; 1024 * 11]; // 11 logical sectors
        // 物理セクタ 512, 論理 1024 → phys_per_logi = 2
        let img = FakeImage {
            data: data.split_off(0),
            sec_size: 512,
        };
        let fat = Fat::load(&img, &bpb).unwrap();
        assert_eq!(fat.kind(), FatKind::Fat12);
    }

    #[test]
    fn fat_load_picks_fat16_for_hdd() {
        let bpb = hdd_like_bpb();
        // 物理セクタ 1024 (HDF風), reserved=1, sectors_per_fat=80, num_fats=2
        // FAT 全体は 1 + 2*80 = 161 logical sectors。1024*161 = 164,864B のバッファが必要。
        let data = vec![0u8; 1024 * 161];
        let img = FakeImage { data, sec_size: 1024 };
        let fat = Fat::load(&img, &bpb).unwrap();
        assert_eq!(fat.kind(), FatKind::Fat16);
    }
}
