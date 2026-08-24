//! ディスクイメージへのセクタ単位アクセス
//!
//! XDFは「ヘッダなしの生セクタダンプ」なので、ファイル全体をそのまま
//! セクタ番号でインデックスできる。
//!
//! HDS / HDF (X68000 HDDイメージ) は [`crate::hdd::HddImage`] が扱う。
//! [`OpenedImage::open`] が拡張子 + マジックバイトで自動判別する。

use crate::hdd::HddImage;
use anyhow::{anyhow, bail, Result};
use memmap2::Mmap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub const SECTOR_SIZE: usize = 512;

/// セクタ単位の読み取りインターフェース
pub trait DiskImage {
    fn sector_size(&self) -> usize;
    fn sector_count(&self) -> usize;
    fn read_sector(&self, lba: usize) -> Result<&[u8]>;
}

pub struct XdfImage {
    mmap: Mmap,
}

impl XdfImage {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        // XDF (2HD) は 1232 KiB = 1261568 bytes が標準
        // ただし他サイズ (2HC: 1200KB, 2HQ: 1440KB) もあるので
        // ここではセクタサイズ単位かどうかだけチェック
        if mmap.len() % SECTOR_SIZE != 0 {
            return Err(anyhow!(
                "Image size {} is not a multiple of sector size {}",
                mmap.len(),
                SECTOR_SIZE
            ));
        }
        Ok(Self { mmap })
    }
}

impl DiskImage for XdfImage {
    fn sector_size(&self) -> usize {
        SECTOR_SIZE
    }

    fn sector_count(&self) -> usize {
        self.mmap.len() / SECTOR_SIZE
    }

    fn read_sector(&self, lba: usize) -> Result<&[u8]> {
        let off = lba * SECTOR_SIZE;
        if off + SECTOR_SIZE > self.mmap.len() {
            return Err(anyhow!("Sector {} out of range", lba));
        }
        Ok(&self.mmap[off..off + SECTOR_SIZE])
    }
}

/// 自動判別で開かれたイメージ。フロッピー or HDD のいずれか。
///
/// HDD はパーティション選択が必要なので、`DiskImage` を直接実装せず
/// [`crate::hdd::HddImage`] をそのまま保持する。
pub enum OpenedImage {
    /// XDF (フロッピー) 単一ボリューム
    Floppy(XdfImage),
    /// HDS / HDF (HDD) パーティション選択が必要
    Hdd(HddImage),
}

impl OpenedImage {
    /// 拡張子 + マジックバイトで形式を自動判別して開く。
    ///
    /// 判別ルール:
    /// 1. 拡張子 `.xdf` → XDF
    /// 2. 拡張子 `.hds` → HDS (SCSI、512B 物理セクタ)
    /// 3. 拡張子 `.hdf` → HDF (SASI、256B 物理セクタ)
    /// 4. 拡張子 `.img` などその他: 先頭マジックで判別
    ///    - `"X68SCSI1"` → HDS
    ///    - sector 4 (offset 0x800) に `"X68K"` → HDS
    ///    - sector 4 (offset 0x400) に `"X68K"` → HDF
    ///    - それ以外 → XDF として開く (失敗するならエラー)
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        match ext.as_deref() {
            Some("xdf") => Ok(OpenedImage::Floppy(XdfImage::open(path)?)),
            Some("hds") => Ok(OpenedImage::Hdd(HddImage::open_hds(path)?)),
            Some("hdf") => Ok(OpenedImage::Hdd(HddImage::open_hdf(path)?)),
            _ => Self::detect_by_magic(path),
        }
    }

    fn detect_by_magic(path: &Path) -> Result<Self> {
        // 先頭 0x900 バイトを読んでマジック判別 (sector 4 of 512B = offset 0x800)
        let mut f = File::open(path)?;
        let mut buf = vec![0u8; 0x900];
        let n = f.read(&mut buf)?;
        buf.truncate(n);
        drop(f);

        if buf.len() >= 8 && &buf[..8] == b"X68SCSI1" {
            return Ok(OpenedImage::Hdd(HddImage::open_hds(path)?));
        }
        if buf.len() >= 0x804 && &buf[0x800..0x804] == b"X68K" {
            return Ok(OpenedImage::Hdd(HddImage::open_hds(path)?));
        }
        if buf.len() >= 0x404 && &buf[0x400..0x404] == b"X68K" {
            return Ok(OpenedImage::Hdd(HddImage::open_hdf(path)?));
        }
        // フォールバック: XDF を試す
        match XdfImage::open(path) {
            Ok(x) => Ok(OpenedImage::Floppy(x)),
            Err(e) => bail!(
                "Cannot determine image format for {:?}. XDF fallback failed: {}",
                path,
                e
            ),
        }
    }

    /// 形式名 (デバッグ・ログ用)
    pub fn format_name(&self) -> &'static str {
        match self {
            OpenedImage::Floppy(_) => "XDF",
            OpenedImage::Hdd(hdd) => match hdd.phys_sec_size() {
                512 => "HDS",
                256 => "HDF",
                _ => "HDD",
            },
        }
    }
}
