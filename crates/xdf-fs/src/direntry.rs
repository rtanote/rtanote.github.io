//! Human68k ディレクトリエントリ
//!
//! 1エントリ = 32バイト。MS-DOS互換だがファイル名領域の使い方が拡張されている。
//!
//! オフセット | サイズ | 内容
//! ----------|-------|------------------------------------------------
//! 0x00      | 1     | ファイル名先頭バイト (0x00=空, 0xE5=削除)
//! 0x00-0x07 | 8     | ファイル名 主要部 (Shift-JIS)
//! 0x08-0x0A | 3     | 拡張子
//! 0x0B      | 1     | 属性 (MS-DOS互換 + 0x40=リンク等のHuman68k拡張)
//! 0x0C-0x14 | 9     | ★Human68k拡張: ファイル名の続き
//! 0x15      | 1     | (予約 / FAT12では未使用領域)
//! 0x16-0x17 | 2     | 時刻
//! 0x18-0x19 | 2     | 日付
//! 0x1A-0x1B | 2     | 開始クラスタ
//! 0x1C-0x1F | 4     | ファイルサイズ

use encoding_rs::SHIFT_JIS;

pub const ENTRY_SIZE: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Empty,
    Deleted,
    Used,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Attr: u8 {
        const READ_ONLY = 0x01;
        const HIDDEN    = 0x02;
        const SYSTEM    = 0x04;
        const VOLUME    = 0x08;
        const DIRECTORY = 0x10;
        const ARCHIVE   = 0x20;
        const LINK      = 0x40;
    }
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub kind: EntryKind,
    pub name: String,
    pub ext: String,
    pub attr: Attr,
    pub start_cluster: u16,
    pub size: u32,
    pub date_raw: u16,
    pub time_raw: u16,
}

fn trim_trailing_padding(v: &mut Vec<u8>) {
    while matches!(v.last(), Some(&0x20) | Some(&0x00)) {
        v.pop();
    }
}

impl DirEntry {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < ENTRY_SIZE {
            return None;
        }
        let first = bytes[0];
        let kind = match first {
            0x00 => EntryKind::Empty,
            0xE5 => EntryKind::Deleted,
            _ => EntryKind::Used,
        };

        // 主名: 0x00-0x07 (8バイト) + 0x0C-0x14 (9バイト)
        let mut name_bytes = Vec::with_capacity(17);
        name_bytes.extend_from_slice(&bytes[0x00..0x08]);
        name_bytes.extend_from_slice(&bytes[0x0C..0x15]);
        trim_trailing_padding(&mut name_bytes);

        if kind == EntryKind::Deleted && !name_bytes.is_empty() {
            name_bytes[0] = b'?';
        }

        let mut ext_bytes: Vec<u8> = bytes[0x08..0x0B].to_vec();
        trim_trailing_padding(&mut ext_bytes);

        let (name, _, _) = SHIFT_JIS.decode(&name_bytes);
        let (ext, _, _) = SHIFT_JIS.decode(&ext_bytes);

        let attr = Attr::from_bits_truncate(bytes[0x0B]);
        let time_raw = u16::from_le_bytes([bytes[0x16], bytes[0x17]]);
        let date_raw = u16::from_le_bytes([bytes[0x18], bytes[0x19]]);
        let start_cluster = u16::from_le_bytes([bytes[0x1A], bytes[0x1B]]);
        let size = u32::from_le_bytes([bytes[0x1C], bytes[0x1D], bytes[0x1E], bytes[0x1F]]);

        Some(DirEntry {
            kind,
            name: name.into_owned(),
            ext: ext.into_owned(),
            attr,
            start_cluster,
            size,
            date_raw,
            time_raw,
        })
    }

    pub fn display_name(&self) -> String {
        if self.ext.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.name, self.ext)
        }
    }

    pub fn date(&self) -> (u16, u8, u8) {
        let d = self.date_raw;
        let year = 1980 + ((d >> 9) & 0x7F);
        let month = ((d >> 5) & 0x0F) as u8;
        let day = (d & 0x1F) as u8;
        (year, month, day)
    }

    pub fn time(&self) -> (u8, u8, u8) {
        let t = self.time_raw;
        let hour = ((t >> 11) & 0x1F) as u8;
        let minute = ((t >> 5) & 0x3F) as u8;
        let second = ((t & 0x1F) as u8) * 2;
        (hour, minute, second)
    }
}
