//! ホスト (macOS/Linux/Windows) ファイルシステムへの書き出し
//!
//! - SJIS主名はDirEntry側で既にUTF-8化済みなので、そのまま使う。
//! - ホストFSで使えない文字 (Windowsでは '\\/:*?"<>|' 等) はサニタイズ。
//! - ファイルの mtime をX68000のタイムスタンプに合わせる (filetime crate)。
//! - 属性は基本ファイル/ディレクトリ判別のみ。READ_ONLYは現時点では無視。

use crate::direntry::DirEntry;
use anyhow::{Context, Result};
use filetime::{set_file_mtime, FileTime};
use std::path::{Path, PathBuf};

/// ホストFSで安全な名前に変換する。
///
/// - Windowsで予約された文字 \\/:*?"<>| を '_' に置換
/// - 制御文字 (< 0x20) も '_' に置換
/// - 末尾の '.' と空白は除去 (Windows互換)
/// - 空文字列の場合は "_" を返す
pub fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => out.push('_'),
            c if (c as u32) < 0x20 => out.push('_'),
            c => out.push(c),
        }
    }
    while matches!(out.chars().last(), Some('.') | Some(' ')) {
        out.pop();
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// X68000パス ("FOO/BAR/BAZ.X") をホスト用の PathBuf に変換する
pub fn sanitize_path(host_root: &Path, x68k_path: &str) -> PathBuf {
    let mut out = host_root.to_path_buf();
    for part in x68k_path.split('/').filter(|s| !s.is_empty()) {
        out.push(sanitize_filename(part));
    }
    out
}

/// FAT date/time を Unix epoch 秒に変換する (ローカル時刻として解釈)
///
/// Human68k は MS-DOS と同じ「2秒精度・ローカル時刻」を採用している。
/// chronoのlocal時刻演算は重いので、ここでは naive な変換で十分。
/// (タイムゾーン補正は呼び出し側が必要なら行う)
pub fn entry_to_filetime(entry: &DirEntry) -> Option<FileTime> {
    let (y, mo, d) = entry.date();
    let (h, mi, s) = entry.time();
    if y < 1980 || mo == 0 || d == 0 || mo > 12 || d > 31 {
        return None;
    }
    // ナイーブにエポック秒を計算 (UTCとして扱う; X68k実機はローカル時刻保存だが
    // ホスト側でわざわざ補正する用途は稀なのでUTC固定で割り切る)
    let epoch_secs = naive_to_epoch(y as i32, mo as u32, d as u32, h as u32, mi as u32, s as u32);
    Some(FileTime::from_unix_time(epoch_secs, 0))
}

fn naive_to_epoch(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
    // 単純なグレゴリオ暦変換 (1970-01-01からの日数)
    let days = days_from_civil(y, mo, d) - days_from_civil(1970, 1, 1);
    (days as i64) * 86400 + (h as i64) * 3600 + (mi as i64) * 60 + s as i64
}

/// Howard Hinnant 'days from civil' algorithm
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era as i64) * 146097 + (doe as i64) - 719468
}

/// ファイルを書き出して、可能ならmtimeをX68k側のタイムスタンプに合わせる
pub fn write_file(dest: &Path, data: &[u8], entry: &DirEntry) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Cannot create directory {}", parent.display()))?;
    }
    std::fs::write(dest, data)
        .with_context(|| format!("Cannot write file {}", dest.display()))?;
    if let Some(ft) = entry_to_filetime(entry) {
        // mtime設定の失敗は致命的ではないので警告のみ
        if let Err(e) = set_file_mtime(dest, ft) {
            eprintln!("warning: cannot set mtime on {}: {}", dest.display(), e);
        }
    }
    Ok(())
}
