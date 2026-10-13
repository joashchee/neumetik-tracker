//! Damages every module given (cut short, bytes flipped) and checks
//! that reading and playing them never panics: `cargo run --release
//! --example tracker_fuzz -- <folder>...`.

use std::path::Path;
use std::sync::Arc;

fn walk(p: &Path, out: &mut Vec<std::path::PathBuf>) {
    if p.is_dir() {
        for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
            walk(&e.path(), out);
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
    files.sort();
    let mut seed = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0x9E37_79B9u32);
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let (mut tried, mut read) = (0, 0);
    for f in &files {
        let Ok(data) = std::fs::read(f) else { continue };
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let companion = neumetik_tracker::beside(f);
        let mut variants: Vec<Vec<u8>> = Vec::new();
        for cut in [7, 3, 2] {
            variants.push(data[..data.len() * (cut - 1) / cut].to_vec());
        }
        for _ in 0..6 {
            let mut v = data.clone();
            for _ in 0..(1 + rand() % 24) {
                let i = rand() as usize % v.len().max(1);
                if let Some(b) = v.get_mut(i) {
                    *b = rand() as u8;
                }
            }
            // Mostly in the headers, where it matters.
            for _ in 0..4 {
                let i = rand() as usize % v.len().min(2048).max(1);
                if let Some(b) = v.get_mut(i) {
                    *b = rand() as u8;
                }
            }
            variants.push(v);
        }
        for v in variants {
            tried += 1;
            let r = std::panic::catch_unwind(|| {
                if let Ok(m) = neumetik_tracker::load_with(&v, &name, &companion) {
                    let mut p = neumetik_tracker::Player::new(Arc::new(m), 22050);
                    let (mut l, mut r) = (vec![0f32; 22050], vec![0f32; 22050]);
                    for _ in 0..3 {
                        p.render(&mut l, &mut r);
                    }
                    assert!(l.iter().chain(&r).all(|x| x.is_finite()), "not finite");
                    true
                } else {
                    false
                }
            });
            match r {
                Ok(true) => read += 1,
                Ok(false) => {}
                Err(_) => println!("PANIC on a damaged {name}"),
            }
        }
    }
    println!("{tried} damaged files tried, {read} still read, no panics unless listed above");
}
