//! CLI/外部 API 用の image spec パーサと filesystem アクセスヘルパ
//!
//! Spec 文字列フォーマット:
//! ```text
//! path                       # フロッピー or HDDの先頭パーティション、ルート
//! path:/inner/path           # 同上、内部パス指定
//! path:N                     # HDDのN番目のパーティション、ルート
//! path:N:/inner/path         # 同上、内部パス指定
//! ```
//!
//! Windows ドライブレター (`C:\`) は適切にスキップされる。

use crate::bpb::Bpb;
use crate::fs::Filesystem;
use crate::image::{DiskImage, OpenedImage};
use anyhow::{anyhow, Result};
use std::path::PathBuf;

/// Image spec: ファイルパス + (任意) パーティション番号 + (任意) 内部パス
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSpec {
    pub path: PathBuf,
    pub partition: Option<usize>,
    /// 内部パス (`/` 区切り、末尾の `/` は trim せずそのまま保持)
    pub inner_path: String,
}

impl ImageSpec {
    pub fn parse(s: &str) -> Result<Self> {
        if s.is_empty() {
            return Err(anyhow!("Empty image spec"));
        }
        // ':/' が内部パス開始位置 (Windows ドライブレターは除外)
        let inner_pos = find_inner_separator(s);
        let (head, inner) = match inner_pos {
            Some(idx) => (&s[..idx], &s[idx + 1..]),
            None => (s, ""),
        };
        let (path_str, partition) = parse_head(head);
        Ok(ImageSpec {
            path: PathBuf::from(path_str),
            partition,
            inner_path: inner.to_string(),
        })
    }
}

/// `':/'` の境界位置を返す。Windowsドライブレター `[A-Z]:` は除外。
fn find_inner_separator(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let start = drive_letter_skip(bytes);
    let mut i = start;
    while i + 1 < bytes.len() {
        if bytes[i] == b':' && bytes[i + 1] == b'/' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// path-spec 部分から末尾の `:N` (N は数字列) をパーティション番号として切り出す
fn parse_head(s: &str) -> (&str, Option<usize>) {
    let bytes = s.as_bytes();
    let drive_skip = drive_letter_skip(bytes);
    // drive_skip 以降の最後の ':' を探す
    let mut last_colon: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate().skip(drive_skip) {
        if b == b':' {
            last_colon = Some(i);
        }
    }
    if let Some(idx) = last_colon {
        let after = &s[idx + 1..];
        if !after.is_empty() && after.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(n) = after.parse::<usize>() {
                return (&s[..idx], Some(n));
            }
        }
    }
    (s, None)
}

fn drive_letter_skip(bytes: &[u8]) -> usize {
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        2
    } else {
        0
    }
}

/// Image spec に対応する `Filesystem` を構築し、コールバックを実行する。
///
/// Filesystem<'_> は HDD パーティションの場合 `PartitionImage` を借用するため
/// クロージャ内でしかアクセスできない。これがその制約をうまく扱うためのラッパ。
pub fn with_filesystem<R, F>(spec: &ImageSpec, f: F) -> Result<R>
where
    F: FnOnce(&Filesystem, &str) -> Result<R>,
{
    let opened = OpenedImage::open(&spec.path)?;
    match opened {
        OpenedImage::Floppy(img) => {
            if spec.partition.is_some() {
                return Err(anyhow!(
                    "Floppy image (XDF) does not have partitions; specified partition number is invalid"
                ));
            }
            let fs = Filesystem::open(&img)?;
            f(&fs, &spec.inner_path)
        }
        OpenedImage::Hdd(hdd) => {
            let part_idx = spec.partition.unwrap_or(0);
            if part_idx >= hdd.partitions().len() {
                return Err(anyhow!(
                    "Partition index {} out of range (image has {} partition(s))",
                    part_idx,
                    hdd.partitions().len()
                ));
            }
            let part = hdd.partition(part_idx)?;
            let boot = part.read_sector(0)?;
            let bpb = Bpb::parse_hdd(boot)?;
            let fs = Filesystem::open_with_bpb(&part, bpb)?;
            f(&fs, &spec.inner_path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(path: &str, partition: Option<usize>, inner: &str) -> ImageSpec {
        ImageSpec {
            path: PathBuf::from(path),
            partition,
            inner_path: inner.to_string(),
        }
    }

    #[test]
    fn parse_plain_path() {
        assert_eq!(ImageSpec::parse("disk.xdf").unwrap(), spec("disk.xdf", None, ""));
    }

    #[test]
    fn parse_with_inner_path() {
        assert_eq!(
            ImageSpec::parse("disk.xdf:/MUSIC").unwrap(),
            spec("disk.xdf", None, "/MUSIC")
        );
    }

    #[test]
    fn parse_with_partition_only() {
        assert_eq!(
            ImageSpec::parse("disk.hds:0").unwrap(),
            spec("disk.hds", Some(0), "")
        );
        assert_eq!(
            ImageSpec::parse("disk.hds:5").unwrap(),
            spec("disk.hds", Some(5), "")
        );
    }

    #[test]
    fn parse_with_partition_and_inner() {
        assert_eq!(
            ImageSpec::parse("disk.hds:0:/MUSIC").unwrap(),
            spec("disk.hds", Some(0), "/MUSIC")
        );
        assert_eq!(
            ImageSpec::parse("disk.hds:2:/SUB/PATH").unwrap(),
            spec("disk.hds", Some(2), "/SUB/PATH")
        );
    }

    #[test]
    fn parse_windows_drive_path() {
        assert_eq!(
            ImageSpec::parse("C:\\disk.xdf").unwrap(),
            spec("C:\\disk.xdf", None, "")
        );
        assert_eq!(
            ImageSpec::parse("C:\\disk.xdf:/MUSIC").unwrap(),
            spec("C:\\disk.xdf", None, "/MUSIC")
        );
        assert_eq!(
            ImageSpec::parse("C:\\disk.hds:0:/MUSIC").unwrap(),
            spec("C:\\disk.hds", Some(0), "/MUSIC")
        );
    }

    #[test]
    fn parse_unix_absolute_path() {
        assert_eq!(
            ImageSpec::parse("/path/to/disk.xdf").unwrap(),
            spec("/path/to/disk.xdf", None, "")
        );
        assert_eq!(
            ImageSpec::parse("/path/disk.hds:0:/MUSIC").unwrap(),
            spec("/path/disk.hds", Some(0), "/MUSIC")
        );
    }

    #[test]
    fn parse_relative_with_dots() {
        assert_eq!(
            ImageSpec::parse("../d.hds:1:/X").unwrap(),
            spec("../d.hds", Some(1), "/X")
        );
    }

    #[test]
    fn parse_non_numeric_after_colon_kept_in_path() {
        // 'disk.hds:abc' は abc がパーティション番号でないので path 全体扱い
        assert_eq!(
            ImageSpec::parse("disk.hds:abc").unwrap(),
            spec("disk.hds:abc", None, "")
        );
    }

    #[test]
    fn parse_empty_errors() {
        assert!(ImageSpec::parse("").is_err());
    }
}
