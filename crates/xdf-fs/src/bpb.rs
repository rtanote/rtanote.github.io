//! BIOS Parameter Block (BPB) の解釈
//!
//! Human68kは基本的にMS-DOS互換のBPBを使う。
//! XDF (2HD/1232KB) の標準値:
//!   bytes_per_sector = 1024 (※注意: PC-DOSの512ではなく1024が標準)
//!   sectors_per_cluster = 1
//!   reserved_sectors = 1
//!   num_fats = 2
//!   root_entries = 192
//!   total_sectors = 1232
//!   sectors_per_fat = 2
//!
//! ただしファイルとしてのXDFは512バイト/セクタで保存されることが多く、
//! その場合は論理セクタ(1024) = 物理セクタ2個分として扱う。
//! ここではまずBPBを素直に読む。

use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
pub struct Bpb {
    pub bytes_per_sector: u16,
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u16,
    pub num_fats: u8,
    pub root_entries: u16,
    pub total_sectors: u32,
    pub sectors_per_fat: u16,
    pub sectors_per_track: u16,
    pub num_heads: u16,
}

impl Bpb {
    /// X68000 標準 2HD 1232KB フロッピーのデフォルト BPB
    ///
    /// Hudson DIM (Hudson Soft 製ディスクイメージ作成ツール) 等で作られた
    /// 「BPB が壊れているが本体レイアウトは標準 XDF 互換」のディスク用 fallback。
    pub const fn default_2hd_1232k() -> Self {
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

    /// フロッピーディスクの BPB パース。標準 XDF (MS-DOS互換) で失敗した場合、
    /// **Hudson DIM 互換ディスク** (OEM 領域に "Hudson soft" シグネチャ) を検出して
    /// 2HD 1232KB のデフォルト値で代用する。
    ///
    /// Hudson DIM (実体は OEM 領域を 16 バイトに拡張した XDF) は BPB の数値部分が
    /// 標準 BPB と互換でないが、ディスク本体のレイアウト (boot=sector0、FAT=sector1-4、
    /// root=sector5-、1024B/sector) は XDF 標準に従う。詳細は
    /// `tests/data/Dennou083B.img` のサンプル解析を参照。
    pub fn parse_floppy(boot_sector: &[u8]) -> Result<Self> {
        match Self::parse(boot_sector) {
            Ok(bpb) => Ok(bpb),
            Err(e) => {
                if Self::looks_like_hudson_dim(boot_sector) {
                    Ok(Self::default_2hd_1232k())
                } else {
                    Err(e)
                }
            }
        }
    }

    /// OEM 領域に Hudson Soft の DIM シグネチャ "Hudson soft" があるかを判定。
    /// 標準 XDF の OEM (0x03-) と Hudson DIM の OEM (0x02-) の両方をチェック。
    pub fn looks_like_hudson_dim(boot_sector: &[u8]) -> bool {
        if boot_sector.len() < 0x12 {
            return false;
        }
        // Hudson DIM の場合: jmp 2 バイト + OEM 16 バイト ("Hudson soft 2.00" 等)
        // 標準 XDF の場合: jmp 3 バイト + OEM 8 バイト ("X68IPL30" 等)
        // どちらの位置にあっても "Hudson soft" を見つけられるよう少し広めに探す
        let needle = b"Hudson soft";
        boot_sector
            .windows(needle.len())
            .take(8) // 0x02 か 0x03 で始まることを想定、最大 0x09 までスキャン
            .any(|w| w == needle)
    }

    /// ブートセクタ(LBA 0)の先頭からBPBを読む
    pub fn parse(boot_sector: &[u8]) -> Result<Self> {
        if boot_sector.len() < 36 {
            return Err(anyhow!("Boot sector too short"));
        }
        // オフセットはMS-DOS BPB標準
        // 0x00-0x02: ジャンプ命令 (X68kではIPLコード)
        // 0x03-0x0A: OEM name
        // 0x0B-0x0C: bytes per sector
        let bytes_per_sector = u16::from_le_bytes([boot_sector[0x0B], boot_sector[0x0C]]);
        let sectors_per_cluster = boot_sector[0x0D];
        let reserved_sectors = u16::from_le_bytes([boot_sector[0x0E], boot_sector[0x0F]]);
        let num_fats = boot_sector[0x10];
        let root_entries = u16::from_le_bytes([boot_sector[0x11], boot_sector[0x12]]);
        let total_sectors_16 = u16::from_le_bytes([boot_sector[0x13], boot_sector[0x14]]);
        // 0x15: media descriptor (FE=2HD, F9=2HC, F0=2HQ など)
        let sectors_per_fat = u16::from_le_bytes([boot_sector[0x16], boot_sector[0x17]]);
        let sectors_per_track = u16::from_le_bytes([boot_sector[0x18], boot_sector[0x19]]);
        let num_heads = u16::from_le_bytes([boot_sector[0x1A], boot_sector[0x1B]]);

        let total_sectors = if total_sectors_16 != 0 {
            total_sectors_16 as u32
        } else {
            // 32bit版 (0x20-0x23) ... 大容量HDD用
            u32::from_le_bytes([
                boot_sector[0x20],
                boot_sector[0x21],
                boot_sector[0x22],
                boot_sector[0x23],
            ])
        };

        // 簡易サニティチェック
        if !matches!(bytes_per_sector, 256 | 512 | 1024 | 2048) {
            return Err(anyhow!(
                "Suspicious bytes_per_sector: {}",
                bytes_per_sector
            ));
        }
        if num_fats == 0 || num_fats > 2 {
            return Err(anyhow!("Suspicious num_fats: {}", num_fats));
        }

        Ok(Bpb {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            num_fats,
            root_entries,
            total_sectors,
            sectors_per_fat,
            sectors_per_track,
            num_heads,
        })
    }

    /// FAT領域の開始セクタ (1番目のFAT)
    pub fn fat_start(&self) -> u32 {
        self.reserved_sectors as u32
    }

    /// ルートディレクトリ領域の開始セクタ
    pub fn root_dir_start(&self) -> u32 {
        self.fat_start() + (self.num_fats as u32) * (self.sectors_per_fat as u32)
    }

    /// ルートディレクトリが占めるセクタ数
    pub fn root_dir_sectors(&self) -> u32 {
        let bytes = (self.root_entries as u32) * 32; // 1エントリ32バイト
        (bytes + self.bytes_per_sector as u32 - 1) / self.bytes_per_sector as u32
    }

    /// データ領域(クラスタ2から始まる)の開始セクタ
    pub fn data_start(&self) -> u32 {
        self.root_dir_start() + self.root_dir_sectors()
    }

    /// クラスタ番号(2始まり)に対応するセクタ番号
    pub fn cluster_to_sector(&self, cluster: u16) -> u32 {
        self.data_start() + (cluster as u32 - 2) * self.sectors_per_cluster as u32
    }

    /// Human68k HDD パーティション (HDS/HDF) のブートセクタから BPB を読む。
    ///
    /// XDF (MS-DOS互換 BPB) との違い:
    /// - jump が **2 バイト** (m68k `BRA.S 0x60 xx`)
    /// - OEM 文字列が **16 バイト** (MS-DOS は 8 バイト)
    /// - 多バイト整数が **ビッグエンディアン** (m68k 慣例)
    /// - `num_fats` が offset **0x15** (u8) ※MS-DOS は 0x10
    /// - `reserved_sectors` が offset **0x16-0x17** (BE u16) ※MS-DOS は 0x0E (LE u16)
    /// - `sectors_per_fat` が **u8** (MS-DOS は u16)。最大 256 セクタ。
    /// - `total_sectors_32` が offset **0x1E** (MS-DOS は 0x20)
    ///
    /// 詳細は `docs/hdd-format.md` の「3. パーティション内 BPB」参照。
    /// `sectors_per_track` / `num_heads` は HDD では意味を持たないため 0 を返す。
    pub fn parse_hdd(boot_sector: &[u8]) -> Result<Self> {
        if boot_sector.len() < 0x22 {
            return Err(anyhow!("Boot sector too short for Human68k HDD BPB"));
        }
        // 0x00-0x01: jump (m68k BRA.S) — 値の妥当性は検証しない
        // 0x02-0x11: OEM (16 B) — 上位レイヤで参照したくなったらフィールド化
        let bytes_per_sector = u16::from_be_bytes([boot_sector[0x12], boot_sector[0x13]]);
        let sectors_per_cluster = boot_sector[0x14];
        let num_fats = boot_sector[0x15];
        let reserved_sectors = u16::from_be_bytes([boot_sector[0x16], boot_sector[0x17]]);
        let root_entries = u16::from_be_bytes([boot_sector[0x18], boot_sector[0x19]]);
        let total_sectors_16 = u16::from_be_bytes([boot_sector[0x1A], boot_sector[0x1B]]);
        // 0x1C: media descriptor — 構造体には取り込まない
        let sectors_per_fat = boot_sector[0x1D] as u16; // u8 を u16 に拡張

        let total_sectors = if total_sectors_16 != 0 {
            total_sectors_16 as u32
        } else {
            u32::from_be_bytes([
                boot_sector[0x1E],
                boot_sector[0x1F],
                boot_sector[0x20],
                boot_sector[0x21],
            ])
        };

        if !matches!(bytes_per_sector, 256 | 512 | 1024 | 2048) {
            return Err(anyhow!(
                "Suspicious bytes_per_sector in HDD BPB: {}",
                bytes_per_sector
            ));
        }
        if num_fats == 0 || num_fats > 2 {
            return Err(anyhow!("Suspicious num_fats in HDD BPB: {}", num_fats));
        }

        Ok(Bpb {
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            num_fats,
            root_entries,
            total_sectors,
            sectors_per_fat,
            sectors_per_track: 0,
            num_heads: 0,
        })
    }
}
