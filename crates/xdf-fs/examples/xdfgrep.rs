//! xdfgrep: ファイル名パターンで X68000 イメージ内を検索
//!
//! 使い方:
//!   xdfgrep disk.xdf "*.ZMS"
//!   xdfgrep disk.hds:0 -i "love.*"
//!   xdfgrep disk.hds:0:/MUSIC "*.ZMS"   ← 指定パス配下のみ検索

use anyhow::Result;
use clap::Parser;
use xdf_fs::direntry::Attr;
use xdf_fs::spec::{with_filesystem, ImageSpec};
use xdf_fs::walker;

#[derive(Parser)]
#[command(about = "Search files in an X68000 disk image (XDF/HDS/HDF) by glob pattern")]
struct Args {
    /// 入力 (path[:partition][:/inner/path])
    image: String,
    /// グロブパターン (例: "*.ZMS", "SONG*.*")
    pattern: String,
    /// 大文字小文字を無視
    #[arg(short = 'i', long)]
    ignore_case: bool,
    /// ディレクトリも対象に含める
    #[arg(short = 'd', long)]
    include_dirs: bool,
    /// ファイルサイズも表示
    #[arg(short = 'l', long)]
    long: bool,
}

/// 簡易グロブマッチ ('*' と '?' のみサポート)
fn glob_match(pattern: &str, name: &str, ignore_case: bool) -> bool {
    let p: Vec<char> = if ignore_case {
        pattern.to_lowercase().chars().collect()
    } else {
        pattern.chars().collect()
    };
    let n: Vec<char> = if ignore_case {
        name.to_lowercase().chars().collect()
    } else {
        name.chars().collect()
    };
    glob_inner(&p, 0, &n, 0)
}

fn glob_inner(p: &[char], pi: usize, n: &[char], ni: usize) -> bool {
    if pi == p.len() {
        return ni == n.len();
    }
    match p[pi] {
        '*' => {
            for k in ni..=n.len() {
                if glob_inner(p, pi + 1, n, k) {
                    return true;
                }
            }
            false
        }
        '?' => ni < n.len() && glob_inner(p, pi + 1, n, ni + 1),
        c => ni < n.len() && n[ni] == c && glob_inner(p, pi + 1, n, ni + 1),
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let spec = ImageSpec::parse(&args.image)?;

    with_filesystem(&spec, |fs, inner_path| {
        let scope_prefix = inner_path.trim_matches('/').to_string();
        walker::walk(fs, |item| {
            // スコープフィルタ: scope_prefix 配下のみ
            if !scope_prefix.is_empty() {
                if item.path != scope_prefix
                    && !(item.path.starts_with(&scope_prefix)
                        && item.path.as_bytes().get(scope_prefix.len()) == Some(&b'/'))
                {
                    return true;
                }
            }
            let is_dir = item.entry.attr.contains(Attr::DIRECTORY);
            if is_dir && !args.include_dirs {
                return true;
            }
            let name = item.entry.display_name();
            if glob_match(&args.pattern, &name, args.ignore_case) {
                if args.long {
                    println!(
                        "{:>10}  {}{}",
                        item.entry.size,
                        item.path,
                        if is_dir { "/" } else { "" }
                    );
                } else {
                    println!("{}{}", item.path, if is_dir { "/" } else { "" });
                }
            }
            true
        })?;
        Ok(())
    })
}
