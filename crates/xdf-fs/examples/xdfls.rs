//! xdfls: X68000 ディスクイメージ内のファイル一覧を表示
//!
//! 使い方:
//!   xdfls disk.xdf                 ルートディレクトリを表示
//!   xdfls disk.xdf -R              サブディレクトリも再帰表示
//!   xdfls disk.xdf:/MUSIC          指定ディレクトリから表示
//!   xdfls disk.hds:0               HDS/HDF パーティション 0 のルート
//!   xdfls disk.hds:0:/MUSIC -R     パーティション + 内部パス + 再帰

use anyhow::Result;
use clap::Parser;
use xdf_fs::direntry::{Attr, DirEntry};
use xdf_fs::fs::Filesystem;
use xdf_fs::image::OpenedImage;
use xdf_fs::spec::{with_filesystem, ImageSpec};

#[derive(Parser)]
#[command(about = "List files in an X68000 disk image (XDF/HDS/HDF)")]
struct Args {
    /// path[:partition][:/inner/path]
    image: String,
    /// 再帰表示
    #[arg(short = 'R', long)]
    recursive: bool,
    /// 削除済みエントリも表示
    #[arg(long)]
    deleted: bool,
    /// パーティション一覧を表示して終了 (HDS/HDF のみ)
    #[arg(long)]
    partitions: bool,
}

fn attr_str(a: Attr) -> String {
    let mut s = String::with_capacity(7);
    s.push(if a.contains(Attr::DIRECTORY) { 'D' } else { '-' });
    s.push(if a.contains(Attr::READ_ONLY) { 'R' } else { '-' });
    s.push(if a.contains(Attr::HIDDEN)    { 'H' } else { '-' });
    s.push(if a.contains(Attr::SYSTEM)    { 'S' } else { '-' });
    s.push(if a.contains(Attr::VOLUME)    { 'V' } else { '-' });
    s.push(if a.contains(Attr::ARCHIVE)   { 'A' } else { '-' });
    s.push(if a.contains(Attr::LINK)      { 'L' } else { '-' });
    s
}

fn print_entry(e: &DirEntry, prefix: &str) {
    let (y, mo, d) = e.date();
    let (h, mi, _) = e.time();
    println!(
        "{}{}  {:>10}  {:04}-{:02}-{:02} {:02}:{:02}  {}{}",
        prefix,
        attr_str(e.attr),
        e.size,
        y, mo, d, h, mi,
        e.display_name(),
        if e.attr.contains(Attr::DIRECTORY) { "/" } else { "" },
    );
}

fn walk(fs: &Filesystem, entries: &[DirEntry], path: &str, recursive: bool, show_deleted: bool) -> Result<()> {
    println!("\n{}:", if path.is_empty() { "/" } else { path });
    for e in entries {
        if e.kind == xdf_fs::direntry::EntryKind::Deleted && !show_deleted {
            continue;
        }
        if e.name == "." || e.name == ".." {
            continue;
        }
        print_entry(e, "  ");
    }
    if recursive {
        for e in entries {
            if e.attr.contains(Attr::DIRECTORY)
                && e.name != "." && e.name != ".."
                && e.kind == xdf_fs::direntry::EntryKind::Used
            {
                let sub = fs.read_subdir(e.start_cluster)?;
                let new_path = format!("{}/{}", path, e.display_name());
                walk(fs, &sub, &new_path, recursive, show_deleted)?;
            }
        }
    }
    Ok(())
}

fn print_partitions(spec: &ImageSpec) -> Result<()> {
    let opened = OpenedImage::open(&spec.path)?;
    match opened {
        OpenedImage::Floppy(_) => {
            println!("Floppy (XDF) — single volume, no partitions");
        }
        OpenedImage::Hdd(hdd) => {
            println!(
                "{} — physical sector {} B, {} partition(s):",
                if hdd.phys_sec_size() == 512 { "HDS (SCSI)" } else { "HDF (SASI)" },
                hdd.phys_sec_size(),
                hdd.partitions().len()
            );
            for (i, p) in hdd.partitions().iter().enumerate() {
                println!(
                    "  [{}] {:<10} start={:>10} sectors  size={:>10} sectors  ({:>10} bytes)",
                    i,
                    p.name,
                    p.phys_start,
                    p.phys_count,
                    p.phys_count * hdd.phys_sec_size(),
                );
            }
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let spec = ImageSpec::parse(&args.image)?;

    if args.partitions {
        return print_partitions(&spec);
    }

    with_filesystem(&spec, |fs, inner_path| {
        eprintln!("BPB: {:?}", fs.bpb);
        eprintln!("phys_per_logi = {}, FAT = {:?}", fs.phys_per_logi, fs.fat.kind());

        let normalized = inner_path.trim_matches('/');
        let (entries, base_path) = if normalized.is_empty() {
            (fs.read_root_dir()?, "")
        } else {
            let entry = fs.resolve(normalized)?;
            if !entry.attr.contains(Attr::DIRECTORY) {
                // 単一ファイル指定: そのエントリだけ表示
                println!();
                print_entry(&entry, "");
                return Ok(());
            }
            (fs.read_subdir(entry.start_cluster)?, normalized)
        };
        walk(fs, &entries, base_path, args.recursive, args.deleted)?;
        Ok(())
    })
}

