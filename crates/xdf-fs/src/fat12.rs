//! FAT12 のクラスタチェーン追跡
//!
//! 12bitエントリが2個ずつ3バイトに詰められている:
//!   byte0 = entry[N] の下位8bit
//!   byte1 = (entry[N+1] 下位4bit << 4) | entry[N] 上位4bit
//!   byte2 = entry[N+1] の上位8bit
//!
//! 終端値: 0xFF8 - 0xFFF
//! 不良 : 0xFF7

use crate::bpb::Bpb;
use crate::image::DiskImage;
use anyhow::{anyhow, Result};

pub struct Fat12 {
    raw: Vec<u8>,
}

impl Fat12 {
    pub fn load(image: &dyn DiskImage, bpb: &Bpb) -> Result<Self> {
        // 注意: BPBのbytes_per_sectorとイメージ側の物理セクタサイズが
        // 食い違う場合がある。XDFは「論理1024B/物理512B」のことが多い。
        // ここでは読み出しを「物理セクタ単位」に正規化する。
        let phys_sec = image.sector_size();
        let logi_sec = bpb.bytes_per_sector as usize;
        if logi_sec % phys_sec != 0 {
            return Err(anyhow!(
                "Logical sector ({}) is not a multiple of physical sector ({})",
                logi_sec, phys_sec
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

    /// クラスタ番号 N の値を取得 (12bit)
    pub fn entry(&self, n: u16) -> u16 {
        let n = n as usize;
        let off = n + n / 2; // n * 1.5
        if off + 1 >= self.raw.len() {
            return 0xFF8; // 範囲外は終端扱い
        }
        let lo = self.raw[off] as u16;
        let hi = self.raw[off + 1] as u16;
        if n & 1 == 0 {
            // 偶数: 下位12bit
            lo | ((hi & 0x0F) << 8)
        } else {
            // 奇数: 上位12bit
            (lo >> 4) | (hi << 4)
        }
    }

    /// 開始クラスタからのクラスタチェーンをたどる
    pub fn chain(&self, start: u16) -> Vec<u16> {
        let mut out = Vec::new();
        let mut cur = start;
        // 0と1は予約 / 0xFF8-0xFFFは終端 / 0xFF7は不良
        while cur >= 2 && cur < 0xFF8 {
            out.push(cur);
            let next = self.entry(cur);
            if next == cur {
                break; // 自己ループ防止
            }
            cur = next;
            if out.len() > 100_000 {
                break; // 念のため暴走防止
            }
        }
        out
    }
}
