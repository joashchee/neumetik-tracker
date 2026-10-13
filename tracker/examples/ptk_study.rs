//! What a ProTrekkr or NoiseTrekker song holds, as its replayer reads it:
//! its tracks' settings, its instruments' synths and splits, its mix
//! settings, the tracks it switches off. `cargo run --example ptk_study
//! -- <file>...`.
//!
//! - RENDER=file (and SECS=n): writes the replayer's own output there
//!   instead, 44.1 kHz stereo, raw 32-bit floats, to compare with a
//!   reference render. SOLO=track plays that track alone; TRACE=1 says
//!   on stderr where each row starts.
//! - CELLS=position:first-last:track: that track's rows as stored.

fn main() {
    for a in std::env::args().skip(1) {
        let Ok(data) = std::fs::read(&a) else { continue };
        let m = match neumetik_tracker::load(&data, &a) {
            Ok(m) => m,
            Err(e) => {
                println!("{a}: {e}");
                continue;
            }
        };
        let Some(s) = m.ptk.as_deref() else {
            println!("{a}: not a ProTrekkr song");
            continue;
        };
        if let Ok(path) = std::env::var("RENDER") {
            let secs: f64 = std::env::var("SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(30.0);
            let mut mach = neumetik_synth::ptk::Machine::new(m.ptk.clone().unwrap());
            if let Some(solo) = std::env::var("SOLO").ok().and_then(|v| v.parse::<usize>().ok()) {
                for t in 0..16 {
                    mach.set_muted(t, t != solo);
                }
            }
            let n = (secs * 44_100.0) as usize;
            let (mut l, mut r) = (vec![0f32; n], vec![0f32; n]);
            if std::env::var("TRACE").is_ok() {
                let mut last = (usize::MAX, usize::MAX);
                for i in 0..n {
                    mach.render_native(&mut l[i..i + 1], &mut r[i..i + 1]);
                    if mach.position() != last {
                        last = mach.position();
                        eprintln!("{:.3}s frame {i}: position {} row {}", i as f64 / 44_100.0, last.0, last.1);
                    }
                }
            } else {
                mach.render_native(&mut l, &mut r);
            }
            let mut out = Vec::with_capacity(n * 8);
            for i in 0..n {
                out.extend_from_slice(&l[i].to_le_bytes());
                out.extend_from_slice(&r[i].to_le_bytes());
            }
            std::fs::write(&path, out).expect("written");
            continue;
        }
        if let Ok(rows) = std::env::var("CELLS") {
            // CELLS=position:first-last:track
            let v: Vec<usize> = rows.split([':', '-']).filter_map(|x| x.parse().ok()).collect();
            let (pos, a0, b0, tr) = (v[0], v[1], v[2], v[3]);
            let (p, _) = s.pattern_at(pos);
            if let Some(p) = p {
                for row in a0..=b0 {
                    if let Some(c) = p.cell(row, tr) {
                        println!("{row:3}: notes {:?} instrs {:?} vol {} pan {} fx {:?}", &c.notes[..4], &c.instrs[..4], c.volume, c.panning, c.fx);
                    }
                }
            }
            continue;
        }
        println!("== {a} ({})", s.version as char);
        println!("bpm {} tpb {} volume {} shuffle {} tracks {} length {}", s.bpm, s.tpb, s.volume, s.shuffle, s.song_tracks, s.length);
        println!("reverb {} thr {} decays {:?} fb {} cut {} res {} damp {} stereo {}", s.reverb, s.reverb_threshold, s.reverb_decays, s.reverb_feedback, s.reverb_cutoff, s.reverb_resonance, s.reverb_damp, s.reverb_stereo);
        println!("chorus {:?} {:?} {:?} comp {:?}", s.chorus_delay, s.chorus_feedback, s.chorus_filter, s.comp);
        for (i, t) in s.tracks.iter().enumerate().take(s.song_tracks) {
            println!(
                "t{i}: cut {} in {} pan {} ftype {} res {} dist {} {}/{} rsend {} csend {} poly {} notes {} fx {} vol {} eq {:?} lfo {:?} flg {:?} mute {} comp {} {}/{}",
                t.cutoff, t.inertia, t.pan, t.filter, t.resonance, t.distortion, t.dist_threshold, t.dist_clamp, t.reverb_send, t.chorus_send, t.polyphony, t.notes, t.effects, t.volume, t.eq, t.lfo, t.flanger, t.muted, t.compress, t.comp_threshold, t.comp_ratio
            );
        }
        for (i, ins) in s.instruments.iter().enumerate() {
            let splits: Vec<String> = ins
                .splits
                .iter()
                .enumerate()
                .filter_map(|(k, sp)| sp.as_ref().map(|sp| format!("{k}:{}@{} loop {}-{} t{} ft {} amp {} dec {} {}ch len {}", sp.name, sp.base_note, sp.loop_start, sp.loop_end, sp.loop_type, sp.finetune, sp.amplify, sp.decay, 1 + usize::from(sp.right.is_some()), sp.len())))
                .collect();
            if ins.name.is_empty() && splits.is_empty() && ins.synth_prg == 0 {
                continue;
            }
            println!("i{i} {:?} prg {} vol {} beat {} {} {:?}", ins.name, ins.synth_prg, ins.volume, ins.beat_sync, ins.beat_lines, splits);
            if ins.synth_prg != 0 {
                println!("   {:?}", ins.synth);
            }
        }
        for (p, a) in s.active.iter().enumerate().take(s.length) {
            if a.iter().take(s.song_tracks).any(|on| !on) {
                println!("position {p}: tracks on {:?}", a.iter().take(s.song_tracks).map(|&b| u8::from(b)).collect::<Vec<_>>());
            }
        }
        for (u, t) in s.tb303.iter().enumerate() {
            println!("303[{u}] sel {} tune {} cut {} res {} env {} dec {} acc {} wave {} scale {} vol {}", t.selected, t.tune, t.cutoff, t.resonance, t.envmod, t.decay, t.accent, t.waveform, t.scale, t.volume);
        }
    }
}
