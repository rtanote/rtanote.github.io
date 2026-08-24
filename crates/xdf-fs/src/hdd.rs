//! X68000 HDD イメージ (HDS / HDF) のセクタ単位アクセス
//!
//! [`HddImage`] が物理セクタ列をマップし、内部にパーティションリストを保持する。
//! [`HddImage::partition`] で各パーティションを [`PartitionImage`] (DiskImage trait
//! 実装) として取り出すと、既存の [`crate::fs::Filesystem`] にそのまま渡せる。
//!
//! HDS と HDF は物理セクタサイズが異なる (512B / 256B) のみで、
//! パーティションテーブルの構造は共通。本モジュールは1つの構造体で両対応。
//!
//! 詳細は `docs/hdd-format.md` 参照。

use crate::image::DiskImage;
use crate::partition::{Partition, PartitionTable, PARTITION_TABLE_SECTOR};
use anyhow::{anyhow, bail, Result};
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;

/// パーティションの位置情報 (物理セクタ単位、`HddImage` 内部用)
#[derive(Debug, Clone)]
pub struct PartitionInfo {
    pub name: String,
    /// 親イメージ内での開始物理セクタ番号
    pub phys_start: usize,
    /// 物理セクタ数
    pub phys_count: usize,
}

impl PartitionInfo {
    pub fn is_human68k(&self) -> bool {
        self.name == "Human68k"
    }
}

/// X68000 HDD イメージ (HDS = SCSI、HDF = SASI 共用)
pub struct HddImage {
    mmap: Mmap,
    phys_sec_size: usize,
    partitions: Vec<PartitionInfo>,
}

impl HddImage {
    /// HDS (SCSI HDD、512 B 物理セクタ) として開く
    pub fn open_hds<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_phys(path, 512)
    }

    /// HDF (SASI HDD、256 B 物理セクタ) として開く
    pub fn open_hdf<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_phys(path, 256)
    }

    fn open_with_phys<P: AsRef<Path>>(path: P, phys_sec_size: usize) -> Result<Self> {
        if !matches!(phys_sec_size, 256 | 512) {
            bail!("Unsupported physical sector size: {}", phys_sec_size);
        }
        let file = File::open(path.as_ref())?;
        let mmap = unsafe { Mmap::map(&file)? };
        if mmap.len() % phys_sec_size != 0 {
            return Err(anyhow!(
                "Image size {} is not a multiple of physical sector size {}",
                mmap.len(),
                phys_sec_size
            ));
        }

        // パーティションテーブル所在: 物理セクタ4 (両形式共通)
        let table_off = phys_sec_size * PARTITION_TABLE_SECTOR;
        // テーブルは1論理セクタ (1024B) に収まる想定。マージンとってバッファ確保。
        let table_end = (table_off + 1024).min(mmap.len());
        if table_off >= mmap.len() {
            bail!("Image too small to hold a partition table");
        }
        let table = PartitionTable::parse(&mmap[table_off..table_end])?;

        // パーティションテーブルの単位: HDS=1024B、HDF=256B
        let partition_unit = match phys_sec_size {
            512 => 1024,
            256 => 256,
            _ => unreachable!(),
        };
        if partition_unit % phys_sec_size != 0 {
            bail!(
                "partition_unit {} not multiple of phys_sec_size {}",
                partition_unit,
                phys_sec_size
            );
        }
        let mul = partition_unit / phys_sec_size;

        let partitions: Vec<PartitionInfo> = table
            .partitions
            .iter()
            .map(|p: &Partition| PartitionInfo {
                name: p.name.clone(),
                phys_start: p.start_sector as usize * mul,
                phys_count: p.length_sectors as usize * mul,
            })
            .collect();

        Ok(Self {
            mmap,
            phys_sec_size,
            partitions,
        })
    }

    /// 物理セクタサイズ (512 or 256)
    pub fn phys_sec_size(&self) -> usize {
        self.phys_sec_size
    }

    /// 全パーティション情報
    pub fn partitions(&self) -> &[PartitionInfo] {
        &self.partitions
    }

    /// `idx` 番目のパーティションを `DiskImage` として取り出す
    pub fn partition(&self, idx: usize) -> Result<PartitionImage<'_>> {
        let info = self
            .partitions
            .get(idx)
            .ok_or_else(|| anyhow!("Partition index {} out of range", idx))?;
        // 範囲検証
        let end_byte = (info.phys_start + info.phys_count) * self.phys_sec_size;
        if end_byte > self.mmap.len() {
            bail!(
                "Partition {} extends beyond image (end_byte={}, image_len={})",
                idx,
                end_byte,
                self.mmap.len()
            );
        }
        Ok(PartitionImage {
            parent: self,
            phys_start: info.phys_start,
            phys_count: info.phys_count,
        })
    }
}

/// パーティション内のセクタアクセス。`HddImage` への借用ビュー。
pub struct PartitionImage<'a> {
    parent: &'a HddImage,
    phys_start: usize,
    phys_count: usize,
}

impl<'a> DiskImage for PartitionImage<'a> {
    fn sector_size(&self) -> usize {
        self.parent.phys_sec_size
    }

    fn sector_count(&self) -> usize {
        self.phys_count
    }

    fn read_sector(&self, lba: usize) -> Result<&[u8]> {
        if lba >= self.phys_count {
            return Err(anyhow!(
                "Sector {} out of partition range (count={})",
                lba,
                self.phys_count
            ));
        }
        let abs = (self.phys_start + lba) * self.parent.phys_sec_size;
        Ok(&self.parent.mmap[abs..abs + self.parent.phys_sec_size])
    }
}
