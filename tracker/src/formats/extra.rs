//! Smaller formats with public descriptions: TCB Tracker (Atari ST),
//! EasyTrax (.etx), Chuck Biscuits' CBA, SoundSmith and MegaTracker
//! (Apple IIgs, their waves beside them), FM Tracker (.fmt, OPL2),
//! His Master's Noise (NoiseTracker with wavetable "Mupp" sounds),
//! Coconizer (Archimedes), the Magnetic Fields Packer (its samples
//! beside it), A.M. Composer (.amc), Editeur Musical Sequentiel (.ems),
//! JamCracker (.jam) and AMOS's music banks (.abk). Described by
//! OpenMPT's loaders (BSD: Load_tcb, Load_etx, Load_cba, Load_ss,
//! Load_fmt), libxmp's (MIT: hmn, coco, mfp, abk) and, for A.M. Composer,
//! EMS, JamCracker and AMOS, their replayers.

use std::sync::Arc;

use super::common::{amiga_pan, finetune_c5, mod_effect, mod_ex_to_s3m, period_to_note, relative_c5, C5_PERIOD};
use super::Companion;
use crate::io::{bad, decode, Pcm, Reader, Result};
use crate::model::*;
use neumetik_synth::sequence::{SynthSound, VOL_JMP, WAVE_JMP};

/// A ProTracker cell (four bytes) as a cell.
fn mod_cell(b: &[u8]) -> Cell {
    let period = u16::from(b[0] & 0x0F) << 8 | u16::from(b[1]);
    let mut c = Cell { note: period_to_note(period).unwrap_or(0), instr: (b[0] & 0xF0) | (b[2] >> 4), ..Default::default() };
    (c.fx, c.param) = mod_effect(b[2] & 0x0F, b[3], false);
    c
}

fn mod_module(format: &str, name: &str, tracker: &str, nch: usize) -> Module {
    Module {
        format: format.into(),
        format_name: name.into(),
        tracker: tracker.into(),
        kind: Kind::Mod,
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: (0..nch).map(|i| Channel { pan: amiga_pan(i), ..Default::default() }).collect(),
        ..Default::default()
    }
}

// ---- TCB Tracker ----
//
// "AN COOL." (or the beta's "AN COOL!"), the pattern count, the speed
// (16 less it), 128 orders and their count; then (new format) whether
// notes use Amiga frequencies, 16 instrument names, 16 values the pitch
// bend effects use; 64-row patterns of four channels, a byte for the
// note (octave and semitone) and one for instrument and effect; then 16
// samples' volumes, loop and place, unsigned 8-bit.

pub fn probe_tcb(d: &[u8]) -> bool {
    d.len() >= 144 && (&d[..8] == b"AN COOL." || &d[..8] == b"AN COOL!") && d[12] <= 15 && d[13] == 0 && d[142] <= 127 && d[143] == 0 && d[14..142].iter().all(|&o| o < 128) && u32::from_be_bytes([d[8], d[9], d[10], d[11]]) <= 128
}

pub fn load_tcb(d: &[u8]) -> Result<Module> {
    let new = d[7] == b'.';
    let npat = u32::from_be_bytes([d[8], d[9], d[10], d[11]]) as usize;
    let mut m = mod_module("tcb", "TCB Tracker", if new { "TCB Tracker 1.0 to 2.0" } else { "TCB Tracker (beta)" }, 4);
    m.speed = 16 - d[12];
    m.orders = d[14..14 + usize::from(d[142]).max(1)].iter().map(|&o| u16::from(o)).collect();
    let mut r = Reader::at(d, 144);
    let amiga = if new { r.u16be()? != 0 } else { false };
    let names: Vec<String> = (0..16).map(|_| r.text(8)).collect::<Result<_>>()?;
    let mut special = [0i16; 16];
    if new {
        for s in special.iter_mut() {
            *s = r.i16be()?;
        }
    }
    let offset = if amiga { 0 } else { 3 };
    for _ in 0..npat {
        let mut p = Pattern::new(64, 4);
        for c in p.cells.iter_mut() {
            let (note, ie) = (r.u8()?, r.u8()?);
            if (0x10..=0x3B).contains(&note) {
                c.note = (NOTE_MIDDLE_C as i32 - 24 + i32::from(note >> 4) * 12 + i32::from(note & 15) + offset) as u8;
                c.instr = (ie >> 4) + 1;
            } else if note != 0 {
                return bad("a note is out of range");
            }
            match ie & 15 {
                0 | 0xB | 0xC | 0xE | 0xF => {}
                0xD => (c.fx, c.param) = (Fx::PatternBreak, 0),
                n => {
                    let v = i32::from(special[usize::from(n)]);
                    if v > 0 {
                        (c.fx, c.param) = (Fx::PortaDown, ((v + 16) / 32).min(255) as u8);
                    } else if v < 0 {
                        (c.fx, c.param) = (Fx::PortaUp, ((-v + 16) / 32).min(255) as u8);
                    }
                }
            }
        }
        m.patterns.push(Arc::new(p));
    }
    let start = r.pos;
    r.skip(4)?;
    let h1 = r.bytes(64)?;
    let h2 = r.bytes(128)?;
    for i in 0..16 {
        let vol = h1[i * 4].min(127) / 2;
        let ls = u32::from(u16::from_be_bytes([h1[i * 4 + 2], h1[i * 4 + 3]]));
        let off = u32::from_be_bytes(h2[i * 8..i * 8 + 4].try_into().unwrap()) as usize;
        let len = u32::from_be_bytes(h2[i * 8 + 4..i * 8 + 8].try_into().unwrap()) as usize;
        let mut s = Sample { name: names[i].clone(), volume: vol, ..Sample::empty() };
        if !amiga {
            s.c5 = finetune_c5(5);
        }
        if len > 1 {
            let at = start.saturating_add(off).min(d.len());
            let pcm = decode(&d[at..(at + len).min(d.len())], Pcm::U8, len);
            if ls > 0 && (ls as usize) < len {
                s.looped = Loop::forward(len as u32 - ls, len as u32);
            }
            s.wave = Arc::new(Wave::new(8, 1, pcm));
        }
        m.samples.push(s);
    }
    m.tidy();
    Ok(m)
}

// ---- EasyTrax ----
//
// "EASYTRAX 1.0" 01 00, the tempo, the last pattern, and where the
// orders, patterns, sample headers and sample data are; four channels
// of four-byte cells (note, volume, instrument); 128 samples (name,
// place, length, loop start, rate, transpose and finetune), unsigned
// 8-bit. Note FF in the first channel ends the pattern; FE turns the
// volume down (not kept).

pub fn probe_etx(d: &[u8]) -> bool {
    d.len() >= 32 + 1024 && &d[..14] == b"EASYTRAX 1.0\x01\x00" && d[14] > 0 && d[15] <= 127
}

pub fn load_etx(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 14);
    let tempo = r.u8()?;
    let last = usize::from(r.u8()?);
    let (ord, pat, heads, data) = (r.u32le()? as usize, r.u32le()? as usize, r.u32le()? as usize, r.u32le()? as usize);
    let mut m = mod_module("etx", "EasyTrax", "EasyTrax 1.0", 4);
    m.kind = Kind::S3m;
    m.tempo = u16::from(tempo).max(32);
    m.orders = d.get(ord..ord + 1024).unwrap_or(&[]).iter().take_while(|&&o| o != 0xFF).map(|&o| u16::from(o)).collect();
    let mut r = Reader::at(d, pat);
    for _ in 0..=last {
        if r.left() < 1024 {
            break;
        }
        let mut p = Pattern::new(64, 4);
        let mut end = 64;
        for row in 0..64 {
            for ch in 0..4 {
                let b = r.bytes(4)?;
                let (note, vol, ins) = (b[0], b[1], b[2]);
                if note == 0xFF && ch == 0 && end == 64 {
                    end = row.max(1);
                } else if (1..=96).contains(&note) {
                    let c = &mut p.cells[row * 4 + ch];
                    c.note = NOTE_MIDDLE_C - 24 + note;
                    c.instr = ins + 1;
                    (c.vol, c.volp) = (Vol::Volume, ((u16::from(vol.min(127)) + 1) / 2) as u8);
                }
            }
        }
        m.patterns.push(Arc::new(if end < 64 { p.with_rows(end) } else { p }));
    }
    let mut h = Reader::at(d, heads);
    for _ in 0..128 {
        let Ok(name) = h.text(13) else { break };
        let off = h.u32le()? as usize;
        let len = h.u32le()? as usize;
        let ls = h.u32le()?;
        let rate = h.u32le()?;
        let tr = h.i8()?;
        let ft = h.i8()?;
        h.skip(1)?;
        let at = data.saturating_add(off).min(d.len());
        let pcm = decode(&d[at..at.saturating_add(len).min(d.len())], Pcm::U8, len);
        let c5 = (f64::from(rate.max(1)) * 2f64.powf((f64::from(tr) * 100.0 + f64::from(ft)) / 1200.0)).round() as u32;
        m.samples.push(Sample { name, c5, looped: if ls != u32::MAX { Loop::forward(ls, len as u32) } else { Loop::default() }, wave: Arc::new(Wave::new(8, 1, pcm)), ..Sample::empty() });
    }
    m.tidy();
    Ok(m)
}

// ---- Chuck Biscuits / Black Artist ----
//
// "CBA" F9, the title, 1A, the message's length, channels, last
// pattern, orders, samples, speed, tempo, 32 pannings, 255 orders; the
// samples' headers; patterns of five-byte cells (instrument, note,
// volume, command, parameter: MOD's commands one up, then the extended
// ones as commands of their own); delta-coded 8-bit samples; the message.

pub fn probe_cba(d: &[u8]) -> bool {
    d.len() >= 332 && &d[..4] == b"CBA\xF9" && d[36] == 0x1A && (1..=32).contains(&d[39]) && d[43] > 0 && d[44] >= 32
}

pub fn load_cba(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 4);
    let title = r.text(32)?;
    r.skip(1)?;
    let msg_len = usize::from(r.u16le()?);
    let nch = usize::from(r.u8()?);
    let last = usize::from(r.u8()?);
    let norders = usize::from(r.u8()?);
    let nsmp = usize::from(r.u8()?);
    let speed = r.u8()?;
    let tempo = r.u8()?;
    let pans = r.bytes(32)?.to_vec();
    let orders = r.bytes(255)?;
    let mut m = mod_module("cba", "Chuck Biscuits / Black Artist", "CBA", nch);
    m.title = title;
    m.kind = Kind::S3m;
    m.speed = speed;
    m.tempo = u16::from(tempo);
    m.orders = orders[..norders.min(255)]
        .iter()
        .map(|&o| match o {
            0xFF => ORDER_END,
            0xFE => ORDER_SKIP,
            o => u16::from(o),
        })
        .collect();
    for (i, c) in m.channels.iter_mut().enumerate() {
        c.pan = (u16::from(pans[i]) * 2).min(255) as u8;
    }
    let mut lens = Vec::new();
    for _ in 0..nsmp {
        let name = r.text(32)?;
        let flags = r.u8()?;
        let vol = r.u8()?;
        let rate = r.u16le()?;
        let len = r.u32le()? as usize;
        let (ls, le) = (r.u32le()?, r.u32le()?);
        lens.push(len);
        m.samples.push(Sample { name, volume: vol.min(64), c5: u32::from(rate.max(1)), looped: if flags & 8 != 0 { Loop::forward(ls, le) } else { Loop::default() }, ..Sample::empty() });
    }
    for _ in 0..=last {
        let mut p = Pattern::new(64, nch);
        for c in p.cells.iter_mut() {
            let b = r.bytes(5)?;
            let (ins, note, vol, cmd, prm) = (b[0], b[1], b[2], b[3], b[4]);
            c.instr = ins;
            c.note = match note {
                255 => NOTE_CUT,
                1..=96 => NOTE_MIDDLE_C - 49 + note,
                _ => 0,
            };
            if vol > 0 {
                (c.vol, c.volp) = (Vol::Volume, vol.min(65) - 1);
            }
            let (fx, p) = match cmd {
                1..=0x0E => mod_effect(cmd - 1, prm, false),
                0x18 => (Fx::Retrig, prm),
                0x10..=0x1E => (Fx::ModEx, (cmd - 0x0F) << 4 | prm.min(15)),
                0x1F => (Fx::Speed, prm),
                0x20 => (Fx::Tempo, prm),
                _ => (Fx::None, 0),
            };
            (c.fx, c.param) = if fx == Fx::ModEx { mod_ex_to_s3m(p) } else { (fx, p) };
        }
        m.patterns.push(Arc::new(p));
    }
    for (s, len) in m.samples.iter_mut().zip(lens) {
        s.wave = Arc::new(Wave::new(8, 1, decode(r.bytes_upto(len), Pcm::D8, len)));
    }
    m.message = crate::io::message(r.bytes_upto(msg_len));
    m.tidy();
    Ok(m)
}

// ---- SoundSmith and MegaTracker ----
//
// "SONGOK" (or "IAN9OK", "IAN92a" for MegaTracker), the pattern
// buffer's size, the speed, 15 instruments (name, volume), the orders;
// then three buffers of 14-channel rows: notes, instruments and
// effects, parameters; then the channels' panning. The waves are in a
// DOC RAM file beside the song (named as it with .D, .W, .DOC or .WB,
// or DOC.DATA) or ASIF files named as the instruments: unsigned 8-bit,
// cut at the first zero (where the Ensoniq chip stopped).

pub fn probe_ss(d: &[u8]) -> bool {
    if d.len() < 600 || !(d.starts_with(b"SONGOK") || d.starts_with(b"IAN9OK") || d.starts_with(b"IAN92a")) {
        return false;
    }
    let buf = usize::from(u16::from_le_bytes([d[6], d[7]]));
    buf >= 14 * 64 && (buf % (14 * 64) == 0 || buf == 0x8000) && u16::from_le_bytes([d[8], d[9]]) <= 15
}

/// A wave of the Ensoniq's, from its two oscillators' wave lists.
fn ss_wave(data: &[u8], lists: &[u8]) -> (Wave, Loop) {
    let half = data.len() / 2;
    let len = data[half..].iter().position(|&b| b == 0).map_or(data.len(), |p| half + p);
    let (a_addr, a_mode, b_addr, b_mode) = (lists[1], (lists[3] >> 1) & 3, lists[7], (lists[9] >> 1) & 3);
    let looped = if a_mode == 0 || (a_mode == 1 && b_mode == 3) || (a_mode == 3 && b_mode != 2) {
        Loop::forward(u32::from(b_addr.wrapping_sub(a_addr)) * 256, len as u32)
    } else {
        Loop::default()
    };
    (Wave::new(8, 1, decode(&data[..len], Pcm::U8, len)), looped)
}

pub fn load_ss(d: &[u8], name: &str, companion: Companion) -> Result<Module> {
    let buf = usize::from(u16::from_le_bytes([d[6], d[7]]));
    let speed = u16::from_le_bytes([d[8], d[9]]) as u8;
    let mut m = mod_module("ss", "SoundSmith / MegaTracker", if d.starts_with(b"SONG") { "SoundSmith" } else { "MegaTracker" }, 14);
    m.speed = speed.max(1);
    let mut names = Vec::new();
    for i in 0..15 {
        let at = 20 + i * 30;
        let n = usize::from(d[at]).min(21);
        let vol = d[at + 24];
        names.push(crate::io::text(&d[at + 1..at + 1 + n]));
        m.samples.push(Sample { name: names[i].clone(), volume: ((u16::from(vol) + 1) / 4).min(64) as u8, ..Sample::empty() });
    }
    let norders = usize::from(d[470]).min(128);
    m.orders = d[472..472 + norders].iter().map(|&o| u16::from(o)).collect();
    let notes = d.get(600..600 + buf).unwrap_or(&[]);
    let fx = d.get(600 + buf..600 + 2 * buf).unwrap_or(&[]);
    let params = d.get(600 + 2 * buf..600 + 3 * buf).unwrap_or(&[]);
    let foot = d.get(600 + 3 * buf..600 + 3 * buf + 30);
    if let Some(f) = foot {
        let pans: Vec<u16> = (0..14).map(|i| u16::from_le_bytes([f[i * 2], f[i * 2 + 1]])).collect();
        if u16::from_le_bytes([f[28], f[29]]) != 0 && pans.windows(2).any(|w| w[0] != w[1]) {
            for (c, &p) in m.channels.iter_mut().zip(&pans) {
                c.pan = if p == 0 { 64 } else { 192 };
            }
        } else {
            for c in m.channels.iter_mut() {
                c.pan = 128;
            }
        }
    }
    for pat in 0..buf / (14 * 64) {
        let mut p = Pattern::new(64, 14);
        for row in 0..64 {
            for ch in 0..14 {
                let i = (pat * 64 + row) * 14 + ch;
                let (Some(&n), Some(&ie), Some(&prm)) = (notes.get(i), fx.get(i), params.get(i)) else { continue };
                let c = &mut p.cells[row * 14 + ch];
                match n {
                    1..=96 => c.note = NOTE_MIDDLE_C - 36 + n,
                    128 => c.note = NOTE_CUT,
                    129 => (c.fx, c.param) = (Fx::PatternBreak, 0),
                    _ => {}
                }
                c.instr = ie >> 4;
                match ie & 15 {
                    0 if prm != 0 => (c.fx, c.param) = (Fx::Arpeggio, prm),
                    3 => (c.fx, c.param) = (Fx::Volume, (prm / 4).min(64)),
                    5 => (c.fx, c.param) = (Fx::ModEx, 0xA0 | prm.min(15)),
                    6 => (c.fx, c.param) = (Fx::ModEx, 0xB0 | prm.min(15)),
                    0xF => (c.fx, c.param) = (Fx::Speed, prm.min(15)),
                    _ => {}
                }
            }
        }
        m.patterns.push(Arc::new(p));
    }
    // The waves: a DOC RAM file, or each instrument's ASIF file.
    let stem = name.rsplit_once('.').map_or(name, |(a, _)| a);
    let doc = [".D", ".d", ".W", ".w", ".DOC", ".doc", ".WB", ".wb"]
        .iter()
        .find_map(|e| companion(&format!("{name}{e}")).or_else(|| companion(&format!("{stem}{e}"))))
        .or_else(|| companion("DOC.DATA"));
    if let Some(doc) = doc.filter(|b| b.len() >= 65538 + 92) {
        let n = usize::from(u16::from_le_bytes([doc[0], doc[1]])).min(15);
        for i in 0..n {
            let Some(lists) = doc.get(65538 + i * 92 + 32..65538 + i * 92 + 44) else { break };
            let size = 1usize << ((lists[2] & 7) + 8);
            let at = 2 + usize::from(lists[1]) * 256;
            if let Some(data) = doc.get(at..(at + size).min(doc.len())) {
                let (w, l) = ss_wave(data, lists);
                m.samples[i].wave = Arc::new(w);
                m.samples[i].looped = l;
            }
        }
    } else {
        for (i, n) in names.iter().enumerate() {
            let Some(f) = (!n.is_empty()).then(|| companion(n)).flatten() else { continue };
            if let Some((w, l)) = ss_asif(&f) {
                m.samples[i].wave = Arc::new(w);
                m.samples[i].looped = l;
            }
        }
    }
    m.tidy();
    Ok(m)
}

/// An ASIF instrument file: its INST's wave lists, its WAVE's data.
fn ss_asif(f: &[u8]) -> Option<(Wave, Loop)> {
    if f.len() < 12 || &f[..4] != b"FORM" || &f[8..12] != b"ASIF" {
        return None;
    }
    let (mut lists, mut wave) = (None, None);
    let mut at = 12;
    while at + 8 <= f.len() {
        let n = u32::from_be_bytes(f[at + 4..at + 8].try_into().ok()?) as usize;
        let body = f.get(at + 8..(at + 8 + n).min(f.len()))?;
        match &f[at..at + 4] {
            b"INST" => {
                let skip = 1 + usize::from(*body.first()?) + 2 + 24 + 6;
                if body.get(skip..skip + 2)? != [1, 1] {
                    return None;
                }
                lists = Some(body.get(skip + 2..skip + 14)?.to_vec());
            }
            b"WAVE" => {
                let mut r = Reader::at(body, 1 + usize::from(*body.first()?));
                let (size, count, off) = (usize::from(r.u16le().ok()?), r.u16le().ok()?, usize::from(r.u16le().ok()?));
                if count != 1 {
                    return None;
                }
                let start = if off > 8 { r.pos + off - 8 } else { r.pos + 10 };
                wave = Some(body.get(start..(start + size).min(body.len()))?.to_vec());
            }
            _ => {}
        }
        at += 8 + n;
    }
    Some(ss_wave(&wave?, &lists?))
}

// ---- FM Tracker ----
//
// "FMTracker" 01 01, the tracker's and song's names, eight channels
// each with a name and an OPL2 instrument (S3M's eleven bytes), the
// last row, order and pattern; the orders and a speed for each; which
// pattern each stored one is; then each pattern channel by channel, a
// byte a row (a note, 1 a cut, or with its top bit set rows skipped).
// Ticks come at 18.2 a second; an order's speed takes over on the
// pattern before's last row, as the original routine does it.

pub fn probe_fmt(d: &[u8]) -> bool {
    d.len() >= 218 && &d[..11] == b"FMTracker\x01\x01" && (0..8).all(|c| {
        let s = &d[63 + c * 19 + 8..63 + c * 19 + 19];
        s[8] & 0xFC == 0 && s[9] & 0xFC == 0 && s[10] & 0xF0 == 0
    })
}

pub fn load_fmt(d: &[u8]) -> Result<Module> {
    let mut m = mod_module("fmt", "FM Tracker", &crate::io::text(&d[11..31]), 8);
    m.kind = Kind::S3m;
    m.title = crate::io::text(&d[31..63]);
    m.tempo = 46;
    for c in 0..8 {
        let at = 63 + c * 19;
        let name = crate::io::text(&d[at..at + 8]);
        let patch: [u8; 11] = d[at + 8..at + 19].try_into().unwrap();
        m.channels[c].name = name.clone();
        m.channels[c].pan = 128;
        m.samples.push(Sample { name, c5: 8215, fm: Some(FmPatch::from_s3m(&patch)), ..Sample::empty() });
    }
    let rows = usize::from(d[215]) + 1;
    let norders = usize::from(d[216]) + 1;
    let npat = usize::from(d[217]) + 1;
    let mut r = Reader::at(d, 218);
    let orders: Vec<u8> = r.bytes(norders)?.to_vec();
    let delays = r.bytes(norders)?.to_vec();
    if delays.iter().any(|&x| !(1..=8).contains(&x)) {
        return bad("an order's speed is out of range");
    }
    let map = r.bytes(npat)?.to_vec();
    let mut stored: Vec<Option<Pattern>> = vec![None; 256];
    for &slot in &map {
        let mut p = Pattern::new(rows, 8);
        for ch in 0..8 {
            let mut row = 0;
            while row < rows {
                let Ok(b) = r.u8() else { break };
                if b & 0x80 != 0 {
                    row += usize::from(b & 0x7F);
                } else if let Some(c) = p.cell_mut(row, ch) {
                    if b == 1 {
                        c.note = NOTE_CUT;
                    } else if (2..=97).contains(&b) {
                        c.note = b + 12;
                        c.instr = ch as u8 + 1;
                    }
                }
                row += 1;
            }
        }
        stored[usize::from(slot)] = Some(p);
    }
    // A pattern an order, the next order's speed on its last row.
    m.speed = delays[0];
    for (i, &o) in orders.iter().enumerate() {
        let mut p = stored[usize::from(o)].clone().unwrap_or_else(|| Pattern::new(rows, 8));
        if let Some(c) = p.cell_mut(rows - 1, 0) {
            (c.fx, c.param) = (Fx::Speed, delays[(i + 1) % norders]);
        }
        m.patterns.push(Arc::new(p));
        m.orders.push(i as u16);
    }
    m.tidy();
    Ok(m)
}

// ---- His Master's Noise ----
//
// NoiseTracker's layout, tagged FEST (or M&K!). An instrument named
// "Mupp" is a wavetable sound kept in a pattern's place (the name's
// next three bytes: which pattern, the loop's start and end): 28
// waveforms of 32 bytes, then 64 steps of which waveform plays and 64
// of its volume, a step a tick. Played here by neumetik-synth's synth
// sound sequences. Its mega-arpeggio (7) isn't kept; 8, 9 and E are
// dropped as the player did.

pub fn probe_hmn(d: &[u8]) -> bool {
    if d.len() < 1084 {
        return false;
    }
    let tag = &d[1080..1084];
    let mupp = (0..31).any(|i| &d[20 + i * 30..24 + i * 30] == b"Mupp");
    tag == b"FEST" || (tag == b"M&K!" && (mupp || (0..31).any(|i| d[20 + i * 30 + 24] > 15)))
}

pub fn load_hmn(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let title = r.text(20)?;
    let mut heads = Vec::new();
    for _ in 0..31 {
        let raw = r.bytes(22)?.to_vec();
        let size = usize::from(r.u16be()?) * 2;
        let ft = r.u8()?;
        let vol = r.u8()?;
        let ls = u32::from(r.u16be()?) * 2;
        let ll = u32::from(r.u16be()?) * 2;
        heads.push((raw, size, ft, vol, ls, ll));
    }
    let len = usize::from(r.u8()?);
    let restart = u16::from(r.u8()?);
    let orders = r.bytes(128)?.to_vec();
    r.skip(4)?;
    let npat = usize::from(*orders.iter().max().unwrap_or(&0)) + 1;
    let mut m = mod_module("hmn", "His Master's Noise", "NoiseTracker (His Master's Noise)", 4);
    m.title = title;
    m.restart = restart;
    m.orders = orders[..len.clamp(1, 128)].iter().map(|&o| u16::from(o)).collect();
    for _ in 0..npat {
        let mut p = Pattern::new(64, 4);
        for c in p.cells.iter_mut() {
            *c = mod_cell(r.bytes(4)?);
            if matches!(c.fx, Fx::Tremolo | Fx::Panning | Fx::Offset | Fx::ModEx) {
                (c.fx, c.param) = (Fx::None, 0);
            }
        }
        m.patterns.push(Arc::new(p));
    }
    for (raw, size, ft, vol, ls, ll) in &heads {
        let mupp = &raw[..4] == b"Mupp";
        let mut s = Sample { volume: (*vol).min(64), c5: relative_c5(0, ((ft << 3) as i8).wrapping_neg()), ..Sample::empty() };
        if mupp {
            s.name = format!("Mupp {:02x} {:02x} {:02x}", raw[4], raw[5], raw[6]);
            let at = 1084 + 1024 * usize::from(raw[4]);
            if let Some(b) = d.get(at..at + 28 * 32 + 128) {
                let waveforms: Vec<Vec<i8>> = b[..28 * 32].chunks(32).map(|w| w.iter().map(|&x| x as i8).collect()).collect();
                let (start, end) = (raw[5].min(63), raw[6].min(63));
                let mut waves: Vec<u8> = b[896..896 + usize::from(end) + 1].iter().map(|&w| w.min(27)).collect();
                waves.extend([WAVE_JMP, start]);
                let mut volumes: Vec<u8> = b[960..960 + usize::from(end) + 1].iter().map(|&v| (v & 0x7F).min(64)).collect();
                volumes.extend([VOL_JMP, start]);
                let first = waveforms[usize::from(waves[0].min(27))].iter().map(|&x| i16::from(x) << 8).collect();
                s.wave = Arc::new(Wave::new(8, 1, first));
                s.looped = Loop::forward(0, 32);
                let sound = SynthSound { waveforms, volumes, waves, vol_speed: 1, wave_speed: 1, hybrid: false };
                s.program = Some(Arc::new(Program::new(sound, &s.wave)));
            }
        } else {
            s.name = crate::io::text(raw);
            let pcm = decode(r.bytes_upto(*size), Pcm::S8, *size);
            s.wave = Arc::new(Wave::new(8, 1, pcm));
            if *ll > 2 {
                s.looped = Loop::forward(*ls, ls + ll);
            }
        }
        m.samples.push(s);
    }
    m.tidy();
    Ok(m)
}

// ---- Coconizer ----
//
// The channels (84 or 88), a title ended by CR, instrument, order and
// pattern counts, where the orders and patterns are; each instrument's
// place, length, volume (logarithmic, inverted), loop and name; orders
// to FF; 64-row patterns of four-byte cells (parameter, command,
// instrument, note); VIDC samples.

pub fn probe_coco(d: &[u8]) -> bool {
    if d.len() < 32 || !(d[0] == 0x84 || d[0] == 0x88) || !d[1..21].contains(&0x0D) {
        return false;
    }
    let n = usize::from(d[21]);
    let le = |at: usize| d.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    if n == 0 || n > 100 || !le(24).is_some_and(|y| (64..=0x10_0000).contains(&y)) || !le(28).is_some_and(|y| (64..=0x10_0000).contains(&y)) {
        return false;
    }
    (0..n).all(|i| {
        let at = 32 + i * 32;
        let (Some(ofs), Some(len), Some(vol), Some(lps), Some(lsz)) = (le(at), le(at + 4), le(at + 8), le(at + 12), le(at + 16)) else { return false };
        (64..=0x10_0000).contains(&ofs) && vol <= 0xFF && len <= 0x10_0000 && lps <= 0x10_0000 && lsz <= 0x10_0000 && !(lps > 0 && lps + lsz > len + 1)
    })
}

pub fn load_coco(d: &[u8]) -> Result<Module> {
    let nch = usize::from(d[0] & 0x3F);
    let title = crate::io::text(&d[1..21]).split('\r').next().unwrap_or("").to_string();
    let (nins, norders, npat) = (usize::from(d[21]), usize::from(d[22]), usize::from(d[23]));
    let mut r = Reader::at(d, 24);
    let (seq, pat) = (r.u32le()? as usize, r.u32le()? as usize);
    let mut m = mod_module("coco", "Coconizer", "Coconizer", nch);
    m.title = title;
    let mut ptrs = Vec::new();
    for _ in 0..nins {
        let ptr = r.u32le()? as usize;
        let len = r.u32le()? as usize;
        let vol = r.u32le()?;
        let ls = r.u32le()?;
        let ll = r.u32le()?;
        let name = crate::io::text(r.bytes(11)?).split('\r').next().unwrap_or("").to_string();
        r.skip(1)?;
        ptrs.push((ptr, len));
        let v = 0xFF - vol.min(0xFF) as u8;
        let volume = if v == 0 { 0 } else { (64.0 * 2f64.powf(-(255.0 - f64::from(v)) / 32.0)).round() as u8 };
        m.samples.push(Sample { name, volume, looped: if ls > 0 { Loop::forward(ls, (ls + ll).saturating_sub(1)) } else { Loop::default() }, ..Sample::empty() });
    }
    m.orders = d.get(seq..).unwrap_or(&[]).iter().take_while(|&&o| o != 0xFF).take(norders.max(1)).map(|&o| u16::from(o)).collect();
    let mut r = Reader::at(d, pat);
    for _ in 0..npat {
        let mut p = Pattern::new(64, nch);
        for c in p.cells.iter_mut() {
            let b = r.bytes(4)?;
            let (prm, cmd, ins, note) = (b[0], b[1], b[2], b[3]);
            c.instr = ins;
            if note > 0 {
                c.note = (note + 12).min(NOTE_MAX);
            }
            (c.fx, c.param) = match cmd {
                0 if prm != 0 => (Fx::Arpeggio, prm),
                1 | 5 => (Fx::PortaUp, prm),
                2 | 6 => (Fx::PortaDown, prm),
                3 => (Fx::VolSlide, (prm.min(15) << 4) | 0x0F),
                4 => (Fx::VolSlide, 0xF0 | prm.min(15)),
                7 if (1..8).contains(&prm) => (Fx::Panning, (42 * u16::from(prm) - 40) as u8),
                0x0C => {
                    let v = 0xFF - prm;
                    (Fx::Volume, if v == 0 { 0 } else { (64.0 * 2f64.powf(-(255.0 - f64::from(v)) / 32.0)).round() as u8 })
                }
                0x0D => (Fx::PatternBreak, prm),
                0x0E => (Fx::PositionJump, prm),
                0x0F => (Fx::Speed, prm),
                0x13 => (Fx::VolSlide, prm.min(15) << 4),
                0x14 => (Fx::VolSlide, prm.min(15)),
                _ => (Fx::None, 0),
            };
        }
        m.patterns.push(Arc::new(p));
    }
    for (s, (ptr, len)) in m.samples.iter_mut().zip(ptrs) {
        let at = ptr.min(d.len());
        let b = &d[at..at.saturating_add(len).min(d.len())];
        s.wave = Arc::new(Wave::new(16, 1, b.iter().map(|&v| super::micro::vidc_point(v)).collect()));
    }
    for (i, c) in m.channels.iter_mut().enumerate() {
        c.pan = if ((i + 3) / 2) % 2 == 1 { 255 } else { 0 };
    }
    m.kind = Kind::S3m;
    m.tidy();
    Ok(m)
}

// ---- Magnetic Fields Packer ----
//
// A packed ProTracker song in "mfp.<name>", its samples in
// "smp.<name>": 31 samples' headers, the song's length, 7F, the
// orders (each order its own pattern), then each order's four track
// offsets and the tracks, each cell found through three levels of
// indices into four-byte ProTracker cells.

pub fn probe_mfp(d: &[u8], name: &str) -> bool {
    let n = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    if d.len() < 384 || !n.starts_with("mfp.") || d[249] != 0x7F {
        return false;
    }
    let be = |at: usize| u16::from_be_bytes([d[at], d[at + 1]]);
    (0..31).all(|i| {
        let (len, ft, vol, lps, lsz) = (be(i * 8), d[i * 8 + 2], d[i * 8 + 3], be(i * 8 + 4), be(i * 8 + 6));
        len <= 0x7FFF && ft & 0xF0 == 0 && vol <= 0x40 && lps <= len && u32::from(lps) + u32::from(lsz) <= u32::from(len) + 1 && !(len > 0 && lsz == 0)
    }) && u16::from(d[248]) == be(378)
        && be(378) == be(380)
}

pub fn load_mfp(d: &[u8], name: &str, companion: Companion) -> Result<Module> {
    let mut r = Reader::new(d);
    let mut m = mod_module("mfp", "Magnetic Fields Packer", "Magnetic Fields Packer", 4);
    let mut heads = Vec::new();
    for _ in 0..31 {
        let len = usize::from(r.u16be()?) * 2;
        let ft = r.u8()?;
        let vol = r.u8()?;
        let ls = u32::from(r.u16be()?) * 2;
        let ll = u32::from(r.u16be()?) * 2;
        heads.push(len);
        m.samples.push(Sample { volume: vol.min(64), c5: finetune_c5(ft), looped: if ll > 2 { Loop::forward(ls, ls + ll) } else { Loop::default() }, ..Sample::empty() });
    }
    let len = usize::from(r.u8()?);
    r.skip(1 + 128)?;
    let size1 = usize::from(r.u16be()?);
    r.skip(2)?;
    if size1 > len {
        return bad("its pattern table doesn't add up");
    }
    let mut offs = Vec::with_capacity(size1 * 4);
    for _ in 0..size1 * 4 {
        offs.push(usize::from(r.u16be()?));
    }
    let base = r.pos;
    let mut sorted = offs.clone();
    sorted.sort_unstable();
    sorted.dedup();
    for o in 0..size1 {
        let mut p = Pattern::new(64, 4);
        for ch in 0..4 {
            let off = offs[o * 4 + ch];
            let next = sorted.iter().find(|&&x| x > off).copied();
            let buf = d.get(base + off..next.map_or(base + off + 514, |n| base + n).min(d.len())).unwrap_or(&[]);
            let at = |i: usize| buf.get(i).map(|&b| usize::from(b));
            let mut row = 0;
            'all: for k in 0..4 {
                for x in 0..4 {
                    for y in 0..4 {
                        let Some(i) = at(k).and_then(|a| at(a + x)).and_then(|b| at(b + y)) else { break 'all };
                        let Some(cell) = buf.get(i * 2..i * 2 + 4) else { break 'all };
                        p.cells[row * 4 + ch] = mod_cell(cell);
                        row += 1;
                    }
                }
            }
        }
        m.patterns.push(Arc::new(p));
        m.orders.push(o as u16);
    }
    // The samples, beside it: "smp." for "mfp." (or a ".set" for a name with a dash).
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let rest = &file[4.min(file.len())..];
    let smp = companion(&format!("smp.{rest}")).or_else(|| companion(&format!("SMP.{rest}"))).or_else(|| rest.rsplit_once('-').and_then(|(a, _)| companion(&format!("smp.{a}.set"))));
    if let Some(smp) = smp {
        let mut sr = Reader::new(&smp);
        for (s, len) in m.samples.iter_mut().zip(heads) {
            s.wave = Arc::new(Wave::new(8, 1, decode(sr.bytes_upto(len), Pcm::S8, len)));
        }
    }
    m.tidy();
    Ok(m)
}

// ---- A.M. Composer ----
//
// "AMC V1.2 REPLAY!", the song's length in rows, the size of the song
// before its samples, where each voice's notes and effects are (eight
// lists), the CIA timer's high byte (the tempo); then 16 bytes a sample
// (where it is, its length, where its loop is and the loop's length, in
// words) up to the first list. A voice's notes are six bytes (row,
// sample, volume, period); its effects a row, a command and the
// command's bytes: 1 a period (no new note), 2 a slide (ticks a step,
// the step, for how many ticks, the period it starts from), 3 a volume,
// 4 eight periods a tick each (an arpeggio), 7 the LED filter. A list
// is read an entry a row, and an entry whose row has passed holds the
// list up, as the replayer does; a row is four ticks, a new note stops a
// slide or arpeggio. The replayer's ticks are followed here and the
// periods it plays made notes, porta and arpeggio row by row. Described
// by Wanted Team's EaglePlayer replayer (in UADE), read for its behavior.

pub fn probe_amc(d: &[u8]) -> bool {
    d.len() >= 0x48 && &d[..16] == b"AMC V1.2 REPLAY!" && d[0x18] == 0 && d[0x19] == 0
}

pub fn load_amc(d: &[u8]) -> Result<Module> {
    let be16 = |at: usize| d.get(at..at + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]));
    let be32 = |at: usize| d.get(at..at + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let rows = usize::from(be16(0x10));
    let lists: Vec<usize> = (0..8).map(|i| be32(0x18 + i * 4)).collect();
    let timer = u32::from(d[0x39]) << 8;
    if rows == 0 || timer == 0 || lists.iter().any(|&l| l < 0x48 || l >= d.len()) {
        return bad("its header doesn't add up");
    }
    let mut m = mod_module("amc", "A.M. Composer", "A.M. Composer 1.2", 4);
    m.speed = 4;
    // The PAL CIA's 709379 Hz, a tick each `timer` counts.
    m.tempo = (1_773_447.0 / f64::from(timer)).round().clamp(32.0, 512.0) as u16;
    for i in 0..((lists[0] - 0x48) / 16).min(255) {
        let at = 0x48 + i * 16;
        let (off, len) = (be32(at), usize::from(be16(at + 4)) * 2);
        let (lo, ll) = (be32(at + 6), u32::from(be16(at + 10)) * 2);
        let pcm = decode(&d[off.min(d.len())..off.saturating_add(len).min(d.len())], Pcm::S8, len);
        // A loop outside the sample (most point at four zero bytes in the header) is silence after it.
        let looped = if lo >= off && lo - off < len && ll > 2 { Loop::forward((lo - off) as u32, (lo - off) as u32 + ll).clamped(len as u32) } else { Loop::default() };
        m.samples.push(Sample { volume: 64, looped, wave: Arc::new(Wave::new(8, 1, pcm)), ..Sample::empty() });
    }
    let nsmp = m.samples.len();
    let period = |note: u8| C5_PERIOD * 2f64.powf(f64::from(61 - i32::from(note)) / 12.0);
    let mut pats: Vec<Pattern> = (0..rows.div_ceil(64)).map(|i| Pattern::new((rows - i * 64).min(64), 4)).collect();
    for v in 0..4 {
        let (mut ni, mut fi) = (lists[v], lists[4 + v]);
        // What Paula plays, and what the cells have it play.
        let mut hw = 0u16;
        let mut cur = 0.0f64;
        // The slide: its reload, counter, step, ticks left and period.
        let (mut reload, mut count, mut step, mut left, mut sper) = (0u8, 0u8, 0u16, 0u16, 0u16);
        // The arpeggio: ticks left, where it is, its periods.
        let (mut arp_left, mut arp_at, mut arp) = (0u8, 0usize, [0u16; 8]);
        for row in 0..rows {
            let cell = &mut pats[row / 64].cells[(row % 64) * 4 + v];
            let (mut slid, mut arped, mut struck) = (false, false, false);
            for tick in 0..4 {
                if left != 0 {
                    left -= 1;
                    if left != 0 {
                        count = count.wrapping_sub(1);
                        if count == 0 {
                            count = reload;
                            sper = sper.wrapping_sub(step);
                            hw = sper;
                            slid = true;
                        }
                    }
                }
                if arp_left != 0 {
                    arp_left -= 1;
                    hw = arp[arp_at];
                    arp_at = (arp_at + 1) % 8;
                    arped = true;
                }
                if tick != 0 {
                    continue;
                }
                if ni + 6 <= d.len() && usize::from(be16(ni)) == row {
                    let (smp, vol, per) = (d[ni + 2], d[ni + 3], be16(ni + 4));
                    ni += 6;
                    (left, arp_left, hw) = (0, 0, per);
                    if let Some(n) = period_to_note(per) {
                        (cell.note, cell.vol, cell.volp) = (n, Vol::Volume, vol.min(64));
                        cell.instr = if usize::from(smp) <= nsmp { smp } else { 0 };
                        (cur, struck) = (period(n), true);
                    }
                }
                if fi + 3 <= d.len() && usize::from(be16(fi)) == row {
                    let arg = d.get(fi + 3).copied().unwrap_or(0);
                    match d[fi + 2] {
                        1 => {
                            hw = be16(fi + 4);
                            fi += 6;
                        }
                        2 => {
                            (reload, count, step, left, sper) = (arg, arg, be16(fi + 4), be16(fi + 6), be16(fi + 8));
                            arp_left = 0;
                            fi += 10;
                        }
                        3 => {
                            let vol = be16(fi + 4).min(64) as u8;
                            if cell.vol == Vol::None {
                                (cell.vol, cell.volp) = (Vol::Volume, vol);
                            } else {
                                (cell.fx, cell.param) = (Fx::Volume, vol);
                            }
                            fi += 6;
                        }
                        4 => {
                            left = 0;
                            (arp_left, arp_at) = (arg, 1);
                            for (k, p) in arp.iter_mut().enumerate() {
                                *p = be16(fi + 4 + k * 2);
                            }
                            fi += 20;
                        }
                        7 => {
                            // 0 turns the filter off (the LED's bit set), as E01 does.
                            if cell.fx == Fx::None {
                                (cell.fx, cell.param) = (Fx::ModEx, u8::from(arg == 0));
                            }
                            fi += 4;
                        }
                        _ => fi += 3,
                    }
                }
            }
            if cur <= 0.0 || hw == 0 {
                continue;
            }
            if arped {
                // Its periods as semitones over the note: an arpeggio when there are two at most.
                let mut semis: Vec<u8> = Vec::new();
                for &p in &arp {
                    let s = 12.0 * (cur / f64::from(p.max(1))).log2();
                    if (s - s.round()).abs() > 0.15 || !(0.0..=15.0).contains(&s.round()) {
                        semis.clear();
                        break;
                    }
                    let s = s.round() as u8;
                    if s != 0 && !semis.contains(&s) {
                        semis.push(s);
                    }
                }
                semis.sort_unstable();
                if !semis.is_empty() && semis.len() <= 2 && cell.fx == Fx::None {
                    (cell.fx, cell.param) = (Fx::Arpeggio, semis[0] << 4 | semis[semis.len() - 1] * u8::from(semis.len() == 2));
                }
                if arp_left != 0 {
                    continue;
                }
            }
            if (12.0 * (cur / f64::from(hw)).log2()).abs() < 0.02 {
                continue;
            }
            // A slide that's on, though it didn't step this row, is still slid to.
            let slid = slid || left != 0;
            if struck && !slid {
                // A period set as the note's struck: the note it's nearest.
                if let Some(n) = period_to_note(hw) {
                    (cell.note, cur) = (n, period(n));
                }
            } else if slid && cell.fx == Fx::None {
                // The slide so far, spread over the row's three ticks after the first.
                let delta = cur - f64::from(hw);
                let p = (delta.abs() / 3.0).round().min(255.0);
                if p > 0.0 {
                    (cell.fx, cell.param) = (if delta > 0.0 { Fx::PortaUp } else { Fx::PortaDown }, p as u8);
                    cur -= delta.signum() * p * 3.0;
                }
            } else if !slid && cell.fx == Fx::None {
                // A period set: slid to at once, without a new note.
                if let Some(n) = period_to_note(hw) {
                    (cell.note, cell.fx, cell.param, cur) = (n, Fx::TonePorta, 0xFF, period(n));
                }
            }
        }
    }
    m.patterns = pats.into_iter().map(Arc::new).collect();
    m.orders = (0..m.patterns.len() as u16).collect();
    m.tidy();
    Ok(m)
}

// ---- Editeur Musical Sequentiel ----
//
// Thomas Pimmel's EMS (Modland's "Electronic Music System"; not EMS v6).
// A version word (01 22 to 01 25), the song's length, the last pattern,
// 128 orders; 15 samples of 12 bytes (a pointer, then the length, loop
// start, loop length and volume in words); 64-row patterns of four
// ProTracker-like cells (a period, the sample over the command, its
// argument); then the samples, signed 8-bit. The replayer reads them two
// bytes early and clears each one's first word. A row is the speed's
// ticks and one more. Commands: 7 volume down, B and C pitch up and down
// (in periods a tick), D a period's swing either way every tick (kept as
// a fast vibrato), 8 break to a row, 9 speed, A the LED filter, E volume,
// F the whole sample looped (not kept). A note with no sample changes
// the pitch without striking it. Described by Wanted Team's EaglePlayer
// replayer (in UADE), read for its behavior.

const EMS_PERIODS: [u16; 48] = [
    0x6B0, 0x650, 0x5F4, 0x5A0, 0x54C, 0x500, 0x4B8, 0x474, 0x434, 0x3F8, 0x3C0, 0x38A, 0x358, 0x328, 0x2FA, 0x2D0, 0x2A6, 0x280, 0x25C, 0x23A, 0x21A, 0x1FC, 0x1E0, 0x1C5, 0x1AC, 0x194, 0x17D, 0x168, 0x153, 0x140, 0x12E, 0x11D,
    0x10D, 0xFE, 0xF0, 0xE2, 0xD6, 0xCA, 0xBE, 0xB4, 0xAA, 0xA0, 0x97, 0x8F, 0x87, 0x7F, 0x78, 0x71,
];

pub fn probe_ems(d: &[u8]) -> bool {
    if d.len() < 1338 || d[0] != 1 || d[1] > 0x25 {
        return false;
    }
    let (len, last) = (u16::from_be_bytes([d[2], d[3]]), u16::from_be_bytes([d[4], d[5]]));
    if len == 0 || len > 128 || last > 63 || d[6..134].iter().any(|&o| u16::from(o) > last) || d.len() < 314 + (usize::from(last) + 1) * 1024 {
        return false;
    }
    // The first pattern's periods: each one of the replayer's, and one at least.
    let mut any = false;
    for cell in d[314..314 + 1024].chunks_exact(4) {
        let p = u16::from_be_bytes([cell[0], cell[1]]);
        if p != 0 {
            if !EMS_PERIODS.contains(&p) {
                return false;
            }
            any = true;
        }
    }
    any
}

pub fn load_ems(d: &[u8]) -> Result<Module> {
    let be16 = |at: usize| u16::from_be_bytes([d[at], d[at + 1]]);
    let (len, npat) = (usize::from(be16(2)), usize::from(be16(4)) + 1);
    let mut m = mod_module("ems", "Editeur Musical Sequentiel", "Editeur Musical Sequentiel (EMS) 1.x", 4);
    m.speed = 7;
    m.orders = d[6..6 + len].iter().map(|&o| u16::from(o)).collect();
    let mut at = 314 + npat * 1024 - 2;
    let mut cleared = false;
    for i in 0..15 {
        let h = 134 + i * 12;
        let (words, rep, repl) = (usize::from(be16(h + 4)), u32::from(be16(h + 6)) * 2, u32::from(be16(h + 8)) * 2);
        // Played at the PAL Amiga's rate, as the replayer was.
        let mut s = Sample { volume: be16(h + 10).min(64) as u8, c5: 8287, ..Sample::empty() };
        if words != 0 {
            let n = words * 2;
            let mut pcm = decode(&d[at.min(d.len())..(at + n).min(d.len())], Pcm::S8, n);
            if let Some(w) = pcm.get_mut(..2) {
                w.fill(0);
            }
            cleared = true;
            if repl > 2 {
                s.looped = Loop::forward(rep, rep + repl).clamped(n as u32);
            }
            s.wave = Arc::new(Wave::new(8, 1, pcm));
            at += n;
        }
        m.samples.push(s);
    }
    // The last stored pattern's last cell lost its command to the first sample's cleared word.
    let end = 314 + npat * 1024;
    let mut pats = d[314..end].to_vec();
    if cleared {
        pats[npat * 1024 - 2..].fill(0);
    }
    for raw in pats.chunks_exact(1024) {
        let mut p = Pattern::new(64, 4);
        for (c, b) in p.cells.iter_mut().zip(raw.chunks_exact(4)) {
            let (per, smp, cmd, arg) = (u16::from_be_bytes([b[0], b[1]]), b[2] >> 4, b[2] & 15, b[3]);
            if smp != 0 {
                c.instr = smp;
            }
            if let Some(n) = period_to_note(per) {
                c.note = n;
            }
            (c.fx, c.param) = match cmd {
                7 => (Fx::VolSlide, arg.min(15)),
                8 => (Fx::PatternBreak, if arg < 64 { arg } else { 0 }),
                9 if arg != 0 => (Fx::Speed, arg.saturating_add(1)),
                // Set turns the filter off, as E01 does.
                0xA => (Fx::ModEx, u8::from(arg != 0)),
                0xB if arg != 0 => (Fx::PortaUp, arg),
                0xC if arg != 0 => (Fx::PortaDown, arg),
                0xD if arg != 0 => (Fx::Vibrato, 0xF0 | (arg / 2).clamp(1, 15)),
                _ => (Fx::None, 0),
            };
            if cmd == 0xE {
                (c.vol, c.volp) = (Vol::Volume, arg.min(64));
            }
            if per != 0 && smp == 0 && c.fx == Fx::None {
                // A new pitch for the sample playing, not struck again.
                (c.fx, c.param) = (Fx::TonePorta, 0xFF);
            }
        }
        m.patterns.push(Arc::new(p));
    }
    m.tidy();
    Ok(m)
}

// ---- JamCracker ----
//
// "BeEp", the sample count, and 40 bytes a sample (a 31-byte name, its
// flags: 1 looped whole, 2 an AM wave; its length, a place for its
// address); the pattern count and 6 bytes each (its rows, an address);
// the song's length and its patterns (words); the patterns, four cells a
// row of 8 bytes (the period's place in the replayer's table, the
// sample, then the speed in the low nibble with bit 6 for a slide to the
// note and bit 7 for a volume set, an arpeggio, a vibrato, the phase,
// the volume and a porta); then the samples, signed 8-bit. An AM wave is
// 384 bytes, played by neumetik-synth (`jam`): its 64-point cycle twice,
// then each channel's copy, mixed with itself as the channel's phase
// runs (`Fx::Phase`). The replayer's effects last from row to row until
// FF stops them and work on every tick, the first too; it runs a tick at
// a time here and what it plays is written into the rows: the volume
// each row starts at and its slide, the pitch's slides spread over the
// row's ticks after the first (a jump is a slide to the nearest note at
// once), the arpeggio and vibrato on every row they last. Each order
// gets its own pattern (shared where they come out the same). Described
// by the replayer (Martin Gemmel's, adapted for EaglePlayer, in UADE) and
// NostalgicPlayer's MIT-licensed reading of it, read for their behavior.

const JAM_PERIODS: [u16; 36] = [1019, 962, 908, 857, 809, 763, 720, 680, 642, 606, 572, 540, 509, 481, 454, 428, 404, 381, 360, 340, 321, 303, 286, 270, 254, 240, 227, 214, 202, 190, 180, 170, 160, 151, 143, 135];

/// The replayer's period at a place in its table (past the end, its highest).
fn jam_period(i: usize) -> i16 {
    JAM_PERIODS.get(i).copied().unwrap_or(135) as i16
}

/// The header's sizes: samples, then the patterns' rows, then the
/// orders, and where the pattern data starts; None when it doesn't fit.
fn jam_layout(d: &[u8]) -> Option<(usize, Vec<usize>, Vec<u16>, usize)> {
    let be16 = |at: usize| d.get(at..at + 2).map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])));
    let nsmp = be16(4)?;
    let mut at = 6 + nsmp * 40;
    let npat = be16(at)?;
    let rows: Vec<usize> = (0..npat).map(|i| be16(at + 2 + i * 6)).collect::<Option<_>>()?;
    at += 2 + npat * 6;
    let len = be16(at)?;
    let orders: Vec<u16> = (0..len).map(|i| be16(at + 2 + i * 2).map(|o| o as u16)).collect::<Option<_>>()?;
    at += 2 + len * 2;
    if nsmp > 255 || npat == 0 || len == 0 || rows.iter().any(|&r| r == 0 || r > 256) || orders.iter().any(|&o| usize::from(o) >= npat) {
        return None;
    }
    let cells: usize = rows.iter().sum::<usize>() * 32;
    (at + cells <= d.len()).then_some((nsmp, rows, orders, at))
}

pub fn probe_jam(d: &[u8]) -> bool {
    d.len() >= 6 && &d[..4] == b"BeEp" && jam_layout(d).is_some()
}

/// A channel as the replayer keeps it.
#[derive(Clone, Copy, Default)]
struct JamVoice {
    /// The note's place in the table, and the three periods the arpeggio turns through.
    at: usize,
    pers: [i16; 3],
    por: i16,
    dpor: i16,
    porlevel: i16,
    vib: i16,
    dvib: i16,
    vibcnt: u8,
    vibmax: u8,
    vol: i16,
    dvol: i16,
    vollevel: i16,
    /// The arpeggio and vibrato commands that are on.
    arp: u8,
    vibrato: u8,
}

impl JamVoice {
    /// The period without vibrato or arpeggio, its porta held at the level it slides to.
    fn center(&self) -> i16 {
        let mut p = jam_period(self.at) + self.por;
        if (self.por < 0 && p < self.porlevel) || (self.por > 0 && p > self.porlevel) {
            p = self.porlevel;
        }
        p.clamp(135, 1019)
    }

    fn rotate(&mut self) {
        self.pers.rotate_left(1);
    }

    /// A tick: the volume it plays at, then its slides move on.
    fn tick(&mut self) -> i16 {
        while self.pers[0] == 0 && self.pers != [0; 3] {
            self.rotate();
        }
        self.rotate();
        self.por = (self.por + self.dpor).clamp(-1019, 1019);
        if self.vibcnt != 0 {
            self.vib += self.dvib;
            self.vibcnt -= 1;
            if self.vibcnt == 0 {
                self.dvib = -self.dvib;
                self.vibcnt = self.vibmax;
            }
        }
        let vol = self.vol;
        self.vol = (self.vol + self.dvol).clamp(0, 64);
        vol
    }
}

pub fn load_jam(d: &[u8]) -> Result<Module> {
    let Some((nsmp, rows, orders, mut at)) = jam_layout(d) else { return bad("its header doesn't add up") };
    let mut m = mod_module("jam", "JamCracker", "JamCracker", 4);
    let mut r = Reader::at(d, 6);
    let mut heads = Vec::with_capacity(nsmp);
    for _ in 0..nsmp {
        let name = r.text(31)?;
        let flags = r.u8()?;
        let len = r.u32be()? as usize;
        r.skip(4)?;
        heads.push((name, flags, len));
    }
    let starts: Vec<usize> = rows.iter().scan(at, |a, &n| Some(std::mem::replace(a, *a + n * 32))).collect();
    at += rows.iter().sum::<usize>() * 32;
    for (name, flags, len) in &heads {
        let pcm = decode(&d[at.min(d.len())..at.saturating_add(*len).min(d.len())], Pcm::S8, *len);
        at = at.saturating_add(*len);
        // Played at the PAL Amiga's rate, as the replayer was.
        let mut s = Sample { name: name.clone(), c5: 8287, ..Sample::empty() };
        if flags & 1 != 0 {
            s.looped = Loop::forward(0, pcm.len() as u32);
        }
        s.wave = Arc::new(Wave::new(8, 1, pcm));
        if flags & 2 != 0 {
            s.program = Some(Arc::new(Program::jam(&s.wave)));
        }
        m.samples.push(s);
    }
    // Which samples sound: one past the count, or without data, is silence.
    let sounds: Vec<bool> = m.samples.iter().map(|s| !s.wave.is_empty()).collect();
    let mut v = [JamVoice { pers: [1019, 0, 0], vollevel: 64, ..Default::default() }; 4];
    // What the player will have: each channel's period (0 silent; the
    // replayer's, which its notes are within a few cents of) and volume, and the speed.
    let (mut cur, mut mv, mut mvib) = ([0.0f64; 4], [64i16; 4], [0u8; 4]);
    let (mut wait, mut speed) = (6u8, 6u8);
    let mut pats: Vec<Pattern> = Vec::new();
    // Where the speed changes: the order's pattern, the row, the speed.
    let mut speeds = Vec::new();
    for &o in &orders {
        let (n, base) = (rows[usize::from(o)], starts[usize::from(o)]);
        let mut p = Pattern::new(n, 4);
        for row in 0..n {
            let raw = &d[base + row * 32..base + row * 32 + 32];
            let mut struck = [None::<Option<u8>>; 4];
            for (ch, b) in raw.chunks_exact(8).enumerate() {
                let (per, ins, spd, arp, vib, phase, vol, por) = (b[0], b[1] as i8, b[2], b[3], b[4], b[5], b[6], b[7]);
                let c = &mut v[ch];
                if per != 0 {
                    let at = usize::from(per) - 1;
                    if spd & 64 != 0 {
                        c.porlevel = jam_period(at);
                    } else {
                        c.at = at;
                        c.pers = [jam_period(at); 3];
                        c.por = 0;
                        c.arp = 0;
                        let ok = usize::try_from(ins).ok().filter(|&i| sounds.get(i) == Some(&true));
                        struck[ch] = Some(ok.map(|i| i as u8));
                        if ok.is_some() {
                            c.vol = c.vollevel;
                        }
                    }
                }
                if spd & 15 != 0 {
                    wait = spd & 15;
                }
                match arp {
                    0 => {}
                    0xFF => (c.pers, c.arp) = ([jam_period(c.at); 3], 0),
                    _ => {
                        let (hi, lo) = (c.at + usize::from(arp >> 4), c.at + usize::from(arp & 15));
                        c.pers = [jam_period(c.at), jam_period(hi), jam_period(lo)];
                        // In semitones, as far as the table's top (where it stays).
                        let top = |i: usize| (i.min(JAM_PERIODS.len() - 1).saturating_sub(c.at)).min(15) as u8;
                        c.arp = top(hi) << 4 | top(lo);
                    }
                }
                match vib {
                    0 => {}
                    0xFF => (c.vib, c.vibcnt, c.vibrato) = (0, 0, 0),
                    _ => (c.vib, c.dvib, c.vibmax, c.vibcnt, c.vibrato) = (0, i16::from(vib & 15), vib >> 4, vib >> 5, vib),
                }
                if vol == 0xFF {
                    c.dvol = 0;
                } else if spd & 128 != 0 {
                    (c.vol, c.vollevel, c.dvol) = (i16::from(vol), i16::from(vol), 0);
                } else if vol != 0 {
                    c.dvol = if vol & 128 != 0 { -i16::from(vol & 127) } else { i16::from(vol) };
                }
                match por {
                    0 => {}
                    0xFF => (c.por, c.dpor) = (0, 0),
                    _ if spd & 64 != 0 => {
                        c.por = 0;
                        let step = i16::from(por as i8);
                        c.dpor = if c.porlevel <= c.pers[0] { -step } else { step };
                    }
                    _ => {
                        c.por = 0;
                        (c.dpor, c.porlevel) = if por & 128 != 0 { (i16::from(por & 127), 1019) } else { (-i16::from(por), 135) };
                    }
                }
                if phase != 0 {
                    let cell = &mut p.cells[row * 4 + ch];
                    (cell.fx, cell.param) = (Fx::Phase, phase);
                }
            }
            let ticks = usize::from(wait);
            for (ch, c) in v.iter_mut().enumerate() {
                let cell = &mut p.cells[row * 4 + ch];
                match struck[ch] {
                    Some(Some(s)) => {
                        let n = period_to_note(jam_period(c.at) as u16).unwrap_or(NOTE_MIDDLE_C);
                        (cell.note, cell.instr) = (n, s + 1);
                        (cur[ch], mv[ch]) = (f64::from(jam_period(c.at)), 64);
                    }
                    Some(None) => {
                        cell.note = NOTE_CUT;
                        cur[ch] = 0.0;
                    }
                    None => {}
                }
                let start = c.center();
                let vols: Vec<i16> = (0..ticks).map(|_| c.tick()).collect();
                if cur[ch] <= 0.0 {
                    continue;
                }
                // The pitch first: a jump made at once, a slide spread over
                // the row, the arpeggio (unless the volume, set as the row
                // starts, slides by 16 or more in it: a pluck).
                let (start, end) = (f64::from(start), f64::from(c.center()));
                let slide = vols.windows(2).map(|w| w[1] - w[0]).find(|&s| s != 0).unwrap_or(0);
                let pluck = vols[0] != mv[ch] && slide.abs() * (ticks as i16 - 1) >= 16;
                if cell.fx == Fx::None {
                    if (12.0 * (cur[ch] / start).log2()).abs() > 0.1 {
                        if let Some(n) = period_to_note(start as u16) {
                            (cell.note, cell.fx, cell.param, cur[ch]) = (n, Fx::TonePorta, 0xFF, start);
                        }
                    } else if (end - cur[ch]).abs() >= 0.5 && ticks > 1 {
                        let step = ((end - cur[ch]).abs() / (ticks - 1) as f64).round().clamp(1.0, 255.0);
                        let up = end < cur[ch];
                        (cell.fx, cell.param) = (if up { Fx::PortaUp } else { Fx::PortaDown }, step as u8);
                        cur[ch] += if up { -step } else { step } * (ticks - 1) as f64;
                    } else if c.arp != 0 && !pluck {
                        (cell.fx, cell.param) = (Fx::Arpeggio, c.arp);
                    }
                }
                // The volume the row starts at, and its slide.
                if vols[0] != mv[ch] {
                    (cell.vol, cell.volp, mv[ch]) = (Vol::Volume, vols[0] as u8, vols[0]);
                }
                if slide != 0 {
                    let (w, up) = (slide, slide > 0);
                    let x = if cell.vol == Vol::None {
                        let x = w.unsigned_abs().min(64) as u8;
                        (cell.vol, cell.volp) = (if up { Vol::VolSlideUp } else { Vol::VolSlideDown }, x);
                        x
                    } else if cell.fx == Fx::None {
                        let x = w.unsigned_abs().min(15) as u8;
                        (cell.fx, cell.param) = (Fx::VolSlide, if up { x << 4 } else { x });
                        x
                    } else {
                        0
                    };
                    mv[ch] = (mv[ch] + if up { 1 } else { -1 } * i16::from(x) * (ticks as i16 - 1)).clamp(0, 64);
                }
                // The vibrato: a triangle of 2 × max ticks swinging by step
                // × max / 2 periods, as MOD's sine; beside a volume slide
                // when it's the one the channel has (6xy).
                if c.vibcnt != 0 && c.vibrato & 15 != 0 {
                    let max = c.vibrato >> 4;
                    let depth = ((u16::from(c.vibrato & 15) * u16::from(max) + 2) / 4).clamp(1, 15) as u8;
                    let want = (32 / max).clamp(1, 15) << 4 | depth;
                    if cell.fx == Fx::None {
                        (cell.fx, cell.param, mvib[ch]) = (Fx::Vibrato, want, want);
                    } else if cell.fx == Fx::VolSlide && mvib[ch] == want {
                        cell.fx = Fx::VibratoVol;
                    }
                }
            }
            if wait != speed {
                speed = wait;
                speeds.push((pats.len(), row, wait));
            }
        }
        pats.push(p);
    }
    // The speed goes in a channel with no effect; where one row has
    // none, in a fifth channel of its own throughout.
    let free = speeds.iter().all(|&(i, row, _)| (0..4).any(|ch| pats[i].cell(row, ch).fx == Fx::None));
    if !free {
        for p in &mut pats {
            *p = p.with_channels(5);
        }
        m.channels.push(Channel::default());
    }
    for (i, row, wait) in speeds {
        let p = &mut pats[i];
        let ch = if free { (0..4).find(|&ch| p.cell(row, ch).fx == Fx::None).unwrap_or(0) } else { 4 };
        let cell = &mut p.cells[row * p.channels + ch];
        (cell.fx, cell.param) = (Fx::Speed, wait);
    }
    // Orders that came out the same share a pattern.
    for p in pats {
        let i = m.patterns.iter().position(|q| **q == p).unwrap_or_else(|| {
            m.patterns.push(Arc::new(p));
            m.patterns.len() - 1
        });
        m.orders.push(i as u16);
    }
    m.tidy();
    Ok(m)
}

// ---- AMOS Music Bank ----
//
// AMOS's music bank (.abk): "AmBk", the bank's type (3) and length, then
// "Music   " and the offsets of its instruments, songs and patterns. A
// song is four play lists, one a channel, of patterns, each pattern four
// tracks: words, either a note (an Amiga period; in the older banks, a
// word whose low byte is the rows to wait, then the period) or a command
// (bit 15) with a 7-bit argument: 80 ends the track, 90 waits so many
// rows, 89 sets the instrument; 83 volume (up to 63), 88 tempo, 91 jump,
// 86 and 87 the filter, 85 a repeat; 8A arpeggio, 8B tone porta, 8C
// vibrato, 8D volume slide and 8E/8F slides up and down last from row to
// row until 84 stops them. A tempo (1 to 100) is added up every vertical
// blank and a row goes by at each 100, so rows come at half the tempo a
// second: here the nearest speed, and the beats a minute that make the
// rows' time right; until one is set, ProTracker's speed 6, which the
// banks were mostly converted from (as UADE plays them). Instruments are
// offsets into the bank, a length in words, a loop's start and length, a
// volume. Described by libxmp's abk_load.c and Stuart Caie's notes on the
// format (MIT, with libxmp), and UADE's AMOS player (which turns the bank
// into a ProTracker module), read for their behavior.

/// An AMOS tempo as a speed and beats a minute: rows at half of it a second.
fn abk_tempo(x: u8) -> (u8, u8) {
    let speed = (100.0 / f64::from(x.max(1))).round().clamp(1.0, 255.0);
    (speed as u8, (1.25 * speed * f64::from(x.max(1))).round().clamp(32.0, 255.0) as u8)
}

pub fn probe_abk(d: &[u8]) -> bool {
    d.len() >= 0x20 && &d[..4] == b"AmBk" && d[4..6] == [0, 3] && &d[12..20] == b"Music   "
}

/// One track of an AMOS pattern as 64 cells, and the rows its tempos
/// change on (each a speed in its cell, and beats a minute to put by it).
fn abk_track(d: &[u8], at: usize) -> (Vec<Cell>, Vec<(usize, u8)>) {
    let mut cells = vec![Cell::default(); 64];
    let mut tempos = Vec::new();
    let mut r = Reader::at(d, at);
    let (mut row, mut instr) = (0usize, 0u8);
    // The effect lasting from row to row, and this row's own.
    let (mut lasting, mut now): ((Fx, u8), Option<(Fx, u8)>) = ((Fx::None, 0), None);
    let put = |cells: &mut Vec<Cell>, row: usize, fx: (Fx, u8)| {
        if let Some(c) = cells.get_mut(row) {
            if c.fx == Fx::None {
                (c.fx, c.param) = fx;
            }
        }
    };
    while row < 64 {
        let Ok(w) = r.u16be() else { break };
        if w & 0x8000 == 0 {
            let (wait, period) = if w & 0x4000 != 0 {
                // The older banks: the rows to wait, then the period.
                let Ok(p) = r.u16be() else { break };
                if p == 0 && w & 0xFF == 0 {
                    break;
                }
                (usize::from(w & 0x7F), p & 0x0FFF)
            } else {
                (0, w & 0x0FFF)
            };
            if let Some(n) = period_to_note(period) {
                cells[row].note = n;
                cells[row].instr = instr;
            }
            if wait > 0 {
                put(&mut cells, row, now.take().unwrap_or(lasting));
                for i in 1..wait {
                    put(&mut cells, row + i, lasting);
                }
                row += wait;
            }
            continue;
        }
        let (cmd, x) = ((w >> 8) & 0x7F, (w & 0x7F) as u8);
        match cmd {
            0x00 => {
                // The end of the track (a command given just before it still
                // plays): the pattern breaks after the last row.
                if let Some(fx) = now {
                    put(&mut cells, row, fx);
                }
                if row > 0 && row < 64 {
                    (cells[row - 1].fx, cells[row - 1].param) = (Fx::PatternBreak, 0);
                }
                return (cells, tempos);
            }
            0x10 => {
                let wait = usize::from(x).max(1);
                put(&mut cells, row, now.take().unwrap_or(lasting));
                for i in 1..wait {
                    put(&mut cells, row + i, lasting);
                }
                row += wait;
            }
            0x09 => instr = x + 1,
            0x03 => now = Some((Fx::Volume, x.min(63))),
            0x04 => lasting = (Fx::None, 0),
            0x05 => now = Some((Fx::ModEx, 0x60 | (x & 15))),
            // Off, then on: ProTracker's E01 and E00.
            0x06 => now = Some((Fx::ModEx, 1)),
            0x07 => now = Some((Fx::ModEx, 0)),
            0x08 if x > 0 => {
                let (speed, bpm) = abk_tempo(x);
                now = Some((Fx::Speed, speed));
                tempos.retain(|&(r, _)| r != row);
                tempos.push((row, bpm));
            }
            0x11 => {
                now = Some((Fx::PositionJump, x));
                put(&mut cells, row, now.take().unwrap_or(lasting));
                return (cells, tempos);
            }
            0x0A => lasting = (Fx::Arpeggio, x),
            0x0B => lasting = (Fx::TonePorta, x),
            0x0C => lasting = (Fx::Vibrato, x),
            0x0D => lasting = (Fx::VolSlide, x),
            0x0E => lasting = (Fx::PortaUp, x),
            0x0F => lasting = (Fx::PortaDown, x),
            _ => {}
        }
        if matches!(cmd, 0x0A..=0x0F) {
            now = Some(lasting);
        }
    }
    (cells, tempos)
}

pub fn load_abk(d: &[u8]) -> Result<Module> {
    let be32 = |at: usize| -> Result<usize> { Reader::at(d, at).u32be().map(|v| v as usize) };
    let (ins, songs, pats) = (0x14 + be32(0x14)?, 0x14 + be32(0x18)?, 0x14 + be32(0x1C)?);
    let mut m = mod_module("abk", "AMOS Music Bank", "AMOS", 4);

    // The first song: four play lists, its name.
    let mut s = Reader::at(d, songs);
    if s.u16be()? == 0 {
        return bad("it has no songs");
    }
    let song = songs + s.u32be()? as usize;
    let mut s = Reader::at(d, song);
    let mut lists = Vec::with_capacity(4);
    for _ in 0..4 {
        let mut l = Reader::at(d, song + usize::from(s.u16be()?));
        let mut list = Vec::new();
        while let Ok(p) = l.u16be() {
            if p >= 0xFFFE || list.len() >= 256 {
                break;
            }
            list.push(p);
        }
        lists.push(list);
    }
    s.skip(4)?;
    m.title = s.text(16)?;

    // Instruments: their sample at an offset from the instruments' start.
    let mut r = Reader::at(d, ins);
    let n = usize::from(r.u16be()?);
    if n > 255 {
        return bad("it says it has more than 255 instruments");
    }
    for _ in 0..n {
        let (at, rep) = (r.u32be()? as usize, r.u32be()? as usize);
        let (words, repl, volume, words2) = (r.u16be()?, r.u16be()?, r.u16be()?, r.u16be()?);
        let name = r.text(16)?;
        // A second length, when there is one, is the sample's, and the
        // first is where its loop starts (in words, twice over).
        let (len, loop_start) = if words2 != 0 { (usize::from(words2) * 2, u32::from(words) * 4) } else { (usize::from(words) * 2, (rep.saturating_sub(at) * 2) as u32) };
        let mut smp = Sample { name, volume: volume.min(64) as u8, c5: 8287, ..Sample::empty() };
        if len > 2 {
            let from = (ins + at).min(d.len());
            smp.wave = Arc::new(Wave::new(8, 1, decode(&d[from..(from + len).min(d.len())], Pcm::S8, len)));
            if repl > 2 {
                smp.looped = Loop::forward(loop_start, loop_start + u32::from(repl) * 2).clamped(len as u32);
            }
        }
        m.samples.push(smp);
    }

    // Patterns: four tracks each, by their offsets from the patterns' start.
    let mut r = Reader::at(d, pats);
    let npat = usize::from(r.u16be()?);
    if npat > 256 {
        return bad("it says it has more than 256 patterns");
    }
    let mut tracks = Vec::with_capacity(npat);
    for _ in 0..npat {
        let mut four = Vec::with_capacity(4);
        for _ in 0..4 {
            four.push(abk_track(d, pats + usize::from(r.u16be()?)));
        }
        tracks.push(four);
    }
    // Each order plays the channels' own patterns from their lists: the
    // same pattern on every channel, almost always.
    for i in 0..lists[0].len() {
        let pick: Vec<usize> = (0..4).map(|c| usize::from(*lists[c].get(i).unwrap_or(&lists[0][i]))).collect();
        if pick.iter().any(|&p| p >= npat) {
            break;
        }
        let mut p = Pattern::new(64, 4);
        for (c, &t) in pick.iter().enumerate() {
            for row in 0..64 {
                p.cells[row * 4 + c] = tracks[t][c].0[row];
            }
        }
        // A tempo's beats a minute go in a channel with no effect on its row.
        for (c, &t) in pick.iter().enumerate() {
            for &(row, bpm) in &tracks[t][c].1 {
                if let Some(free) = (0..4).find(|&ch| ch != c && p.cells[row * 4 + ch].fx == Fx::None) {
                    (p.cells[row * 4 + free].fx, p.cells[row * 4 + free].param) = (Fx::Tempo, bpm);
                }
            }
        }
        let at = m.patterns.iter().position(|q| **q == p).unwrap_or_else(|| {
            m.patterns.push(Arc::new(p));
            m.patterns.len() - 1
        });
        m.orders.push(at as u16);
    }
    if m.patterns.is_empty() {
        return bad("its song plays no patterns");
    }
    m.tidy();
    Ok(m)
}
