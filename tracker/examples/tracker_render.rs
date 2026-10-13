//! Plays modules through and reports each: how long it plays, its peak
//! and loudness, any silence or damage. With OUT=dir, writes each to a
//! WAV file there to listen to. `cargo run --release --example
//! tracker_render -- <file or folder>...`.

use std::path::Path;
use std::sync::Arc;

fn walk(p: &Path, out: &mut Vec<std::path::PathBuf>) {
    if p.is_dir() {
        let Ok(entries) = std::fs::read_dir(p) else { return };
        let mut v: Vec<_> = entries.flatten().map(|e| e.path()).collect();
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
    let out = std::env::var("OUT").ok();
    let rate = 44_100;
    for f in files {
        let Ok(data) = std::fs::read(&f) else { continue };
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let Ok(m) = neumetik_tracker::load_with(&data, &name, &neumetik_tracker::beside(&f)) else { continue };
        let started = std::time::Instant::now();
        let audio = neumetik_tracker::player::render(Arc::new(m), rate, 900.0, 0.5);
        let took = started.elapsed().as_secs_f64();
        let secs = audio.len() as f64 / 2.0 / rate as f64;
        let bad = audio.iter().any(|v| !v.is_finite());
        let peak = audio.iter().fold(0f32, |a, v| a.max(v.abs()));
        let rms = (audio.iter().map(|v| (v * v) as f64).sum::<f64>() / audio.len().max(1) as f64).sqrt();
        println!(
            "{name}: {:.0}:{:04.1}, peak {:.2}, rms {:.3}{}, {:.0}x real time",
            (secs / 60.0).floor(),
            secs % 60.0,
            peak,
            rms,
            if bad { " NOT FINITE" } else if peak < 0.001 { " SILENT" } else { "" },
            secs / took.max(1e-9)
        );
        if let Some(dir) = &out {
            let _ = std::fs::write(Path::new(dir).join(format!("{name}.wav")), neumetik_tracker::wav::write(&audio, rate));
        }
    }
}
