//! The synth and chip trackers, whose sounds are made as the song plays:
//! each read by neumetik-synth (`replay`), which plays it by its own
//! replayer; the module here shows it (its patterns, its instruments'
//! names as samples with no points) and keeps it in `Module::tune`.
//!
//! - AHX ("THX") and HivelyTracker ("HVL"): positions of a track a
//!   channel, each transposed; a pattern here a position.
//! - Future Composer 1.3 and 1.4: positions of a pattern a channel; its
//!   samples (not its waveforms) the module's, for the repository.
//! - SidMon 1: its rip's data found by its replayer's instructions; each
//!   channel's track rows laid out by their durations.
//! - SidMon 2: positions of a track a channel, its rows packed with runs
//!   of empty ones.
//! - BP SoundMon 2.0 and 2.2: steps of a 16-row track a channel.
//! - Delta Music 2: each channel its own list of 16-row blocks, looping on
//!   its own; a pattern here the longest list's step, each channel's own
//!   step beside it.
//! - Delta Music 1: the same, each channel's list with jumps followed.
//! - Digital Mugician 1 to 3: sub-songs of 64-row tracks a channel; each
//!   sub-song its own patterns and order list (7 channels in 2 and 3).
//! - Sonic Arranger: positions, each channel a run of the song's rows;
//!   each sub-song its span of the positions.
//! - InStereo! 1.0 and 2.0: laid out as Sonic Arranger is; Synthesis as
//!   InStereo! 1.0 (its file laid out as 1.0's).
//! - Voodoo Supreme Synthesizer: each channel's stream of notes (its
//!   calls and loops followed, to its jump back) laid out by tick, a row
//!   for as many ticks as every note's hold is a multiple of.
//! - Music Assembler: a rip, its data found by its replayer's
//!   instructions; each sub-song a pattern for each of the first
//!   channel's tracks (repeats their own), each channel's rows laid out
//!   by how long each is held.

use std::sync::Arc;

use neumetik_synth::{ahx, deltamusic1, deltamusic2, fc, instereo, mugician, musicassembler, sidmon1, sidmon2, sonic, soundmon, synthesis, voodoo};

use crate::io::{Error, Result};
use crate::formats::common::amiga_pan as common_pan;
use crate::model::*;

/// The patterns' note for a format's 1-based note on `base` (the
/// tracker's note for its first), none when out of range.
fn note_at(note: i32, base: i32) -> u8 {
    let n = note - 1 + base;
    if (i32::from(NOTE_MIN)..=i32::from(NOTE_MAX)).contains(&n) {
        n as u8
    } else {
        NOTE_NONE
    }
}

/// A sample standing for an instrument: its name, no points.
fn named(name: &str) -> Sample {
    Sample { name: name.to_string(), ..Sample::empty() }
}

/// The order lists of a song whose positions are its patterns, one a
/// subsong from its first position (and back to the song's restart, when
/// that's in it).
fn subsong_orders(m: &mut Module, starts: &[usize], positions: usize, restart: usize) {
    let seqs: Vec<Sequence> = starts
        .iter()
        .enumerate()
        .map(|(k, &s)| Sequence { name: if k == 0 { "Song".into() } else { format!("Song {}", k + 1) }, orders: (s..positions).map(|p| p as u16).collect(), restart: restart.saturating_sub(s) as u16 })
        .collect();
    m.orders = seqs[0].orders.clone();
    m.restart = seqs[0].restart;
    if seqs.len() > 1 {
        m.sequences = seqs;
    }
}

pub fn probe_ahx(d: &[u8]) -> bool {
    ahx::probe(d)
}

/// AHX's effects the tracker has a name for, for the screen.
fn ahx_fx(fx: u8, p: u8) -> (Fx, u8) {
    let decimal = (p >> 4) * 10 + (p & 0xF);
    match fx {
        0x1 => (Fx::PortaUp, p),
        0x2 => (Fx::PortaDown, p),
        0x3 => (Fx::TonePorta, p),
        0x5 => (Fx::TonePortaVol, p),
        0x7 => (Fx::Panning, (p as i8 as i16 + 128).clamp(0, 255) as u8),
        0xA => (Fx::VolSlide, p),
        0xB => (Fx::PositionJump, decimal),
        0xC if p <= 0x40 => (Fx::Volume, p),
        0xD => (Fx::PatternBreak, decimal),
        0xE if matches!(p >> 4, 1 | 2 | 0xA | 0xB | 0xC | 0xD) => (Fx::ModEx, p),
        0xF => (Fx::Speed, p),
        _ => (Fx::None, 0),
    }
}

pub fn load_ahx(d: &[u8]) -> Result<Module> {
    let song = ahx::Song::parse(d).ok_or_else(|| Error::Bad("its header or tracks are damaged".into()))?;
    let hvl = song.kind == ahx::Kind::Hvl;
    let nch = song.channels;
    let mut m = Module {
        title: song.title.clone(),
        format: if hvl { "hvl" } else { "ahx" }.into(),
        format_name: if hvl { "HivelyTracker" } else { "AHX" }.into(),
        tracker: match song.kind {
            ahx::Kind::Ahx1 => "AHX 1",
            ahx::Kind::Ahx2 => "AHX 2",
            ahx::Kind::Hvl => "HivelyTracker",
        }
        .into(),
        kind: Kind::Mod,
        speed: 6,
        tempo: (125 * song.speed_multiplier.max(1)) as u16,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        ..Default::default()
    };
    const LEFT: [u8; 5] = [128, 96, 64, 32, 0];
    const RIGHT: [u8; 5] = [128, 160, 193, 225, 255];
    m.channels = (0..nch).map(|c| Channel { pan: if matches!(c % 4, 0 | 3) { LEFT[song.stereo] } else { RIGHT[song.stereo] }, ..Default::default() }).collect();
    for p in 0..song.positions.len() {
        let mut pat = Pattern::new(song.track_length, nch);
        for c in 0..nch {
            let tr = i32::from(song.positions[p].transpose[c]);
            for row in 0..song.track_length {
                let s = song.step(p, c, row);
                let cell = pat.cell_mut(row, c).expect("in the pattern");
                // AHX's C-1 is the tracker's C-2 (ProTracker's C-1 is AHX's C-3).
                if s.note != 0 {
                    cell.note = note_at(i32::from(s.note) + tr, 25);
                }
                cell.instr = s.instrument;
                let (fx, param) = ahx_fx(s.fx, s.param);
                let (fx, param) = if fx == Fx::None { ahx_fx(s.fx2, s.param2) } else { (fx, param) };
                (cell.fx, cell.param) = (fx, param);
            }
        }
        m.patterns.push(Arc::new(pat));
    }
    subsong_orders(&mut m, &song.subsongs, song.positions.len(), song.restart);
    m.samples = song.instruments.iter().skip(1).map(|i| named(&i.name)).collect();
    m.message = song.instruments.iter().skip(1).map(|i| i.name.clone()).collect::<Vec<_>>().join("\n").trim_end().to_string();
    m.tune = Some(Tune(Arc::new(song)));
    m.tidy();
    Ok(m)
}

/// The patterns' note for an Amiga period (ProTracker's C-1, 856, is 49).
fn note_of_period(p: u16) -> u8 {
    if p == 0 {
        return NOTE_NONE;
    }
    let n = 49.0 + 12.0 * (856.0 / f64::from(p)).log2();
    (n.round() as i32).clamp(i32::from(NOTE_MIN), i32::from(NOTE_MAX)) as u8
}

/// A sample of 8-bit points from a replayer's memory.
fn amiga_points(mem: &[i8], at: usize, len: usize, loop_start: usize, loop_len: usize, name: String) -> Sample {
    let pcm: Vec<i16> = mem.get(at..at + len).unwrap_or_default().iter().map(|&x| i16::from(x) << 8).collect();
    let looped = if loop_len > 2 && loop_start < len { Loop::forward(loop_start as u32, (loop_start + loop_len).min(len) as u32) } else { Loop::default() };
    Sample { name, wave: Arc::new(Wave::new(8, 1, pcm)), c5: 8287, volume: 64, looped, ..Sample::empty() }
}

pub fn probe_fc(d: &[u8]) -> bool {
    fc::probe(d)
}

pub fn load_fc(d: &[u8]) -> Result<Module> {
    let song = fc::Song::parse(d).ok_or_else(|| Error::Bad("its header or sequences are damaged".into()))?;
    let mut m = Module {
        format: "fc".into(),
        format_name: "Future Composer".into(),
        tracker: if song.v13 { "Future Composer 1.3" } else { "Future Composer 1.4" }.into(),
        kind: Kind::Mod,
        speed: song.positions[0].speed.max(1),
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        ..Default::default()
    };
    m.channels = (0..4).map(|c| Channel { pan: common_pan(c), ..Default::default() }).collect();
    // The samples: each of the ten, or each of a set's twenty.
    for i in 0..10 {
        if let Some(Some(set)) = song.multi.get(i) {
            for (k, s) in set.iter().enumerate().filter(|(_, s)| s.present()) {
                m.samples.push(amiga_points(&song.mem, s.at, s.len, s.loop_start, s.loop_len, format!("Sample {} ({})", i + 1, k + 1)));
            }
        } else if song.sounds[i].present() {
            let s = song.sounds[i];
            m.samples.push(amiga_points(&song.mem, s.at, s.len, s.loop_start, s.loop_len, format!("Sample {}", i + 1)));
        }
    }
    for (p, pos) in song.positions.iter().enumerate() {
        // A pattern ends at its END mark (each channel's on its own; the first's shown).
        let rows = (0..32).find(|&r| song.row(p, 0, r).0 == 0x49).unwrap_or(32).max(1);
        let mut pat = Pattern::new(rows, 4);
        for c in 0..4 {
            let mut ended = false;
            for row in 0..rows {
                let (note, info) = song.row(p, c, row);
                if note == 0x49 {
                    ended = true;
                }
                if ended {
                    continue;
                }
                let cell = pat.cell_mut(row, c).expect("in the pattern");
                if note & 0x7F != 0 {
                    let n = (i32::from(note & 0x7F) + i32::from(pos.transpose[c])) as u8 & 0x7F;
                    cell.note = note_of_period(fc::period(usize::from(n), song.v13).clamp(113, 3424));
                    cell.instr = ((info & 0x3F) as i8).wrapping_add(pos.sound_transpose[c]) as u8 & 0x3F;
                    cell.instr = cell.instr.saturating_add(1);
                }
                if info & 0x80 != 0 && row + 1 < 32 {
                    let v = song.row(p, c, row + 1).1;
                    (cell.fx, cell.param) = if v <= 31 { (Fx::PortaUp, v) } else { (Fx::PortaDown, v & 0x1F) };
                }
            }
        }
        if pos.speed != 0 {
            let cell = pat.cell_mut(0, 0).expect("in the pattern");
            if cell.fx == Fx::None {
                (cell.fx, cell.param) = (Fx::Speed, pos.speed);
            }
        }
        m.patterns.push(Arc::new(pat));
    }
    m.orders = (0..song.positions.len() as u16).collect();
    m.tune = Some(Tune(Arc::new(song)));
    m.tidy();
    Ok(m)
}

pub fn probe_sidmon1(d: &[u8]) -> bool {
    sidmon1::probe(d)
}

pub fn load_sidmon1(d: &[u8]) -> Result<Module> {
    let song = sidmon1::Song::parse(d).ok_or_else(|| Error::Bad("its replayer or data can't be found".into()))?;
    let mut m = Module {
        format: "sid1".into(),
        format_name: "SidMon 1".into(),
        tracker: "SidMon 1".into(),
        kind: Kind::Mod,
        speed: (song.speed + 1).min(255) as u8,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        ..Default::default()
    };
    m.channels = (0..4).map(|c| Channel { pan: common_pan(c), ..Default::default() }).collect();
    let rows = (song.rows as usize).clamp(1, 256);
    for p in 0..song.positions() {
        let mut pat = Pattern::new(rows, 4);
        for c in 0..4 {
            let tr = song.sequences[c][p].transpose;
            for (at, r) in song.rows_at(p, c) {
                let Some(cell) = pat.cell_mut(at, c) else { continue };
                if r.note == -1 {
                    cell.note = NOTE_OFF;
                } else if r.note != 0 {
                    let n = i32::from(r.note.wrapping_add(tr));
                    let fine = song.instruments.get(usize::from(r.instrument).wrapping_sub(1)).map_or(0, |i| usize::from(i.finetune));
                    let per = song.periods.get((n.max(0) as usize) + fine * song.finetune_step).copied().unwrap_or(0);
                    cell.note = note_of_period(per);
                }
                cell.instr = r.instrument;
                (cell.fx, cell.param) = match r.effect {
                    0 => (Fx::None, 0),
                    2 => (Fx::Speed, r.param.saturating_add(1)),
                    3 => (Fx::None, 0),
                    _ if r.note > 0 => (Fx::TonePorta, r.param),
                    _ => (Fx::None, 0),
                };
            }
        }
        m.patterns.push(Arc::new(pat));
    }
    m.orders = (0..song.positions() as u16).collect();
    m.samples = song.samples.iter().filter(|s| s.len > 0).map(|s| amiga_points(&song.mem, s.at, s.len, s.loop_start.unwrap_or(0), s.loop_start.map_or(0, |l| s.len - l), if s.name.is_empty() { "Sample".into() } else { s.name.clone() })).collect();
    m.tune = Some(Tune(Arc::new(song)));
    m.tidy();
    Ok(m)
}

pub fn probe_sidmon2(d: &[u8]) -> bool {
    sidmon2::probe(d)
}

/// SidMon 2's effects the tracker has a name for, for the screen.
fn sidmon2_fx(e: u8, a: u8) -> (Fx, u8) {
    if e < 0x70 {
        return if e != 0 && a != 0 { (Fx::TonePorta, a) } else { (Fx::None, 0) };
    }
    match e & 0x0F {
        0x0 => (Fx::Arpeggio, a),
        0x1 => (Fx::PortaUp, a),
        0x2 => (Fx::PortaDown, a),
        0x3 => (Fx::VolSlide, (a.min(15)) << 4),
        0x4 => (Fx::VolSlide, a.min(15)),
        0xC => (Fx::Volume, a.min(64)),
        0xF => (Fx::Speed, a & 0x0F),
        _ => (Fx::None, 0),
    }
}

pub fn load_sidmon2(d: &[u8]) -> Result<Module> {
    let song = sidmon2::Song::parse(d).ok_or_else(|| Error::Bad("its blocks are damaged".into()))?;
    let mut m = Module {
        format: "sid2".into(),
        format_name: "SidMon 2".into(),
        tracker: "SidMon 2".into(),
        kind: Kind::Mod,
        speed: song.speed.max(1),
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        ..Default::default()
    };
    m.channels = (0..4).map(|c| Channel { pan: common_pan(c), ..Default::default() }).collect();
    for p in 0..song.positions {
        let mut pat = Pattern::new(64, 4);
        for c in 0..4 {
            let step = song.sequences[c][p];
            for (note, ins, e, a, row) in song.events(step.track) {
                let Some(cell) = pat.cell_mut(row, c) else { continue };
                if note != 0 {
                    cell.note = note_of_period(sidmon2::period((i32::from(note) + i32::from(step.note_transpose)).clamp(0, 72) as usize));
                }
                if ins != 0 {
                    cell.instr = (i32::from(ins) + i32::from(step.instrument_transpose)).clamp(1, 255) as u8;
                }
                (cell.fx, cell.param) = sidmon2_fx(e, a);
            }
        }
        m.patterns.push(Arc::new(pat));
    }
    m.orders = (0..song.positions as u16).collect();
    m.samples = song.samples.iter().map(|s| amiga_points(&song.mem, s.at, s.len, s.loop_start, s.loop_len, if s.name.is_empty() { "Sample".into() } else { s.name.clone() })).collect();
    m.tune = Some(Tune(Arc::new(song)));
    m.tidy();
    Ok(m)
}

pub fn probe_soundmon(d: &[u8]) -> bool {
    soundmon::probe(d)
}

pub fn load_soundmon(d: &[u8]) -> Result<Module> {
    let song = soundmon::Song::parse(d).ok_or_else(|| Error::Bad("its steps or tracks are damaged".into()))?;
    let mut m = Module {
        title: song.title.clone(),
        format: "bp".into(),
        format_name: "BP SoundMon".into(),
        tracker: if song.v22 { "BP SoundMon 2.2" } else { "BP SoundMon 2.0" }.into(),
        kind: Kind::Mod,
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        ..Default::default()
    };
    m.channels = (0..4).map(|c| Channel { pan: common_pan(c), ..Default::default() }).collect();
    for step in 0..song.steps[0].len() {
        let mut pat = Pattern::new(16, 4);
        for c in 0..4 {
            let st = song.steps[c][step];
            for row in 0..16 {
                let r = song.row(step, c, row);
                let cell = pat.cell_mut(row, c).expect("in the pattern");
                if r.note != 0 {
                    let n = i32::from(r.note) + if r.optional != 0xA || r.data & 0xF0 == 0 { i32::from(st.transpose) } else { 0 };
                    cell.note = note_of_period(soundmon::period((n + 35).clamp(0, 83) as usize));
                    if r.instrument != 0 {
                        let k = i32::from(r.instrument) + if r.optional != 0xA || r.data & 0x0F == 0 { i32::from(st.sound_transpose) } else { 0 };
                        cell.instr = if (1..=15).contains(&k) { k as u8 } else { r.instrument };
                    }
                }
                (cell.fx, cell.param) = match r.optional {
                    0x0 if r.data != 0 => (Fx::Arpeggio, r.data),
                    0x1 => (Fx::Volume, r.data.min(64)),
                    0x2 => (Fx::Speed, r.data),
                    0x4 => (Fx::FinePortaUp, r.data),
                    0x5 => (Fx::FinePortaDown, r.data),
                    0x6 if song.v22 => (Fx::Vibrato, r.data),
                    0x7 if song.v22 => (Fx::PositionJump, r.data),
                    0x9 | 0xD | 0xE | 0xF if r.data != 0 => (Fx::Arpeggio, r.data),
                    _ => (Fx::None, 0),
                };
            }
        }
        m.patterns.push(Arc::new(pat));
    }
    m.orders = (0..song.steps[0].len() as u16).collect();
    for ins in &song.instruments {
        m.samples.push(match ins {
            soundmon::Instrument::Sample { name, at, len, loop_start, loop_len, volume } => Sample { volume: (*volume).min(64) as u8, ..amiga_points(&song.mem, *at, *len, *loop_start, *loop_len, name.clone()) },
            soundmon::Instrument::Synth(_) => named("Synth"),
        });
    }
    m.tune = Some(Tune(Arc::new(song)));
    m.tidy();
    Ok(m)
}

pub fn probe_deltamusic2(d: &[u8]) -> bool {
    deltamusic2::probe(d)
}

pub fn load_deltamusic2(d: &[u8]) -> Result<Module> {
    let song = deltamusic2::Song::parse(d).ok_or_else(|| Error::Bad("its blocks or instruments are damaged".into()))?;
    let mut m = Module {
        format: "dm2".into(),
        format_name: "Delta Music 2".into(),
        tracker: "Delta Music 2".into(),
        kind: Kind::Mod,
        speed: (song.speed.max(0) as u8).saturating_add(1),
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        ..Default::default()
    };
    m.channels = (0..4).map(|c| Channel { pan: common_pan(c), ..Default::default() }).collect();
    for p in 0..song.positions() {
        let mut pat = Pattern::new(16, 4);
        for c in 0..4 {
            let Some((b, tr)) = song.step_at(c, p) else { continue };
            for (row, l) in song.blocks[b].iter().enumerate() {
                let cell = pat.cell_mut(row, c).expect("in the pattern");
                if l.note != 0 {
                    cell.note = note_of_period(deltamusic2::period((i32::from(l.note) + i32::from(tr)).clamp(0, 84) as usize));
                    cell.instr = l.instrument.saturating_add(1);
                }
                (cell.fx, cell.param) = match l.effect {
                    1 => (Fx::Speed, (l.arg & 0x0F) + 1),
                    3 => (Fx::PortaUp, l.arg),
                    4 => (Fx::PortaDown, l.arg),
                    5 => (Fx::TonePorta, l.arg),
                    6 => (Fx::Volume, l.arg & 0x3F),
                    7 => (Fx::GlobalVolume, (l.arg & 0x3F) * 2),
                    _ => (Fx::None, 0),
                };
            }
        }
        m.patterns.push(Arc::new(pat));
    }
    m.orders = (0..song.positions() as u16).collect();
    for ins in song.instruments.iter() {
        m.samples.push(match ins {
            Some(i) if i.is_sample => amiga_points(&song.mem, i.at, i.length, i.repeat_start, i.repeat_length, format!("Sample {}", i.number + 1)),
            Some(_) => named("Synth"),
            None => named(""),
        });
    }
    while m.samples.last().is_some_and(|s| s.name.is_empty()) {
        m.samples.pop();
    }
    m.tune = Some(Tune(Arc::new(song)));
    m.tidy();
    Ok(m)
}

pub fn probe_deltamusic1(d: &[u8]) -> bool {
    deltamusic1::probe(d)
}

pub fn load_deltamusic1(d: &[u8]) -> Result<Module> {
    let song = deltamusic1::Song::parse(d).ok_or_else(|| Error::Bad("its blocks or instruments are damaged".into()))?;
    let mut m = Module {
        format: "dm1".into(),
        format_name: "Delta Music 1".into(),
        tracker: "Delta Music 1".into(),
        kind: Kind::Mod,
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        ..Default::default()
    };
    m.channels = (0..4).map(|c| Channel { pan: common_pan(c), ..Default::default() }).collect();
    let walks: Vec<Vec<(usize, i8)>> = (0..4).map(|c| song.walk(c, 512)).collect();
    let n = walks.iter().map(Vec::len).max().unwrap_or(0).max(1);
    for p in 0..n {
        let mut pat = Pattern::new(16, 4);
        for c in 0..4 {
            let w = &walks[c];
            if w.is_empty() {
                continue;
            }
            let (b, tr) = w[p % w.len()];
            for (row, l) in song.blocks[b].iter().enumerate() {
                let cell = pat.cell_mut(row, c).expect("in the pattern");
                if l.note != 0 {
                    cell.note = note_of_period(deltamusic1::period((i32::from(l.note) + i32::from(tr)).clamp(0, 83) as usize));
                    cell.instr = l.instrument.saturating_add(1);
                }
                (cell.fx, cell.param) = match l.effect {
                    0x01 if l.arg != 0 => (Fx::Speed, l.arg),
                    0x02 => (Fx::PortaUp, l.arg),
                    0x03 => (Fx::PortaDown, l.arg),
                    0x09 => (Fx::TonePorta, l.arg),
                    0x0A => (Fx::Volume, l.arg.min(64)),
                    _ => (Fx::None, 0),
                };
            }
        }
        m.patterns.push(Arc::new(pat));
    }
    m.orders = (0..n as u16).collect();
    for ins in song.instruments.iter() {
        m.samples.push(match ins {
            Some(i) if i.is_sample => amiga_points(&song.mem, i.at, i.length.min(i.size), i.repeat_start, i.repeat_length, format!("Sample {}", i.number + 1)),
            Some(_) => named("Synth"),
            None => named(""),
        });
    }
    while m.samples.last().is_some_and(|s| s.name.is_empty()) {
        m.samples.pop();
    }
    m.tune = Some(Tune(Arc::new(song)));
    m.tidy();
    Ok(m)
}

pub fn probe_mugician(d: &[u8]) -> bool {
    mugician::probe(d)
}

pub fn load_mugician(d: &[u8]) -> Result<Module> {
    let song = mugician::Song::parse(d).ok_or_else(|| Error::Bad("its songs or tracks are damaged".into()))?;
    let nch = song.channels();
    let mut m = Module {
        format: "dmu".into(),
        format_name: "Digital Mugician".into(),
        tracker: if song.v3 { "Digital Mugician 3" } else if song.v2 { "Digital Mugician 2" } else { "Digital Mugician" }.into(),
        kind: Kind::Mod,
        speed: (song.subsongs[song.playable[0]].speed & 0x0F).max(1),
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        ..Default::default()
    };
    m.channels = (0..nch).map(|c| Channel { pan: if c >= 3 && nch == 7 { 0 } else { common_pan(c) }, ..Default::default() }).collect();
    let mut seqs = Vec::new();
    for (k, &sub) in song.playable.iter().enumerate() {
        let first = m.patterns.len() as u16;
        let len = song.subsongs[sub].length;
        for p in 0..len {
            let mut pat = Pattern::new(64, nch);
            for c in 0..nch {
                let (t, tr) = song.step(k, p, c);
                for (row, r) in song.tracks[usize::from(t)].iter().enumerate() {
                    let cell = pat.cell_mut(row, c).expect("in the pattern");
                    if r.note != 0 {
                        let fine = song.instruments.get(usize::from(r.instrument).wrapping_sub(1)).map_or(0, |i| usize::from(i.finetune));
                        let idx = (7 + i32::from(r.note) + i32::from(tr)).max(0) as usize + fine * 64;
                        cell.note = note_of_period(mugician::period(idx.min(1023)));
                        cell.instr = r.instrument;
                    }
                    (cell.fx, cell.param) = match r.effect {
                        1..=63 if r.param != 0 => (Fx::TonePorta, r.param),
                        67 if r.param != 0 && r.param <= 64 => (Fx::PatternBreak, 0),
                        68 => (Fx::Speed, r.param & 0x0F),
                        73 => (Fx::Arpeggio, r.param),
                        _ => (Fx::None, 0),
                    };
                }
            }
            m.patterns.push(Arc::new(pat));
        }
        let ss = song.subsongs[sub];
        seqs.push(Sequence { name: if song.names[sub].is_empty() { format!("Song {}", k + 1) } else { song.names[sub].clone() }, orders: (first..first + len as u16).collect(), restart: if ss.looped { ss.loop_position.min(len.saturating_sub(1)) as u16 } else { 0 } });
    }
    m.orders = seqs[0].orders.clone();
    m.restart = seqs[0].restart;
    if seqs.len() > 1 {
        m.sequences = seqs;
    }
    m.samples = song.samples.iter().enumerate().map(|(i, s)| amiga_points(&song.mem, s.at, s.len, s.loop_start.unwrap_or(0), s.loop_start.map_or(0, |l| s.len - l), format!("Sample {}", i + 1))).collect();
    m.tune = Some(Tune(Arc::new(song)));
    m.tidy();
    Ok(m)
}

pub fn probe_sonic(d: &[u8]) -> bool {
    sonic::probe(d)
}

pub fn load_sonic(d: &[u8]) -> Result<Module> {
    let song = sonic::Song::parse(d).ok_or_else(|| Error::Bad("its blocks are damaged".into()))?;
    let first = song.subsongs[0];
    let mut m = Module {
        format: "sa".into(),
        format_name: "Sonic Arranger".into(),
        tracker: "Sonic Arranger".into(),
        kind: Kind::Mod,
        speed: first.speed.clamp(1, 255) as u8,
        tempo: (first.tempo * 5 / 2).clamp(32, 999),
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        ..Default::default()
    };
    m.channels = (0..4).map(|c| Channel { pan: common_pan(c), ..Default::default() }).collect();
    let rows = usize::from(first.rows).clamp(1, 256);
    for p in 0..song.positions.len() {
        let mut pat = Pattern::new(rows, 4);
        for c in 0..4 {
            let (_, st, nt) = song.positions[p][c];
            for row in 0..rows {
                let l = song.line(p, c, row);
                let cell = pat.cell_mut(row, c).expect("in the pattern");
                match l.note {
                    0 => {}
                    0x7F => cell.note = NOTE_CUT,
                    0x80 => {}
                    n => {
                        let n = if l.no_note_transpose { i32::from(n) } else { i32::from(n) + i32::from(nt) };
                        cell.note = note_of_period(sonic::period(n.clamp(0, 108) as usize));
                    }
                }
                if l.instrument != 0 {
                    let k = if l.no_sound_transpose { i32::from(l.instrument) } else { i32::from(l.instrument) + i32::from(st) };
                    cell.instr = k.clamp(0, 255) as u8;
                }
                (cell.fx, cell.param) = match l.effect {
                    0x0 if l.arg != 0 => (Fx::Arpeggio, l.arg),
                    0x4 => (Fx::Vibrato, l.arg),
                    0x7 => (Fx::TonePorta, l.arg),
                    0xA => (Fx::VolSlide, if (l.arg as i8) < 0 { (l.arg as i8).unsigned_abs().min(15) } else { l.arg.min(15) << 4 }),
                    0xB => (Fx::PositionJump, l.arg),
                    0xC => (Fx::Volume, l.arg.min(64)),
                    0xD => (Fx::PatternBreak, 0),
                    0xF => (Fx::Speed, l.arg),
                    _ => (Fx::None, 0),
                };
            }
        }
        m.patterns.push(Arc::new(pat));
    }
    let n = song.positions.len() as u16;
    let seqs: Vec<Sequence> = song
        .subsongs
        .iter()
        .enumerate()
        .map(|(k, s)| {
            let (a, b) = (s.first.min(n - 1), s.last.min(n - 1).max(s.first.min(n - 1)));
            Sequence { name: format!("Song {}", k + 1), orders: (a..=b).collect(), restart: s.restart.saturating_sub(a).min(b - a) }
        })
        .collect();
    m.orders = seqs[0].orders.clone();
    m.restart = seqs[0].restart;
    if seqs.len() > 1 {
        m.sequences = seqs;
    }
    let mut samples: Vec<Sample> = song.samples.iter().enumerate().map(|(i, &(at, len))| amiga_points(&song.mem, at, len, 0, 0, format!("Sample {}", i + 1))).collect();
    // Each sample's loop, as the first instrument playing it has it.
    for ins in song.instruments.iter().filter(|i| !i.synth) {
        if let Some(s) = samples.get_mut(usize::from(ins.waveform)) {
            if !s.looped.on() && ins.repeat > 1 {
                let (ls, ll) = (u32::from(ins.length) * 2, u32::from(ins.repeat) * 2);
                s.looped = Loop::forward(ls, ls + ll).clamped(s.wave.frames() as u32);
            }
            if s.name.starts_with("Sample") && !ins.name.is_empty() {
                s.name = ins.name.clone();
            }
        }
    }
    m.samples = samples;
    m.tune = Some(Tune(Arc::new(song)));
    m.tidy();
    Ok(m)
}

pub fn probe_instereo(d: &[u8]) -> bool {
    instereo::probe(d)
}

pub fn load_instereo(d: &[u8]) -> Result<Module> {
    let song = instereo::Song::parse(d).ok_or_else(|| Error::Bad("its blocks are damaged".into()))?;
    let mut m = instereo_layout(&song, false);
    (m.format, m.format_name, m.tracker) = ("is".into(), "InStereo!".into(), if song.v2() { "InStereo! 2.0" } else { "InStereo! 1.0" }.into());
    m.tune = Some(Tune(Arc::new(song)));
    m.tidy();
    Ok(m)
}

pub fn probe_synthesis(d: &[u8]) -> bool {
    synthesis::probe(d)
}

pub fn load_synthesis(d: &[u8]) -> Result<Module> {
    let song = synthesis::Song::parse(d).ok_or_else(|| Error::Bad("its blocks are damaged".into()))?;
    let mut m = instereo_layout(&song.song, true);
    (m.format, m.format_name, m.tracker) = ("syn".into(), "Synthesis".into(), if d.starts_with(b"Synth4.0") { "Synthesis 4.0" } else { "Synthesis 4.2" }.into());
    m.tune = Some(Tune(Arc::new(song)));
    m.tidy();
    Ok(m)
}

/// InStereo!'s module and Synthesis's (laid out as InStereo! 1.0's).
fn instereo_layout(song: &instereo::Song, synthesis: bool) -> Module {
    let v2 = song.v2();
    let first = song.subsongs[0];
    let mut m = Module {
        title: song.title.clone(),
        kind: Kind::Mod,
        speed: first.speed.max(1),
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        ..Default::default()
    };
    m.channels = (0..4).map(|c| Channel { pan: common_pan(c), ..Default::default() }).collect();
    let rows = usize::from(first.rows).clamp(1, 256);
    for p in 0..song.positions.len() {
        let mut pat = Pattern::new(rows, 4);
        for c in 0..4 {
            let (_, st, nt) = song.positions[p][c];
            for row in 0..rows {
                let l = song.line(p, c, row);
                let cell = pat.cell_mut(row, c).expect("in the pattern");
                match l.note {
                    0 | 0x80 => {}
                    0x7F => cell.note = NOTE_CUT,
                    n => {
                        let n = if l.no_note_transpose { i32::from(n) } else { i32::from(n) + i32::from(nt) };
                        cell.note = note_of_period(sonic::period(n.clamp(0, 108) as usize));
                    }
                }
                if l.instrument != 0 {
                    let k = if l.no_sound_transpose { i32::from(l.instrument) } else { i32::from(l.instrument) + i32::from(st) };
                    cell.instr = (k & 0x3F).max(1) as u8;
                }
                (cell.fx, cell.param) = match (v2, l.effect) {
                    _ if synthesis => match l.effect {
                        0x8 => (Fx::Speed, l.arg),
                        0xF => (Fx::Volume, l.arg.min(64)),
                        _ => (Fx::None, 0),
                    },
                    (true, 0x0) if l.arg != 0 => (Fx::Arpeggio, l.arg),
                    (true, 0x7) => (Fx::TonePorta, l.arg),
                    (true, 0xB) => (Fx::PositionJump, l.arg),
                    (true, 0xC) => (Fx::Volume, l.arg.min(64)),
                    (true, 0xD) => (Fx::PatternBreak, 0),
                    (false, 0x7) => (Fx::Volume, l.arg & 0x3F),
                    (_, 0xF) => (Fx::Speed, l.arg),
                    _ => (Fx::None, 0),
                };
            }
        }
        m.patterns.push(Arc::new(pat));
    }
    let n = song.positions.len() as u16;
    let seqs: Vec<Sequence> = song
        .subsongs
        .iter()
        .enumerate()
        .map(|(k, s)| {
            let (a, b) = (s.first.min(n - 1), s.last.min(n - 1).max(s.first.min(n - 1)));
            Sequence { name: format!("Song {}", k + 1), orders: (a..=b).collect(), restart: s.restart.saturating_sub(a).min(b - a) }
        })
        .collect();
    m.orders = seqs[0].orders.clone();
    m.restart = seqs[0].restart;
    if seqs.len() > 1 {
        m.sequences = seqs;
    }
    let spans: Vec<(usize, usize)> = match &song.kind {
        instereo::Kind::V1 { samples, .. } => samples.clone(),
        instereo::Kind::V2 { data, .. } => data.clone(),
    };
    m.samples = spans.iter().enumerate().map(|(i, &(at, len))| amiga_points(&song.mem, at, len, 0, 0, song.names.get(i).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| format!("Sample {}", i + 1)))).collect();
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small AHX 2 song: one position, a track of two notes, one instrument.
    fn tiny_ahx() -> Vec<u8> {
        let mut d = b"THX\x01".to_vec();
        d.extend_from_slice(&[0, 0]); // the names' offset, set below
        d.extend_from_slice(&[0x00, 1, 0, 0, 4, 1, 1, 0]); // flag, positions, restart, track length, tracks, instruments, subsongs
        d.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0]); // position 0: track 1 on channel 0, none elsewhere
        d.extend_from_slice(&[0; 12]); // track 0
        // Track 1: C-3 instrument 1, then nothing, E-3, nothing.
        let step = |note: u8, ins: u8| [note << 2 | ins >> 4, (ins & 0xF) << 4, 0];
        for (n, i) in [(25, 1), (0, 0), (29, 1), (0, 0)] {
            d.extend_from_slice(&step(n, i));
        }
        // Instrument 1: volume 64, a 32-point wave, a quick envelope, a play list of a triangle.
        d.extend_from_slice(&[64, 3, 1, 64, 1, 64, 10, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1]);
        d.extend_from_slice(&[0x00, 0x80, 0, 0]); // waveform 1 (triangle), no note
        let names = d.len() as u16;
        d[4..6].copy_from_slice(&names.to_be_bytes());
        d.extend_from_slice(b"tiny\0lead\0");
        d
    }

    #[test]
    fn ahx_reads_and_plays() {
        let d = tiny_ahx();
        assert!(probe_ahx(&d));
        let m = load_ahx(&d).unwrap();
        assert_eq!(m.title, "tiny");
        assert_eq!(m.samples[0].name, "lead");
        assert_eq!(m.patterns.len(), 1);
        assert_eq!(m.patterns[0].cell(0, 0).note, 49);
        assert_eq!(m.patterns[0].cell(2, 0).note, 53);
        let audio = crate::player::render(Arc::new(m.clone()), 22_050, 1.0, 0.0);
        let rms = (audio.iter().map(|v| v * v).sum::<f32>() / audio.len() as f32).sqrt();
        assert!(rms > 0.02, "it plays: {rms}");
        // Through a UMF: the song kept, played the same.
        let refs: Vec<_> = m.samples.iter().map(|_| None).collect();
        let bytes = crate::umf::write(&m, &refs, &[], &Default::default(), true);
        let back = crate::umf::read(&bytes, &mut |_| None).expect("reads back").module;
        assert_eq!(back.tune, m.tune);
        assert_eq!(crate::player::render(Arc::new(back), 22_050, 1.0, 0.0), audio);
    }
}

pub fn probe_musicassembler(d: &[u8]) -> bool {
    musicassembler::probe(d)
}

pub fn load_musicassembler(d: &[u8]) -> Result<Module> {
    let song = musicassembler::Song::parse(d).ok_or_else(|| Error::Bad("its replayer or data can't be found".into()))?;
    let mut m = Module {
        format: "ma".into(),
        format_name: "Music Assembler".into(),
        tracker: "Music Assembler".into(),
        kind: Kind::Mod,
        speed: song.subsongs[0].speed.max(1),
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        ..Default::default()
    };
    m.channels = (0..4).map(|c| Channel { pan: common_pan(c), ..Default::default() }).collect();
    let mut seqs = Vec::new();
    for sub in 0..song.subsongs.len() {
        let first = m.patterns.len() as u16;
        let steps: Vec<Vec<(usize, u8)>> = (0..4).map(|c| song.steps(sub, c)).collect();
        let n = steps[0].len().max(1);
        let mut instrument = [0usize; 4];
        for p in 0..n {
            let rows: Vec<Vec<musicassembler::Row>> = steps.iter().map(|s| if s.is_empty() { Vec::new() } else { song.rows(s[p % s.len()].0) }).collect();
            let len = rows.iter().map(|r| r.iter().map(|x| usize::from(x.delay) + 1).sum::<usize>()).max().unwrap_or(1).clamp(1, 256);
            let mut pat = Pattern::new(len, 4);
            for c in 0..4 {
                let Some(&(_, tr)) = steps[c].get(p % steps[c].len().max(1)) else { continue };
                let mut at = 0;
                for r in &rows[c] {
                    if let Some(k) = r.instrument {
                        instrument[c] = usize::from(k).min(song.instruments.len() - 1);
                    }
                    if let Some(cell) = pat.cell_mut(at, c) {
                        let ins = &song.instruments[instrument[c]];
                        let smp = usize::from(ins.sample).min(song.samples.len() - 1);
                        if r.release {
                            cell.note = NOTE_OFF;
                        } else if let Some(nt) = r.note {
                            let key = if song.samples[smp].synth() { 0 } else { ins.key_wave_rate };
                            let i = usize::from(nt.wrapping_add(tr).wrapping_add(key)).min(47);
                            cell.note = note_of_period(musicassembler::PERIODS[i]);
                            cell.instr = (smp + 1) as u8;
                            (cell.fx, cell.param) = match r.portamento {
                                0 => (Fx::None, 0),
                                x if x & 0x80 != 0 => (Fx::PortaDown, x & 0x7F),
                                x => (Fx::PortaUp, x),
                            };
                        }
                    }
                    at += usize::from(r.delay) + 1;
                }
            }
            m.patterns.push(Arc::new(pat));
        }
        seqs.push(Sequence { name: if sub == 0 { "Song".into() } else { format!("Song {}", sub + 1) }, orders: (first..first + n as u16).collect(), restart: 0 });
    }
    m.orders = seqs[0].orders.clone();
    if seqs.len() > 1 {
        m.sequences = seqs;
    }
    m.samples = song
        .samples
        .iter()
        .map(|s| {
            let name = if s.name.chars().all(|c| c == '-') { String::new() } else { s.name.clone() };
            if s.synth() || s.len <= 1 {
                named(&name)
            } else {
                let lp = if s.loop_len != 0 { (s.len - s.loop_len.min(s.len)) * 2 } else { 0 };
                amiga_points(&song.mem, s.at, s.len * 2, lp, if s.loop_len != 0 { s.loop_len.min(s.len) * 2 } else { 0 }, name)
            }
        })
        .collect();
    m.tune = Some(Tune(Arc::new(song)));
    m.tidy();
    Ok(m)
}

pub fn probe_voodoo(d: &[u8]) -> bool {
    voodoo::probe(d)
}

pub fn load_voodoo(d: &[u8]) -> Result<Module> {
    let song = voodoo::Song::parse(d).ok_or_else(|| Error::Bad("its tracks or tables are damaged".into()))?;
    let (grain, _) = song.rows(0);
    let mut m = Module {
        format: "vss".into(),
        format_name: "Voodoo Supreme Synthesizer".into(),
        tracker: "Voodoo Supreme Synthesizer".into(),
        kind: Kind::Mod,
        speed: grain as u8,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        ..Default::default()
    };
    m.channels = (0..4).map(|c| Channel { pan: common_pan(c), ..Default::default() }).collect();
    let mut seqs = Vec::new();
    for sub in 0..song.subsongs.len() {
        let (grain, rows) = song.rows(sub);
        let n = rows.div_ceil(64).min(256);
        let first = m.patterns.len();
        let mut pats: Vec<Pattern> = (0..n).map(|_| Pattern::new(64, 4)).collect();
        for c in 0..4 {
            for (t, note, _) in song.timeline(sub, c, 100_000) {
                let r = t / grain;
                let Some(cell) = pats.get_mut(r / 64).and_then(|p| p.cell_mut(r % 64, c)) else { continue };
                cell.note = if note == 0xFF { NOTE_CUT } else { note_of_period(voodoo::period(usize::from(note))) };
            }
        }
        m.patterns.extend(pats.into_iter().map(Arc::new));
        seqs.push(Sequence { name: format!("Song {}", sub + 1), orders: (first as u16..(first + n) as u16).collect(), restart: 0 });
    }
    m.orders = seqs[0].orders.clone();
    if seqs.len() > 1 {
        m.sequences = seqs;
    }
    // Its samples, each once, in the file's order.
    let mut seen: Vec<(usize, usize)> = song.subsongs.iter().flat_map(|s| s.items.iter()).filter_map(|i| if let voodoo::Item::Sample(at, len) = i { Some((*at, *len)) } else { None }).collect();
    seen.sort_unstable();
    seen.dedup();
    m.samples = seen.iter().enumerate().map(|(k, &(at, len))| amiga_points(&song.mem, at, len, 0, 0, format!("Sample {}", k + 1))).collect();
    m.tune = Some(Tune(Arc::new(song)));
    m.tidy();
    Ok(m)
}
