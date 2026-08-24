//! xdfcp: X68000 ディスクイメージ内のファイル/ディレクトリをホストFSへコピー
//!
//! 使い方:
//!   xdfcp disk.xdf:/MUSIC/STRANGE.ZMS ./out/        単一ファイル
//!   xdfcp -R disk.xdf:/MUSIC ./out/                 ディレクトリ再帰
//!   xdfcp -R disk.xdf:/ ./out/                      ルート全体
//!   xdfcp -R disk.hds:0:/GAMES ./out/               HDS パーティション

use anyhow::{anyhow, Result};
use clap::Parser;
use std::path::PathBuf;
use xdf_fs::direntry::Attr;
use xdf_fs::export::{sanitize_filename, sanitize_path, write_file};
use xdf_fs::fs::Filesystem;
use xdf_fs::spec::{with_filesystem, ImageSpec};
use xdf_fs::walker;

#[derive(Parser)]
#[command(about = "Copy files from an X68000 disk image (XDF/HDS/HDF) to host filesystem")]
struct Args {
    /// 入力 (path[:partition][:/inner/path])
    source: String,
    /// 出力先ディレクトリ
    dest: PathBuf,
    /// ディレクトリを再帰的にコピー
    #[arg(short = 'R', long)]
    recursive: bool,
    /// 詳細表示
    #[arg(short = 'v', long)]
    verbose: bool,
    /// 上書き禁止
    #[arg(short = 'n', long)]
    no_clobber: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let spec = ImageSpec::parse(&args.source)?;

    let copy_count = with_filesystem(&spec, |fs, inner_path| {
        let normalized = inner_path.trim_matches('/');
        if normalized.is_empty() {
            if !args.recursive {
                return Err(anyhow!("Copying root requires -R"));
            }
            copy_walk(fs, "", &args.dest, args.verbose, args.no_clobber)
        } else {
            let entry = fs.resolve(normalized)?;
            if entry.attr.contains(Attr::DIRECTORY) {
                if !args.recursive {
                    return Err(anyhow!(
                        "{} is a directory; use -R to copy recursively",
                        normalized
                    ));
                }
                copy_walk(fs, normalized, &args.dest, args.verbose, args.no_clobber)
            } else {
                let data = fs.read_file(&entry)?;
                let final_dest = if args.dest.is_dir() || ends_with_sep(&args.dest) {
                    let mut p = args.dest.clone();
                    p.push(sanitize_filename(&entry.display_name()));
                    p
                } else {
                    args.dest.clone()
                };
                if args.no_clobber && final_dest.exists() {
                    eprintln!("skip (exists): {}", final_dest.display());
                } else {
                    write_file(&final_dest, &data, &entry)?;
                    if args.verbose {
                        println!(
                            "{} -> {} ({} bytes)",
                            normalized,
                            final_dest.display(),
                            data.len()
                        );
                    }
                }
                Ok(1usize)
            }
        }
    })?;

    eprintln!("done: {} file(s) copied", copy_count);
    Ok(())
}

fn ends_with_sep(p: &std::path::Path) -> bool {
    let s = p.to_string_lossy();
    s.ends_with('/') || s.ends_with('\\')
}

fn copy_walk(
    fs: &Filesystem,
    base: &str,
    dest_root: &std::path::Path,
    verbose: bool,
    no_clobber: bool,
) -> Result<usize> {
    let mut count = 0usize;
    let mut error: Option<anyhow::Error> = None;
    walker::walk(fs, |item| {
        let rel = if base.is_empty() {
            Some(item.path.as_str())
        } else if item.path == base {
            Some("")
        } else if item.path.starts_with(base)
            && item.path.as_bytes().get(base.len()) == Some(&b'/')
        {
            Some(&item.path[base.len() + 1..])
        } else {
            None
        };
        let Some(rel) = rel else { return true };
        if rel.is_empty() && item.entry.attr.contains(Attr::DIRECTORY) {
            return true;
        }
        let dest = sanitize_path(dest_root, rel);
        if item.entry.attr.contains(Attr::DIRECTORY) {
            if let Err(e) = std::fs::create_dir_all(&dest) {
                error = Some(anyhow!("Cannot create dir {}: {}", dest.display(), e));
                return false;
            }
            if verbose {
                println!("DIR  {}/", dest.display());
            }
        } else {
            if no_clobber && dest.exists() {
                if verbose {
                    eprintln!("skip (exists): {}", dest.display());
                }
                return true;
            }
            match fs.read_file(item.entry) {
                Ok(data) => {
                    if let Err(e) = write_file(&dest, &data, item.entry) {
                        error = Some(e);
                        return false;
                    }
                    count += 1;
                    if verbose {
                        println!("FILE {} ({} bytes)", dest.display(), data.len());
                    }
                }
                Err(e) => {
                    error = Some(e);
                    return false;
                }
            }
        }
        true
    })?;
    if let Some(e) = error {
        return Err(e);
    }
    Ok(count)
}
