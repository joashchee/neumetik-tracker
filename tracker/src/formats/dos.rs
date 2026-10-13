//! The DOS trackers of the early nineties: MultiTracker (.mtm), Composer
//! 669 and UNIS 669 (.669), Scream Tracker 2 (.stm) and its Music
//! Interface Kit (.stx), PolyTracker (.ptm), General DigiMusic (.gdm) and
//! the Digital Sound Interface Kit (.dsm).

use std::sync::Arc;

use super::common::{finetune_c5, mod_effect, s3m_effect};
use crate::io::{bad, decode, message, Pcm, Reader, Result};
use crate::model::*;

fn pans(n: usize, f: impl Fn(usize) -> u8) -> Vec<Channel> {
    (0..n).map(|i| Channel { pan: f(i), ..Default::default() }).collect()
}

// ---- MultiTracker ----

pub fn probe_mtm(d: &[u8]) -> bool {
    d.len() >= 66 && &d[..3] == b"MTM" && d[3] <= 0x10 && d[33] <= 32
}

pub fn load_mtm(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    r.skip(4)?;
    let title = r.text(20)?;
    let tracks = usize::from(r.u16le()?);
    let last_pattern = usize::from(r.u8()?);
    let last_order = usize::from(r.u8()?);
    let comment_len = usize::from(r.u16le()?);
    let nsmp = usize::from(r.u8()?);
    let _attr = r.u8()?;
    let rows = usize::from(r.u8()?).clamp(1, 64);
    let nch = usize::from(r.u8()?).clamp(1, 32);
    let pan = r.bytes(32)?.to_vec();
    let mut heads = Vec::new();
    for _ in 0..nsmp {
        let name = r.text(22)?;
        let len = r.u32le()? as usize;
        let ls = r.u32le()?;
        let le = r.u32le()?;
        let ft = r.u8()?;
        let vol = r.u8()?;
        let attr = r.u8()?;
        heads.push((name, len, ls, le, ft, vol, attr));
    }
    let orders: Vec<u16> = r.bytes(128)?[..=last_order.min(127)].iter().map(|&o| u16::from(o)).collect();
    let track_data = r.bytes(tracks * 192)?;
    let mut patterns = Vec::new();
    for _ in 0..=last_pattern {
        let mut p = Pattern::new(rows, nch);
        for c in 0..32 {
            let t = usize::from(r.u16le()?);
            if t == 0 || t > tracks || c >= nch {
                continue;
            }
            let td = &track_data[(t - 1) * 192..t * 192];
            for row in 0..rows {
                let b = &td[row * 3..row * 3 + 3];
                let note = b[0] >> 2;
                let (fx, param) = mod_effect(b[1] & 0x0F, b[2], false);
                p.cells[row * nch + c] = Cell { note: if note > 0 { (note + 25).min(NOTE_MAX) } else { 0 }, instr: (b[0] & 3) << 4 | b[1] >> 4, fx, param, ..Default::default() };
            }
        }
        patterns.push(Arc::new(p));
    }
    let comment = r.bytes_upto(comment_len);
    // The comment is lines of 40 characters.
    let msg = comment.chunks(40).map(crate::io::text).collect::<Vec<_>>().join("\n");
    let mut samples = Vec::new();
    for (name, len, ls, le, ft, vol, attr) in heads {
        let b16 = attr & 1 != 0;
        let bytes = r.bytes_upto(len);
        let pcm = decode(bytes, if b16 { Pcm::S16le } else { Pcm::U8 }, len / if b16 { 2 } else { 1 });
        let div = if b16 { 2 } else { 1 };
        samples.push(Sample {
            name,
            wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
            c5: finetune_c5(ft),
            volume: vol.min(64),
            looped: if le > ls + 2 { Loop::forward(ls / div, le / div) } else { Loop::default() },
            ..Sample::empty()
        });
    }
    let mut m = Module {
        title,
        message: msg.trim_end().to_string(),
        format: "mtm".into(),
        format_name: "MultiTracker".into(),
        tracker: "MultiTracker".into(),
        kind: Kind::Mod,
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: pans(nch, |i| (pan[i] & 15) * 17),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Composer 669 ----

pub fn probe_669(d: &[u8]) -> bool {
    d.len() >= 0x1F1 && (&d[..2] == b"if" || &d[..2] == b"JN") && d[0x6E] <= 64 && d[0x6F] <= 128 && d[0x70] < 128
}

pub fn load_669(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let unis = &d[..2] == b"JN";
    r.skip(2)?;
    let msg = r.bytes(108)?;
    let nsmp = usize::from(r.u8()?);
    let npat = usize::from(r.u8()?);
    let restart = r.u8()?;
    let ord = r.bytes(128)?.to_vec();
    let tempos = r.bytes(128)?.to_vec();
    let breaks = r.bytes(128)?.to_vec();
    let mut heads = Vec::new();
    for _ in 0..nsmp {
        let name = r.text(13)?;
        let len = r.u32le()? as usize;
        let ls = r.u32le()?;
        let le = r.u32le()?;
        heads.push((name, len, ls, le));
    }
    // A pattern's speed and its last row go with it as effects on its
    // first and last rows.
    let mut patterns = Vec::new();
    for p in 0..npat {
        let data = r.bytes(64 * 8 * 3)?;
        let mut pat = Pattern::new(64, 8);
        for row in 0..64 {
            for c in 0..8 {
                let b = &data[(row * 8 + c) * 3..(row * 8 + c) * 3 + 3];
                let mut cell = Cell::default();
                match b[0] {
                    0xFF => {}
                    0xFE => {
                        cell.vol = Vol::Volume;
                        cell.volp = ((b[1] & 15) as u16 * 64 / 15) as u8;
                    }
                    n => {
                        cell.note = ((n >> 2) + 37).min(NOTE_MAX);
                        cell.instr = ((n & 3) << 4 | b[1] >> 4) + 1;
                        cell.vol = Vol::Volume;
                        cell.volp = ((b[1] & 15) as u16 * 64 / 15) as u8;
                    }
                }
                if b[2] != 0xFF {
                    let (cmd, v) = (b[2] >> 4, b[2] & 15);
                    (cell.fx, cell.param) = match cmd {
                        0 => (Fx::PortaUp, v),
                        1 => (Fx::PortaDown, v),
                        2 => (Fx::TonePorta, v),
                        3 => (Fx::FinePortaUp, v),
                        4 => (Fx::Vibrato, 0x10 | v),
                        5 => (Fx::Speed, v.max(1)),
                        // UNIS 669: g is panning, h a fine slide.
                        6 if unis => (Fx::Panning, 128u8.wrapping_add(((v as i8) - 8).wrapping_mul(16) as u8)),
                        _ => (Fx::None, 0),
                    };
                }
                pat.cells[row * 8 + c] = cell;
            }
        }
        let speed = tempos.get(p).copied().unwrap_or(4).max(1);
        let c0 = &mut pat.cells[7];
        if c0.fx == Fx::None {
            (c0.fx, c0.param) = (Fx::Speed, speed);
        }
        let last = usize::from(breaks.get(p).copied().unwrap_or(63)).min(63);
        if last < 63 {
            let c = &mut pat.cells[last * 8 + 7];
            if c.fx == Fx::None || c.fx == Fx::Speed {
                (c.fx, c.param) = (Fx::PatternBreak, 0);
            }
        }
        patterns.push(Arc::new(pat));
    }
    let mut samples = Vec::new();
    for (name, len, ls, le) in heads {
        let pcm = decode(r.bytes_upto(len), Pcm::U8, len);
        samples.push(Sample {
            name,
            wave: Arc::new(Wave::new(8, 1, pcm)),
            looped: if le <= len as u32 && le > ls { Loop::forward(ls, le) } else { Loop::default() },
            ..Sample::empty()
        });
    }
    let orders: Vec<u16> = ord.iter().take_while(|&&o| o < 0xFE).map(|&o| u16::from(o)).collect();
    let mut m = Module {
        title: crate::io::text(&msg[..36]),
        message: msg.chunks(36).map(crate::io::text).collect::<Vec<_>>().join("\n").trim_end().to_string(),
        format: "669".into(),
        format_name: if unis { "UNIS 669".into() } else { "Composer 669".into() },
        tracker: if unis { "UNIS 669".into() } else { "Composer 669".into() },
        kind: Kind::S3m,
        speed: 4,
        // 669 ticks about 32 times a second.
        tempo: 78,
        global_volume: 128,
        mix_volume: 40,
        separation: 128,
        restart: u16::from(restart),
        channels: pans(8, |i| if i % 2 == 0 { 0x30 } else { 0xD0 }),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Scream Tracker 2 ----

pub fn probe_stm(d: &[u8]) -> bool {
    d.len() >= 1168 && d[28] == 0x1A && d[29] == 2 && d[20..28].iter().all(|&c| (32..127).contains(&c))
}

pub fn load_stm(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let title = r.text(20)?;
    let tracker = r.text(8)?;
    r.skip(2)?;
    let (major, minor) = (r.u8()?, r.u8()?);
    let tempo = r.u8()?;
    let npat = usize::from(r.u8()?);
    let gv = r.u8()?;
    r.seek(48)?;
    let mut heads = Vec::new();
    for _ in 0..31 {
        let name = r.text(12)?;
        r.skip(2)?;
        let para = usize::from(r.u16le()?);
        let len = usize::from(r.u16le()?);
        let ls = r.u16le()?;
        let le = r.u16le()?;
        let vol = r.u8()?;
        r.skip(1)?;
        let c2 = r.u16le()?;
        r.skip(6)?;
        heads.push((name, para * 16, len, ls, le, vol, c2));
    }
    let ord_len = if major == 2 && minor == 0 { 64 } else { 128 };
    let orders: Vec<u16> = r.bytes(ord_len)?.iter().take_while(|&&o| o < 99).map(|&o| u16::from(o)).collect();
    let mut patterns = Vec::new();
    for _ in 0..npat {
        let data = r.bytes(64 * 4 * 4)?;
        let mut p = Pattern::new(64, 4);
        for (i, b) in data.chunks_exact(4).enumerate() {
            let mut c = Cell::default();
            match b[0] {
                251..=255 => {}
                // Two octaves below Scream Tracker 3's numbering.
                n => c.note = ((n >> 4) * 12 + (n & 15) + 36 + 1).min(NOTE_MAX),
            }
            if b[0] != 251 {
                c.instr = b[1] >> 3;
                let vol = (b[1] & 7) | (b[2] & 0xF0) >> 1;
                if vol <= 64 {
                    (c.vol, c.volp) = (Vol::Volume, vol);
                }
                let cmd = b[2] & 15;
                (c.fx, c.param) = match cmd {
                    // A is the speed in its high nibble.
                    1 => (Fx::Speed, (b[3] >> 4).max(1)),
                    _ => s3m_effect(cmd, b[3], false),
                };
            }
            p.cells[i] = c;
        }
        patterns.push(Arc::new(p));
    }
    let samples = heads
        .into_iter()
        .map(|(name, at, len, ls, le, vol, c2)| {
            let pcm = decode(d.get(at..).unwrap_or(&[]), Pcm::S8, len);
            Sample {
                name,
                wave: Arc::new(Wave::new(8, 1, pcm)),
                c5: u32::from(c2.max(1000)),
                volume: vol.min(64),
                looped: if le != 0xFFFF && le > ls { Loop::forward(u32::from(ls), u32::from(le)) } else { Loop::default() },
                ..Sample::empty()
            }
        })
        .collect();
    let mut m = Module {
        title,
        format: "stm".into(),
        format_name: "Scream Tracker 2".into(),
        tracker: if tracker == "!Scream!" { format!("Scream Tracker {major}.{minor:02}") } else { tracker },
        kind: Kind::S3m,
        speed: (tempo >> 4).max(1),
        tempo: 125,
        global_volume: gv.min(64) * 2,
        mix_volume: 48,
        separation: 128,
        channels: pans(4, super::common::amiga_pan),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Scream Tracker Music Interface Kit ----
//
// STMIK's .stx: Scream Tracker 2's song in a beta of Scream Tracker 3's
// file (parapointers, S3M's packed patterns and sample headers), made by
// STM2STX. Described by OpenMPT's Load_stm.cpp (BSD) and libxmp's
// stx_load.c (MIT).

pub fn probe_stx(d: &[u8]) -> bool {
    if d.len() < 64 || &d[60..64] != b"SCRM" || !d[20..28].iter().all(|&c| (32..127).contains(&c)) {
        return false;
    }
    let w = |at: usize| u16::from_le_bytes([d[at], d[at + 1]]);
    let psize = w(28);
    (psize >= 64 || psize == 0x1A) && psize <= 0x840 && w(30) == 0 && d[38..42] == [0; 4] && (d[42] <= 64 || d[42] == 0x58) && d[44..48] == [1, 0, 0, 0] && w(48) <= 64 && w(50) <= 96 && (w(52) <= 0x81 || w(52) == 0x101)
}

pub fn load_stx(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let title = r.text(20)?;
    let tracker = r.text(8)?;
    let psize = r.u16le()?;
    r.skip(2)?;
    let pat_table = usize::from(r.u16le()?) * 16;
    let smp_table = usize::from(r.u16le()?) * 16;
    let chn_table = usize::from(r.u16le()?) * 16;
    r.seek(42)?;
    let gv = r.u8()?;
    let tempo = match r.u8()? {
        0 => 0x60,
        t => t,
    };
    r.seek(48)?;
    let npat = usize::from(r.u16le()?);
    let nsmp = usize::from(r.u16le()?);
    let nord = usize::from(r.u16le()?);
    let words = |at: usize, n: usize| -> Result<Vec<usize>> {
        let mut t = Reader::at(d, at);
        (0..n).map(|_| t.u16le().map(|p| usize::from(p) * 16)).collect()
    };
    let pat_ptr = words(pat_table, npat)?;
    let smp_ptr = words(smp_table, nsmp)?;
    // The order list: a byte and four unknown ones each, after the channel table.
    let mut orders = Vec::with_capacity(nord);
    let mut o = Reader::at(d, chn_table + 32);
    for _ in 0..nord {
        let p = o.u8()?;
        o.skip(4)?;
        match p {
            99 | 255 => {}
            p if p < 64 => orders.push(u16::from(p)),
            _ => return bad("its order list points past its patterns"),
        }
    }

    // S3M's sample headers; the samples signed.
    let mut samples = Vec::with_capacity(nsmp);
    for &p in &smp_ptr {
        let mut h = Reader::at(d, p);
        let mut s = Sample::empty();
        h.skip(1)?;
        s.filename = h.text(12)?;
        let hi = h.u8()?;
        let lo = h.u16le()?;
        let at = (usize::from(hi) << 16 | usize::from(lo)) * 16;
        let len = (h.u32le()? as usize).min(4 << 20);
        let ls = h.u32le()?;
        let le = h.u32le()?;
        s.volume = h.u8()?.min(64);
        h.skip(2)?;
        let flags = h.u8()?;
        s.c5 = h.u32le()?.max(1000);
        h.skip(12)?;
        s.name = h.text(28)?;
        if len > 0 {
            s.wave = Arc::new(Wave::new(8, 1, decode(d.get(at..).unwrap_or(&[]), Pcm::S8, len)));
        }
        if flags & 1 != 0 && le > ls {
            s.looped = Loop::forward(ls, le);
        }
        samples.push(s);
    }

    // STM2STX 1.0 starts each pattern with its length, as S3M does; 1.1 doesn't.
    let sized = psize != 0x1A && pat_ptr.first().is_some_and(|&p| d.get(p..p + 2) == Some(&psize.to_le_bytes()[..]));
    let mut patterns = Vec::with_capacity(npat);
    for &p in &pat_ptr {
        let mut pat = Pattern::new(64, 4);
        let mut pr = Reader::at(d, p + if sized { 2 } else { 0 });
        let mut row = 0;
        while row < 64 {
            let Ok(what) = pr.u8() else { break };
            if what == 0 {
                row += 1;
                continue;
            }
            let mut cell = Cell::default();
            if what & 32 != 0 {
                let (Ok(n), Ok(i)) = (pr.u8(), pr.u8()) else { break };
                cell.note = match n {
                    254 => NOTE_CUT,
                    // Two octaves below Scream Tracker 3's numbering, as STM's.
                    n if n < 0xF0 => ((n >> 4) * 12 + (n & 0x0F) + 36 + 1).min(NOTE_MAX),
                    _ => 0,
                };
                cell.instr = i;
            }
            if what & 64 != 0 {
                let Ok(v) = pr.u8() else { break };
                (cell.vol, cell.volp) = (Vol::Volume, v.min(64));
            }
            if what & 128 != 0 {
                let (Ok(c), Ok(i)) = (pr.u8(), pr.u8()) else { break };
                (cell.fx, cell.param) = match c & 15 {
                    // A is the speed in its high nibble.
                    1 => (Fx::Speed, (i >> 4).max(1)),
                    // Scream Tracker 2 has no effect memory: no value, no effect.
                    _ if i == 0 => (Fx::None, 0),
                    // No fine slides either: the low nibble wins.
                    4 => s3m_effect(4, if i & 15 != 0 { i & 15 } else { i & 0xF0 }, false),
                    c @ 2..=10 => s3m_effect(c, i, false),
                    _ => (Fx::None, 0),
                };
            }
            if let Some(c) = pat.cell_mut(row, usize::from(what & 31)) {
                *c = cell;
            }
        }
        patterns.push(Arc::new(pat));
    }

    let mut m = Module {
        title,
        format: "stx".into(),
        format_name: "Scream Tracker Music Interface Kit".into(),
        tracker: if tracker == "BMOD2STM" { tracker } else { format!("STM2STX 1.{}", if sized { 0 } else { 1 }) },
        kind: Kind::S3m,
        speed: (tempo >> 4).max(1),
        tempo: 125,
        global_volume: gv.min(64) * 2,
        mix_volume: 48,
        separation: 128,
        channels: pans(4, super::common::amiga_pan),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- PolyTracker ----

pub fn probe_ptm(d: &[u8]) -> bool {
    d.len() >= 608 && &d[44..48] == b"PTMF"
}

pub fn load_ptm(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let title = r.text(28)?;
    r.skip(1)?;
    let version = r.u16le()?;
    r.skip(1)?;
    let ordnum = usize::from(r.u16le()?);
    let insnum = usize::from(r.u16le()?);
    let patnum = usize::from(r.u16le()?);
    let nch = usize::from(r.u16le()?).clamp(1, 32);
    r.seek(64)?;
    let pan = r.bytes(32)?.to_vec();
    let ord = r.bytes(256)?.to_vec();
    let mut pat_ptr = Vec::new();
    for _ in 0..128 {
        pat_ptr.push(usize::from(r.u16le()?) * 16);
    }
    let mut samples = Vec::new();
    for _ in 0..insnum {
        let start = r.pos;
        let kind = r.u8()?;
        let filename = r.text(12)?;
        let vol = r.u8()?;
        let c4 = r.u16le()?;
        r.skip(2)?;
        let off = r.u32le()? as usize;
        let len = r.u32le()? as usize;
        let ls = r.u32le()?;
        let le = r.u32le()?;
        r.skip(14)?;
        let name = r.text(28)?;
        r.seek(start + 80)?;
        let mut s = Sample { name, filename, volume: vol.min(64), c5: u32::from(c4.max(1000)), ..Sample::empty() };
        if kind & 3 == 1 && len > 0 {
            let b16 = kind & 16 != 0;
            let bytes = d.get(off..).unwrap_or(&[]);
            // Deltas, a byte at a time even for 16-bit samples.
            let raw = decode(bytes, Pcm::D8, len);
            let pcm = if b16 { raw.chunks_exact(2).map(|c| ((c[0] >> 8) as u8 as u16 | (((c[1] >> 8) as u8 as u16) << 8)) as i16).collect() } else { raw };
            let div = if b16 { 2 } else { 1 };
            s.wave = Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm));
            if kind & 4 != 0 && le > ls {
                s.looped = Loop { kind: if kind & 8 != 0 { LoopKind::PingPong } else { LoopKind::Forward }, start: ls / div, end: le.saturating_sub(1).max(ls + 1) / div };
            }
        }
        samples.push(s);
    }
    let mut patterns = Vec::new();
    for &p in pat_ptr.iter().take(patnum) {
        let mut pat = Pattern::new(64, nch);
        if p > 0 {
            let mut pr = Reader::at(d, p);
            let mut row = 0;
            while row < 64 {
                let Ok(what) = pr.u8() else { break };
                if what == 0 {
                    row += 1;
                    continue;
                }
                let ch = usize::from(what & 31);
                let mut c = Cell::default();
                if what & 32 != 0 {
                    let (Ok(n), Ok(i)) = (pr.u8(), pr.u8()) else { break };
                    c.note = match n {
                        254 => NOTE_CUT,
                        1..=120 => (n + 12).min(NOTE_MAX),
                        _ => 0,
                    };
                    c.instr = i;
                }
                if what & 64 != 0 {
                    let (Ok(f), Ok(v)) = (pr.u8(), pr.u8()) else { break };
                    (c.fx, c.param) = match f {
                        0..=15 => mod_effect(f, v, false),
                        16 => (Fx::GlobalVolume, v.min(64) * 2),
                        17 => (Fx::Retrig, v),
                        18 => (Fx::FineVibrato, v),
                        19 => (Fx::NoteSlideDown, v),
                        20 => (Fx::NoteSlideUp, v),
                        _ => (Fx::None, 0),
                    };
                }
                if what & 128 != 0 {
                    let Ok(v) = pr.u8() else { break };
                    (c.vol, c.volp) = (Vol::Volume, v.min(64));
                }
                if let Some(slot) = pat.cell_mut(row, ch) {
                    *slot = c;
                }
            }
        }
        patterns.push(Arc::new(pat));
    }
    let orders = ord[..ordnum.min(256)].iter().take_while(|&&o| o != 255).map(|&o| u16::from(o)).collect();
    let mut m = Module {
        title,
        format: "ptm".into(),
        format_name: "PolyTracker".into(),
        tracker: format!("PolyTracker {}.{:02x}", version >> 8, version & 0xFF),
        kind: Kind::S3m,
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels: pans(nch, |i| (pan[i] & 15) * 17),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- General DigiMusic ----

pub fn probe_gdm(d: &[u8]) -> bool {
    d.len() >= 157 && &d[..4] == b"GDM\xFE" && &d[71..75] == b"GMFS"
}

pub fn load_gdm(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    r.skip(4)?;
    let title = r.text(32)?;
    let artist = r.text(32)?;
    r.seek(75)?;
    let _fmt_ver = r.u16le()?;
    let tracker_id = r.u16le()?;
    let tracker_ver = r.u16le()?;
    let pan = r.bytes(32)?.to_vec();
    let gv = r.u8()?;
    let speed = r.u8()?;
    let bpm = r.u8()?;
    let original = r.u16le()?;
    let ord_off = r.u32le()? as usize;
    let ord_n = usize::from(r.u8()?) + 1;
    let pat_off = r.u32le()? as usize;
    let pat_n = usize::from(r.u8()?) + 1;
    let smp_off = r.u32le()? as usize;
    let data_off = r.u32le()? as usize;
    let smp_n = usize::from(r.u8()?) + 1;
    let msg_off = r.u32le()? as usize;
    let msg_len = r.u32le()? as usize;
    let nch = (0..32).rev().find(|&i| pan[i] != 0xFF).map_or(4, |i| i + 1);
    let orders: Vec<u16> = d.get(ord_off..ord_off + ord_n).ok_or(crate::io::Error::Bad("its orders are cut short".into()))?.iter().take_while(|&&o| o != 255).map(|&o| u16::from(o)).collect();
    let mut sr = Reader::at(d, smp_off);
    let mut heads = Vec::new();
    for _ in 0..smp_n {
        let name = sr.text(32)?;
        let filename = sr.text(12)?;
        sr.skip(1)?;
        let len = sr.u32le()? as usize;
        let lb = sr.u32le()?;
        let le = sr.u32le()?;
        let flags = sr.u8()?;
        let c4 = sr.u16le()?;
        let vol = sr.u8()?;
        let pan = sr.u8()?;
        heads.push((name, filename, len, lb, le, flags, c4, vol, pan));
    }
    let mut at = data_off;
    let mut samples = Vec::new();
    for (name, filename, len, lb, le, flags, c4, vol, span) in heads {
        let b16 = flags & 2 != 0;
        let bytes = d.get(at..).unwrap_or(&[]);
        at += len;
        let pcm = decode(bytes, if b16 { Pcm::U16le } else { Pcm::U8 }, len / if b16 { 2 } else { 1 });
        let div = if b16 { 2 } else { 1 };
        samples.push(Sample {
            name,
            filename,
            wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
            c5: u32::from(c4.max(1000)),
            volume: if flags & 4 != 0 { vol.min(64) } else { 64 },
            pan: (flags & 8 != 0 && span < 16).then(|| span * 17),
            looped: if flags & 1 != 0 && le > lb { Loop::forward(lb / div, (le - 1).max(lb + 1) / div) } else { Loop::default() },
            ..Sample::empty()
        });
    }
    let mut pr = Reader::at(d, pat_off);
    let mut patterns = Vec::new();
    for _ in 0..pat_n {
        let len = usize::from(pr.u16le()?);
        let data = pr.bytes(len.saturating_sub(2))?;
        let mut p = Reader::new(data);
        let mut pat = Pattern::new(64, nch);
        let mut row = 0;
        while row < 64 && !p.eof() {
            let what = p.u8()?;
            if what == 0 {
                row += 1;
                continue;
            }
            let ch = usize::from(what & 31);
            let mut c = Cell::default();
            if what & 32 != 0 {
                let n = p.u8()?;
                let i = p.u8()?;
                if n & 0x7F != 0 {
                    let n = n & 0x7F;
                    c.note = ((n >> 4) * 12 + (n & 15) + 12).clamp(NOTE_MIN, NOTE_MAX);
                }
                c.instr = i;
            }
            if what & 64 != 0 {
                loop {
                    let e = p.u8()?;
                    let v = p.u8()?;
                    let (fx, param) = match e & 31 {
                        0x00..=0x0F => mod_effect(e & 31, v, false),
                        0x10 => (Fx::Arpeggio, v),
                        0x12 => (Fx::Retrig, v),
                        0x13 => (Fx::GlobalVolume, v.min(64) * 2),
                        0x14 => (Fx::FineVibrato, v),
                        0x1E => (Fx::S3mEx, v),
                        0x1F => (Fx::Tempo, v),
                        _ => (Fx::None, 0),
                    };
                    if fx == Fx::Volume && c.vol == Vol::None {
                        (c.vol, c.volp) = (Vol::Volume, param);
                    } else if c.fx == Fx::None && fx != Fx::None {
                        (c.fx, c.param) = (fx, param);
                    }
                    if e & 32 == 0 {
                        break;
                    }
                }
            }
            if let Some(slot) = pat.cell_mut(row, ch) {
                *slot = c;
            }
        }
        patterns.push(Arc::new(pat));
    }
    let names = ["", "MOD", "MTM", "S3M", "669", "FAR", "ULT", "STM", "MED", "PSM"];
    let mut m = Module {
        title,
        artist,
        message: if msg_len > 0 { d.get(msg_off..msg_off + msg_len).map(message).unwrap_or_default() } else { String::new() },
        format: "gdm".into(),
        format_name: "General DigiMusic".into(),
        tracker: format!(
            "{} {}.{:02}{}",
            if tracker_id == 0 { "2GDM" } else { "General DigiMusic" },
            tracker_ver & 0xFF,
            tracker_ver >> 8,
            names.get(usize::from(original)).filter(|n| !n.is_empty()).map(|n| format!(", from {n}")).unwrap_or_default()
        ),
        kind: Kind::S3m,
        speed: speed.max(1),
        tempo: u16::from(bpm.max(32)),
        global_volume: gv.min(64) * 2,
        mix_volume: 48,
        separation: 128,
        channels: pans(nch, |i| if pan[i] < 16 { pan[i] * 17 } else { 128 }),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Digital Sound Interface Kit ----

pub fn probe_dsm(d: &[u8]) -> bool {
    d.len() >= 12 && &d[..4] == b"RIFF" && &d[8..12] == b"DSMF"
}

pub fn load_dsm(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 12);
    let mut m = Module { format: "dsm".into(), format_name: "Digital Sound Interface Kit".into(), tracker: "DSIK".into(), kind: Kind::Mod, ..Default::default() };
    let mut nch = 16;
    let mut pan = vec![0x40u8; 16];
    while r.left() >= 8 {
        let id = r.tag()?;
        let len = r.u32le()? as usize;
        // DSIK doesn't pad its chunks to even lengths.
        let body = r.bytes_upto(len);
        let mut c = Reader::new(body);
        match &id {
            b"SONG" => {
                m.title = c.text(28)?;
                c.skip(8)?;
                let ordnum = usize::from(c.u16le()?);
                c.skip(4)?;
                nch = usize::from(c.u16le()?).clamp(1, 16);
                m.global_volume = c.u8()?.min(64) * 2;
                m.mix_volume = (c.u8()? & 0x7F).max(16);
                m.speed = c.u8()?.max(1);
                m.tempo = u16::from(c.u8()?.max(32));
                pan = c.bytes(16)?.to_vec();
                m.orders = c.bytes(128)?[..ordnum.min(128)].iter().take_while(|&&o| o < 0xFE).map(|&o| u16::from(o)).collect();
            }
            b"INST" => {
                let filename = c.text(13)?;
                let flags = c.u16le()?;
                let vol = c.u8()?;
                let n = c.u32le()? as usize;
                let ls = c.u32le()?;
                let le = c.u32le()?;
                c.skip(4)?;
                let c2 = c.u16le()?;
                c.skip(2)?;
                let name = c.text(28)?;
                let pcm = decode(c.bytes_upto(n), if flags & 2 != 0 { Pcm::S8 } else { Pcm::U8 }, n);
                m.samples.push(Sample {
                    name,
                    filename,
                    wave: Arc::new(Wave::new(8, 1, pcm)),
                    c5: u32::from(c2.max(1000)),
                    volume: vol.min(64),
                    looped: if flags & 1 != 0 && le > ls { Loop::forward(ls, le) } else { Loop::default() },
                    ..Sample::empty()
                });
            }
            b"PATT" => {
                c.skip(2)?;
                let mut pat = Pattern::new(64, nch);
                let mut row = 0;
                while row < 64 && !c.eof() {
                    let f = c.u8()?;
                    if f == 0 {
                        row += 1;
                        continue;
                    }
                    let ch = usize::from(f & 15);
                    let mut cell = Cell::default();
                    if f & 0x80 != 0 {
                        let n = c.u8()?;
                        if n > 0 && n < 0x80 {
                            cell.note = (n + 12).min(NOTE_MAX);
                        }
                    }
                    if f & 0x40 != 0 {
                        cell.instr = c.u8()?;
                    }
                    if f & 0x20 != 0 {
                        let v = c.u8()?;
                        if v <= 64 {
                            (cell.vol, cell.volp) = (Vol::Volume, v);
                        }
                    }
                    if f & 0x10 != 0 {
                        let (e, p) = (c.u8()?, c.u8()?);
                        (cell.fx, cell.param) = if e == 8 {
                            // DSIK's panning is 0 to 80, A4 surround.
                            if p <= 0x80 {
                                (Fx::Panning, (u16::from(p) * 255 / 128) as u8)
                            } else {
                                (Fx::None, 0)
                            }
                        } else {
                            mod_effect(e, p, false)
                        };
                    }
                    if let Some(s) = pat.cell_mut(row, ch) {
                        *s = cell;
                    }
                }
                m.patterns.push(Arc::new(pat));
            }
            _ => {}
        }
    }
    if m.patterns.is_empty() {
        return bad("it has no patterns");
    }
    m.channels = pans(nch, |i| if pan[i] <= 0x80 { (u16::from(pan[i]) * 255 / 128) as u8 } else { 128 });
    m.separation = 128;
    m.tidy();
    Ok(m)
}
