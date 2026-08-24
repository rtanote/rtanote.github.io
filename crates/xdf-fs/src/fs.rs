//! Human68k ファイルシステム層
//!
//! BPB + FAT12 + ディレクトリエントリ走査をまとめて提供する。

use crate::bpb::Bpb;
use crate::direntry::{Attr, DirEntry, EntryKind, ENTRY_SIZE};
use crate::fat::Fat;
use crate::image::DiskImage;
use anyhow::{anyhow, Result};

pub struct Filesystem<'a> {
    pub image: &'a dyn DiskImage,
    pub bpb: Bpb,
    pub fat: Fat,
    pub phys_per_logi: usize,
}

impl<'a> Filesystem<'a> {
    /// XDF (フロッピー、MS-DOS互換BPB) を開く
    ///
    /// 標準 BPB のパースに失敗した場合、Hudson DIM (OEM 16 バイトの XDF 互換) を
    /// 検出して 2HD 1232KB のデフォルト BPB で代用する fallback 経路を持つ。
    pub fn open(image: &'a dyn DiskImage) -> Result<Self> {
        let boot = image.read_sector(0)?;
        let bpb = Bpb::parse_floppy(boot)?;
        Self::open_with_bpb(image, bpb)
    }

    /// 既にパース済みの BPB を使って Filesystem を構築する。
    /// HDS/HDF パーティションのように、外側でパース済みの BPB を渡したいケースに使う。
    pub fn open_with_bpb(image: &'a dyn DiskImage, bpb: Bpb) -> Result<Self> {
        let phys_sec = image.sector_size();
        let logi_sec = bpb.bytes_per_sector as usize;
        if logi_sec % phys_sec != 0 {
            return Err(anyhow!(
                "Logical sector ({}) not multiple of physical sector ({})",
                logi_sec, phys_sec
            ));
        }
        let phys_per_logi = logi_sec / phys_sec;
        let fat = Fat::load(image, &bpb)?;
        Ok(Self { image, bpb, fat, phys_per_logi })
    }

    /// 論理セクタを物理セクタ群として読む
    fn read_logical_sector(&self, logi_lba: u32) -> Result<Vec<u8>> {
        let phys_lba = (logi_lba as usize) * self.phys_per_logi;
        let mut buf = Vec::with_capacity(self.bpb.bytes_per_sector as usize);
        for i in 0..self.phys_per_logi {
            buf.extend_from_slice(self.image.read_sector(phys_lba + i)?);
        }
        Ok(buf)
    }

    /// ルートディレクトリを読む
    pub fn read_root_dir(&self) -> Result<Vec<DirEntry>> {
        let start = self.bpb.root_dir_start();
        let count = self.bpb.root_dir_sectors();
        let mut buf = Vec::new();
        for i in 0..count {
            buf.extend_from_slice(&self.read_logical_sector(start + i)?);
        }
        Ok(parse_directory(&buf))
    }

    /// クラスタチェーンの全データを連結して読む
    pub fn read_cluster_chain(&self, start_cluster: u16) -> Result<Vec<u8>> {
        // クラスタ < 2 は「ファイルなし」を意味する慣例値
        // (".." の親がルートディレクトリの場合 cluster=0 となる)
        if start_cluster < 2 {
            return Ok(Vec::new());
        }
        let chain = self.fat.chain(start_cluster);
        let cluster_bytes =
            self.bpb.sectors_per_cluster as usize * self.bpb.bytes_per_sector as usize;
        let mut buf = Vec::with_capacity(chain.len() * cluster_bytes);
        for c in chain {
            let logi_sec = self.bpb.cluster_to_sector(c);
            for i in 0..(self.bpb.sectors_per_cluster as u32) {
                buf.extend_from_slice(&self.read_logical_sector(logi_sec + i)?);
            }
        }
        Ok(buf)
    }

    /// サブディレクトリを読む
    pub fn read_subdir(&self, start_cluster: u16) -> Result<Vec<DirEntry>> {
        let buf = self.read_cluster_chain(start_cluster)?;
        Ok(parse_directory(&buf))
    }

    /// ファイルの中身を読む (sizeで切り詰め)
    pub fn read_file(&self, entry: &DirEntry) -> Result<Vec<u8>> {
        if entry.attr.contains(Attr::DIRECTORY) {
            return Err(anyhow!("Not a regular file: {}", entry.display_name()));
        }
        let mut buf = self.read_cluster_chain(entry.start_cluster)?;
        buf.truncate(entry.size as usize);
        Ok(buf)
    }

    /// パスを '/' 区切りで解決してエントリを返す。
    ///
    /// - 比較はASCIIに対しては大文字小文字を無視 (Human68kの慣例に合わせる)
    /// - 全角文字は完全一致
    /// - 末尾の '/' は許容
    /// - 空または "/" はルートを指すがDirEntryは存在しないためErrを返す
    pub fn resolve(&self, path: &str) -> Result<DirEntry> {
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return Err(anyhow!(
                "Path refers to root directory; no DirEntry exists for it"
            ));
        }
        let mut current = self.read_root_dir()?;
        for (i, part) in parts.iter().enumerate() {
            let is_last = i == parts.len() - 1;
            let found = current
                .iter()
                .find(|e| e.kind == EntryKind::Used && name_matches(e, part));
            let entry = match found {
                Some(e) => e.clone(),
                None => return Err(anyhow!("Not found: {}", part)),
            };
            if is_last {
                return Ok(entry);
            }
            if !entry.attr.contains(Attr::DIRECTORY) {
                return Err(anyhow!("Not a directory: {}", part));
            }
            current = self.read_subdir(entry.start_cluster)?;
        }
        unreachable!()
    }

    /// パスを解決してその中身のエントリ一覧を返す。
    /// 空文字列または "/" はルートディレクトリを返す。
    pub fn list_path(&self, path: &str) -> Result<Vec<DirEntry>> {
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return self.read_root_dir();
        }
        let entry = self.resolve(trimmed)?;
        if !entry.attr.contains(Attr::DIRECTORY) {
            return Err(anyhow!("Not a directory: {}", path));
        }
        self.read_subdir(entry.start_cluster)
    }
}

fn name_matches(entry: &DirEntry, part: &str) -> bool {
    entry.display_name().eq_ignore_ascii_case(part)
}

fn parse_directory(buf: &[u8]) -> Vec<DirEntry> {
    let mut out = Vec::new();
    for chunk in buf.chunks_exact(ENTRY_SIZE) {
        if let Some(e) = DirEntry::parse(chunk) {
            if e.kind == EntryKind::Empty {
                break;
            }
            out.push(e);
        }
    }
    out
}
