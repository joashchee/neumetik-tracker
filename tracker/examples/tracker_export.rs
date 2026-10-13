//! Writes every module given (files, or folders of them) as MOD, S3M,
//! XM and IT, reads each back and plays its first seconds: what each
//! format lost, and any that won't read back.
//!
//!     cargo run --release --example tracker_export -- <files or folders>

use std::path::{Path, PathBuf};
use std::sync::Arc;

use neumetik_tracker::export::{export, Format};
use neumetik_tracker::{load, player};

fn walk(p: &Path, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path()).collect();
        v.sort();
        for e in v {
            walk(&e, out);
        }
    } else {
        out.push(p.to_path_buf());
    }
}

fn main() {
    let mut files = Vec::new();
    for a in std::env::args().skip(1) {
        walk(Path::new(&a), &mut files);
    }
    let (mut done, mut failed) = (0, 0);
    let mut lost_counts: std::collections::BTreeMap<String, usize> = Default::default();
    for f in &files {
        let Ok(data) = std::fs::read(f) else { continue };
        let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let Ok(m) = neumetik_tracker::load_with(&data, &name, &neumetik_tracker::beside(f)) else { continue };
        for to in [Format::Mod, Format::S3m, Format::Xm, Format::It] {
            let e = export(&m, to);
            match load(&e.bytes, &format!("x.{}", to.extension())) {
                Ok(back) => {
                    let audio = player::render(Arc::new(back), 22_050, 4.0, 0.0);
                    if audio.iter().any(|v| !v.is_finite()) {
                        println!("{name} as {}: it plays, but not finitely", to.name());
                        failed += 1;
                    } else {
                        done += 1;
                    }
                    for (what, _) in e.lost {
                        *lost_counts.entry(format!("{}: {what}", to.extension())).or_default() += 1;
                    }
                }
                Err(err) => {
                    println!("{name} as {}: doesn't read back: {err}", to.name());
                    failed += 1;
                }
            }
        }
    }
    println!("\nWhat was lost, by how many songs:");
    for (what, n) in &lost_counts {
        println!("{n:5}  {what}");
    }
    println!("\n{done} written and read back, {failed} failed");
}
