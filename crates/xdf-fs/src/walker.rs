//! ディレクトリ再帰トラバース
//!
//! ファイルシステム全体を歩いて、各エントリに対してコールバックを呼ぶ。
//! 検索 (`xdf-grep`) や再帰コピー (`xdfcp -R`) の共通基盤として使う。

use crate::direntry::{Attr, DirEntry, EntryKind};
use crate::fs::Filesystem;
use anyhow::Result;

/// 走査中の各エントリに渡される情報
pub struct WalkItem<'a> {
    /// '/'区切りの絶対パス (ルートからの相対、先頭'/'なし)
    pub path: String,
    /// このエントリ自身
    pub entry: &'a DirEntry,
}

/// コールバックの戻り値: trueなら続行、falseなら(そのディレクトリ以降を)中断
pub type Continue = bool;

/// ルートから再帰的に全エントリを訪問する。
///
/// 訪問順は深さ優先。ディレクトリ自体を先に訪問してからその中身に入る。
/// "." ".." と削除エントリはコールバックに渡さない。
pub fn walk<F>(fs: &Filesystem, mut cb: F) -> Result<()>
where
    F: FnMut(&WalkItem) -> Continue,
{
    let root = fs.read_root_dir()?;
    walk_dir(fs, &root, "", &mut cb)
}

fn walk_dir<F>(
    fs: &Filesystem,
    entries: &[DirEntry],
    prefix: &str,
    cb: &mut F,
) -> Result<()>
where
    F: FnMut(&WalkItem) -> Continue,
{
    for e in entries {
        if e.kind != EntryKind::Used {
            continue;
        }
        if e.name == "." || e.name == ".." {
            continue;
        }
        let path = if prefix.is_empty() {
            e.display_name()
        } else {
            format!("{}/{}", prefix, e.display_name())
        };
        let item = WalkItem { path: path.clone(), entry: e };
        if !cb(&item) {
            return Ok(());
        }
        if e.attr.contains(Attr::DIRECTORY) {
            let sub = fs.read_subdir(e.start_cluster)?;
            walk_dir(fs, &sub, &path, cb)?;
        }
    }
    Ok(())
}
