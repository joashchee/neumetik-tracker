//! Reads every file under the folders given and says what each is:
//! `cargo run --release --example tracker_scan -- <folder>...`. For
//! checking the loaders against modules in the wild.

use std::path::Path;

fn walk(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let mut files = Vec::new();
    for arg in std::env::args().skip(1) {
        walk(Path::new(&arg), &mut files);
    }
    files.sort();
    let (mut ok, mut failed) = (0, 0);
    for f in files {
        let Ok(data) = std::fs::read(&f) else { continue };
        let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let shown = f.strip_prefix(f.parent().and_then(|p| p.parent()).unwrap_or(Path::new(""))).unwrap_or(&f).display().to_string();
        match neumetik_tracker::load_with(&data, &name, &neumetik_tracker::beside(&f)) {
            Ok(m) => {
                ok += 1;
                let frames: usize = m.samples.iter().map(|s| s.frames()).sum();
                println!(
                    "ok   {shown}: {} \"{}\" [{}] {} ch, {} pat, {} ord, {} smp ({} frames), {} ins",
                    m.format,
                    m.title,
                    m.tracker,
                    m.channels.len(),
                    m.patterns.len(),
                    m.orders.len(),
                    m.samples.len(),
                    frames,
                    m.instruments.len()
                );
            }
            Err(e) => {
                failed += 1;
                println!("FAIL {shown}: {e}");
            }
        }
    }
    println!("{ok} read, {failed} not");
}
