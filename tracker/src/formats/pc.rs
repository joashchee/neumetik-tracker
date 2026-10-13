//! More of the PC's trackers: Astroidea's XMF (Imperium Galactica's
//! music), Composer 670 (CDFM, .c67), Funktracker (.fnk), Disorder
//! Tracker 2 (.plm) and Real Tracker (.rtm).

use std::sync::Arc;

use super::common::mod_effect;
use crate::io::{bad, decode, Pcm, Reader, Result};
use crate::model::*;

/// A second effect in a cell: a volume goes to the volume column, else
/// it plays when the first is empty.
fn second_effect(cell: &mut Cell, fx: Fx, param: u8) {
    if fx == Fx::Volume && cell.vol == Vol::None {
        (cell.vol, cell.volp) = (Vol::Volume, param.min(64));
    } else if cell.fx == Fx::None {
        (cell.fx, cell.param) = (fx, param);
    }
}

fn u24le(b: &[u8]) -> u32 {
    u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16
}

// ---- Astroidea XMF ----
//
// A type byte, 256 sample headers, 256 orders, the channels' panning,
// patterns of six-byte cells (two effects each), the samples.

fn xmf_sample_ok(h: &[u8], kind: u8) -> bool {
    let (ls, le, start, end, flags, rate) = (u24le(&h[0..]), u24le(&h[3..]), u24le(&h[6..]), u24le(&h[9..]), h[13], u16::from_le_bytes([h[14], h[15]]));
    if flags & !0x1C != 0 || flags & 0x18 == 0x10 || start > end {
        return false;
    }
    let len = end - start;
    if (kind != 2 && len > 0 && rate < 100) || (kind == 2 && len > 0 && rate >= 0x8000) || (flags & 4 != 0 && len % 2 != 0) {
        return false;
    }
    if (flags & 8 != 0 && le == 0) || ls > le || ls > len {
        return false;
    }
    le == 0 || (le < len && ls < le)
}

pub fn probe_xmf(d: &[u8]) -> bool {
    if d.len() < 1 + 256 * 16 + 256 + 2 || !(2..=4).contains(&d[0]) {
        return false;
    }
    let heads = || d[1..1 + 256 * 16].chunks_exact(16);
    heads().all(|h| xmf_sample_ok(h, d[0])) && heads().any(|h| u24le(&h[9..]) > u24le(&h[6..])) && d[1 + 256 * 16 + 256] <= 31
}

pub fn load_xmf(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    // 2 and 3 slide to a new note on their own (as UltraTracker); 4 as MOD.
    let kind = r.u8()?;
    let heads: Vec<&[u8]> = r.bytes(256 * 16)?.chunks_exact(16).collect();
    let nsmp = heads.iter().rposition(|h| u24le(&h[9..]) > u24le(&h[6..])).map_or(0, |i| i + 1);
    let orders: Vec<u16> = r.bytes(256)?.iter().take_while(|&&o| o != 0xFF).map(|&o| u16::from(o)).collect();
    let nch = usize::from(r.u8()?) + 1;
    let npat = usize::from(r.u8()?) + 1;
    let pans = r.bytes(nch)?.to_vec();
    let effect = |cmd: u8, param: u8| -> (Fx, u8) {
        match cmd {
            0x0B => (Fx::PositionJump, param.saturating_add(u8::from(param < 0xFF))),
            0x10 => (Fx::ModEx, 0x80 | (param & 0x0F)),
            0x11 => (Fx::ModEx, 0x90 | (param & 0x0F)),
            // Volume runs 0 to 255.
            0x0C => (Fx::Volume, u16::from(param).div_ceil(4).min(64) as u8),
            0x0F if kind != 4 && param == 0x20 => (Fx::Speed, param),
            0x12.. => (Fx::None, 0),
            _ => mod_effect(cmd, param, false),
        }
    };
    let mut patterns = Vec::with_capacity(npat);
    for _ in 0..npat {
        let data = r.bytes(64 * nch * 6)?;
        let mut p = Pattern::new(64, nch);
        for (i, b) in data.chunks_exact(6).enumerate() {
            let mut cell = Cell { note: if (1..=77).contains(&b[0]) { b[0] + 36 } else { 0 }, instr: b[1], ..Default::default() };
            (cell.fx, cell.param) = effect(b[2], b[5]);
            let (fx2, p2) = effect(b[3], b[4]);
            if fx2 != Fx::None {
                second_effect(&mut cell, fx2, p2);
            }
            p.cells[i] = cell;
        }
        patterns.push(Arc::new(p));
    }
    let mut samples = Vec::with_capacity(nsmp);
    for h in &heads[..nsmp] {
        let (ls, le, start, end, flags) = (u24le(&h[0..]), u24le(&h[3..]), u24le(&h[6..]), u24le(&h[9..]), h[13]);
        let b16 = flags & 4 != 0;
        let len = (end - start) as usize;
        let pcm = decode(r.bytes_upto(len), if b16 { Pcm::S16le } else { Pcm::S8 }, len / if b16 { 2 } else { 1 });
        let div = if b16 { 2 } else { 1 };
        let kind_loop = if flags & 0x10 != 0 { LoopKind::PingPong } else { LoopKind::Forward };
        samples.push(Sample {
            wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
            c5: if kind == 2 { 8363 } else { u32::from(u16::from_le_bytes([h[14], h[15]])) },
            volume: u16::from(h[12]).div_ceil(4).min(64) as u8,
            looped: if flags & 8 != 0 { Loop { kind: kind_loop, start: (ls + 1) / div, end: (le + 1) / div } } else { Loop::default() },
            ..Sample::empty()
        });
        let frames = samples.last().map_or(0, |s| s.frames() as u32);
        if let Some(s) = samples.last_mut() {
            s.looped = s.looped.clamped(frames);
        }
    }
    let mut m = Module {
        format: "xmf".into(),
        format_name: "Astroidea XMF".into(),
        tracker: format!("Astroidea XMF (type {kind})"),
        kind: Kind::Mod,
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: pans.iter().map(|&p| Channel { pan: (p & 15) * 0x11, ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Composer 670 ----
//
// CDFM's: 32 PCM samples for four channels and 32 FM (OPL2) instruments
// for nine, patterns as a stream of commands. The FM instruments have
// no PCM: neumetik-synth plays them.

const C67_HEADER: usize = 1954;

pub fn probe_c67(d: &[u8]) -> bool {
    if d.len() < C67_HEADER + 1024 || !(1..=15).contains(&d[0]) {
        return false;
    }
    let le32 = |i: usize| u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
    if d[1698..1954].iter().any(|&o| o >= 128 && o != 0xFF) {
        return false;
    }
    let mut any = false;
    for s in 0..32 {
        let h = 418 + s * 16;
        let fm = 1346 + s * 11;
        let (len, ls, le) = (le32(h + 4), le32(h + 8), le32(h + 12));
        if d[2 + s * 13 + 12] != 0 || le32(h) != 0 || len > 0xF_FFFF || d[930 + s * 13 + 12] != 0 || d[fm] & 0xF0 != 0 || d[fm + 5] & 0xFC != 0 || d[fm + 10] & 0xFC != 0 {
            return false;
        }
        if len != 0 && le < 0xF_FFFF && (le > len || ls > le) {
            return false;
        }
        any |= len != 0 || d[fm..fm + 11].iter().any(|&b| b != 0);
    }
    any && (0..128).all(|p| {
        let (off, len) = (le32(C67_HEADER + p * 4) as usize, le32(C67_HEADER + 512 + p * 4) as usize);
        off <= 0xFF_FFFF && (3..=0x1000).contains(&len) && 2978 + off + len <= d.len()
    })
}

pub fn load_c67(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let speed = r.u8()?;
    let restart = r.u8()?;
    let mut names = Vec::with_capacity(32);
    for _ in 0..32 {
        names.push(r.text(13)?);
    }
    let mut heads = Vec::with_capacity(32);
    for _ in 0..32 {
        r.skip(4)?;
        heads.push((r.u32le()? as usize, r.u32le()?, r.u32le()?));
    }
    let mut fm_names = Vec::with_capacity(32);
    for _ in 0..32 {
        fm_names.push(r.text(13)?);
    }
    let mut fm_patches = Vec::with_capacity(32);
    for _ in 0..32 {
        let b: [u8; 11] = r.bytes(11)?.try_into().expect("eleven bytes");
        fm_patches.push(FmPatch::from_c67(&b));
    }
    let orders: Vec<u16> = r.bytes(256)?.iter().take_while(|&&o| o != 0xFF).map(|&o| u16::from(o)).collect();
    let mut places = Vec::with_capacity(128);
    for _ in 0..128 {
        places.push(r.u32le()? as usize);
    }
    let mut ends = 2978;
    let mut patterns = Vec::with_capacity(128);
    for p in 0..128 {
        let len = r.u32le()? as usize;
        let at = 2978 + places[p];
        ends = ends.max(at + len);
        let mut pr = Reader::new(d.get(at..at + len).unwrap_or(&[]));
        let mut pat = Pattern::new(64, 13);
        let mut row = 0usize;
        // CDFM's volumes are 0 to 15 and never silent: PCM goes up in
        // fours, FM on a curve.
        const FM_VOLUME: [u8; 16] = [0x08, 0x10, 0x18, 0x20, 0x28, 0x2C, 0x30, 0x34, 0x36, 0x38, 0x3A, 0x3C, 0x3D, 0x3E, 0x3F, 0x40];
        let volume = |v: u8, fm: bool| if fm { FM_VOLUME[usize::from(v & 15)] } else { 4 + (v & 15) * 4 };
        while row < 64 {
            let Ok(cmd) = pr.u8() else { break };
            match cmd {
                0x00..=0x0C => {
                    let (Ok(note), Ok(iv)) = (pr.u8(), pr.u8()) else { break };
                    let fm = cmd >= 4;
                    let cell = &mut pat.cells[row * 13 + usize::from(cmd)];
                    cell.note = (if fm { 13 } else { 37 }) + (note & 0x0F).min(11) + ((note >> 4) & 7) * 12;
                    cell.instr = (if fm { 33 } else { 1 }) + (iv >> 4) + ((note & 0x80) >> 3);
                    (cell.vol, cell.volp) = (Vol::Volume, volume(iv, fm));
                }
                0x20..=0x2C => {
                    let Ok(v) = pr.u8() else { break };
                    let c = usize::from(cmd - 0x20);
                    pat.cells[row * 13 + c].vol = Vol::Volume;
                    pat.cells[row * 13 + c].volp = volume(v, c >= 4);
                }
                0x40 => row += usize::from(pr.u8().unwrap_or(64)),
                0x60 => {
                    if row > 0 {
                        let cell = &mut pat.cells[(row - 1) * 13];
                        (cell.fx, cell.param) = (Fx::PatternBreak, 0);
                    }
                    break;
                }
                _ => return bad("a pattern has a command CDFM doesn't write"),
            }
        }
        patterns.push(Arc::new(pat));
    }
    let mut at = ends;
    let mut samples = Vec::with_capacity(64);
    for (name, &(len, ls, le)) in names.into_iter().zip(&heads) {
        let pcm = decode(d.get(at..).unwrap_or(&[]), Pcm::U8, len);
        at += len;
        samples.push(Sample {
            name,
            wave: Arc::new(Wave::new(8, 1, pcm)),
            c5: 8287,
            looped: if le as usize <= len && le > ls { Loop::forward(ls, le) } else { Loop::default() },
            ..Sample::empty()
        });
    }
    for (name, patch) in fm_names.into_iter().zip(fm_patches) {
        let fm = (!patch.is_silent()).then_some(patch);
        samples.push(Sample { name: if name.is_empty() { String::new() } else { format!("{name} (FM)") }, c5: 8287, fm, ..Sample::empty() });
    }
    let mut m = Module {
        format: "c67".into(),
        format_name: "CDFM".into(),
        tracker: "Composer 670".into(),
        kind: Kind::S3m,
        speed,
        tempo: 143,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        restart: u16::from(restart),
        channels: (0..13)
            .map(|c| Channel { pan: if c >= 4 { 128 } else if c & 1 != 0 { 192 } else { 64 }, name: if c >= 4 { format!("FM {}", c - 3) } else { String::new() }, ..Default::default() })
            .collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    if usize::from(m.restart) >= m.orders.len() {
        m.restart = 0;
    }
    m.message = m.samples.iter().map(|s| s.name.clone()).filter(|n| !n.is_empty()).collect::<Vec<_>>().join("\n");
    m.tidy();
    Ok(m)
}

// ---- Funktracker ----
//
// "Funk", the date and the machine it was made on, the file's size, the
// variant (F2xx, Fkxx, Fvxx), orders, a break row for each pattern, 64
// samples, patterns of three-byte cells. Its slides go on until stopped
// (or another takes over); here they play on the rows they're written.

pub fn probe_fnk(d: &[u8]) -> bool {
    if d.len() < 2449 || &d[..4] != b"Funk" || d[5] >> 1 < 10 || d[6] >> 4 > 7 || d[6] & 15 > 9 {
        return false;
    }
    let size = u32::from_le_bytes([d[8], d[9], d[10], d[11]]) as usize;
    // A file cut short still reads, as far as it goes.
    size >= 1024 && size >= d.len() && d[273..401].iter().all(|&b| b < 64)
}

pub fn load_fnk(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    r.skip(4)?;
    let info = r.bytes(4)?.to_vec();
    r.skip(4)?;
    let variant = r.bytes(4)?.to_vec();
    let _loop = r.u8()?;
    let orders: Vec<u16> = r.bytes(256)?.iter().take_while(|&&o| o != 0xFF).map(|&o| u16::from(o)).collect();
    let breaks = r.bytes(128)?.to_vec();
    let mut heads = Vec::with_capacity(64);
    for _ in 0..64 {
        let name = r.text(19)?;
        let ls = r.u32le()?;
        let len = r.u32le()? as usize;
        let vol = r.u8()?;
        let pan = r.u8()?;
        r.skip(3)?;
        heads.push((name, ls, len, vol, pan));
    }
    let npat = orders.iter().max().map_or(1, |&o| usize::from(o) + 1);
    if npat > 128 {
        return bad("it has more than 128 patterns");
    }
    let mut tempo: i32 = 125;
    let (nch, tracker, gus_pan) = match (variant[0], variant[1]) {
        (b'F', b'2') => {
            // A tempo away from 125.
            let shift = i32::from((info[3] >> 1) & 0x3F);
            tempo += if (info[3] as i8 >> 1) & 0x40 != 0 { -shift } else { shift };
            (digits(&variant[2..]).unwrap_or(8), "Funktracker GOLD", true)
        }
        (b'F', b'v') => (digits(&variant[2..]).unwrap_or(8), "Funktracker", true),
        (b'F', b'k') => (digits(&variant[2..]).unwrap_or(8), "Funktracker", false),
        _ => (8, "Funktracker (DOS32)", false),
    };
    if nch == 0 || nch > 64 {
        return bad(format!("it says it has {nch} channels"));
    }
    let mut patterns = Vec::with_capacity(npat);
    for p in 0..npat {
        let data = r.bytes(64 * nch * 3)?;
        let mut pat = Pattern::new(64, nch);
        for (i, b) in data.chunks_exact(3).enumerate() {
            let n = b[0] >> 2;
            let mut cell = Cell::default();
            // 3F: an effect alone; 3E and 3D: a sample and an effect.
            if n < 0x3D {
                cell.note = n + 37;
            }
            if n < 0x3F {
                cell.instr = 1 + (b[1] >> 4) + ((b[0] & 3) << 4);
            }
            let x = b[2];
            (cell.fx, cell.param) = match b[1] & 0x0F {
                0x0 => (Fx::PortaUp, x),
                0x1 => (Fx::PortaDown, x),
                0x2 => (Fx::TonePorta, x),
                0x3 => (Fx::Vibrato, x),
                0x6 => (Fx::VolSlide, (x.saturating_mul(2)).min(15) << 4),
                0x7 => (Fx::VolSlide, (x.saturating_mul(2)).min(15)),
                0xB => (Fx::Arpeggio, x),
                0xD => (Fx::Volume, x / 4),
                0xE => match x >> 4 {
                    0x1 => (Fx::ModEx, 0xC0 | (x & 15)),
                    0x2 => (Fx::ModEx, 0xD0 | (x & 15)),
                    0xD => (Fx::ModEx, 0x90 | (x & 15)),
                    0xE => (Fx::Panning, 8 + ((x & 15) << 4)),
                    0xF if x & 15 > 0 => (Fx::Speed, x & 15),
                    _ => (Fx::None, 0),
                },
                _ => (Fx::None, 0),
            };
            pat.cells[i] = cell;
        }
        // The pattern ends after its break row.
        let last = usize::from(breaks[p]);
        if last < 63 {
            if let Some(c) = (0..nch).map(|c| last * nch + c).find(|&i| pat.cells[i].fx == Fx::None) {
                (pat.cells[c].fx, pat.cells[c].param) = (Fx::PatternBreak, 0);
            }
        }
        patterns.push(Arc::new(pat));
    }
    let mut samples = Vec::with_capacity(64);
    for (name, ls, len, vol, pan) in heads {
        let pcm = if len > 2 { decode(r.bytes_upto(len), Pcm::S8, len) } else { Vec::new() };
        samples.push(Sample {
            name,
            wave: Arc::new(Wave::new(8, 1, pcm)),
            volume: vol / 4,
            pan: gus_pan.then_some(pan),
            looped: if ls != u32::MAX && (ls as usize) < len { Loop::forward(ls, len as u32) } else { Loop::default() },
            ..Sample::empty()
        });
    }
    let mut m = Module {
        format: "fnk".into(),
        format_name: "Funktracker".into(),
        tracker: tracker.into(),
        kind: Kind::Mod,
        flags: Flags { linear: true, ..Default::default() },
        speed: 4,
        tempo: (tempo * 4 / 5).clamp(32, 255) as u16,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: (0..nch).map(|c| Channel { pan: if gus_pan { 128 } else if c % 2 == 0 { 0 } else { 255 }, ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.message = m.samples.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join("\n").trim_end().to_string();
    m.tidy();
    Ok(m)
}

/// Two ASCII digits as a number.
fn digits(b: &[u8]) -> Option<usize> {
    let d = |c: u8| c.is_ascii_digit().then(|| usize::from(c - b'0'));
    Some(d(b[0])? * 10 + d(b[1])?)
}

// ---- Disorder Tracker 2 ----
//
// The song is one long pattern: blocks of rows placed along it, each on
// a channel and those after it. Read as patterns of 64 rows, with one
// channel more for the jumps.

pub fn probe_plm(d: &[u8]) -> bool {
    d.len() >= 96 && &d[..4] == b"PLM\x1A" && d[5] == 0x10 && (1..=32).contains(&d[54]) && usize::from(d[4]) >= 96
}

pub fn load_plm(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    r.skip(4)?;
    let hsize = usize::from(r.u8()?);
    r.skip(1)?;
    let title = r.text(48)?;
    let nch = usize::from(r.u8()?);
    r.skip(2)?;
    let _amplify = r.u8()?;
    let tempo = r.u8()?;
    let speed = r.u8()?;
    let pans = r.bytes(32)?.to_vec();
    let nsmp = usize::from(r.u8()?);
    let npat = usize::from(r.u8()?);
    let nord = usize::from(r.u16le()?);
    r.seek(hsize)?;
    let mut items = Vec::with_capacity(nord);
    for _ in 0..nord {
        items.push((usize::from(r.u16le()?), usize::from(r.u8()?), usize::from(r.u8()?)));
    }
    let mut pat_at = Vec::with_capacity(npat);
    for _ in 0..npat {
        pat_at.push(r.u32le()? as usize);
    }
    let mut smp_at = Vec::with_capacity(nsmp);
    for _ in 0..nsmp {
        smp_at.push(r.u32le()? as usize);
    }

    let mut samples = Vec::with_capacity(nsmp);
    for &at in &smp_at {
        let mut s = Sample::empty();
        let mut sr = Reader::at(d, at);
        if at == 0 || sr.tag().ok().as_ref() != Some(b"PLS\x1A") {
            samples.push(s);
            continue;
        }
        let shsize = usize::from(sr.u8()?);
        sr.skip(1)?;
        s.name = sr.text(32)?;
        s.filename = sr.text(12)?;
        let pan = sr.u8()?;
        let vol = sr.u8()?;
        let flags = sr.u8()?;
        s.c5 = u32::from(sr.u16le()?).max(1);
        sr.skip(4)?;
        let (mut ls, mut le, mut len) = (sr.u32le()?, sr.u32le()?, sr.u32le()? as usize);
        let b16 = flags & 1 != 0;
        if b16 {
            (ls, le, len) = (ls / 2, le / 2, len / 2);
        }
        if pan <= 15 {
            s.pan = Some(pan * 0x11);
        }
        s.global_volume = vol.min(64);
        let bytes = d.get(at + shsize..).unwrap_or(&[]);
        s.wave = Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, decode(bytes, if b16 { Pcm::U16le } else { Pcm::U8 }, len)));
        if le > ls {
            s.looped = Loop { kind: if flags & 2 != 0 { LoopKind::PingPong } else { LoopKind::Forward }, start: ls, end: le }.clamped(s.frames() as u32);
        }
        samples.push(s);
    }

    // Lay the blocks along the song, 64 rows a pattern.
    let width = nch + 1;
    let mut grid: Vec<Pattern> = Vec::new();
    let mut most = 0usize;
    for &(x, y, block) in &items {
        if block >= npat || y > nch {
            continue;
        }
        let mut pr = Reader::at(d, pat_at[block]);
        let Ok(_size) = pr.u32le() else { continue };
        let rows = usize::from(pr.u8()?);
        let bch = usize::from(pr.u8()?);
        pr.skip(1 + 25)?;
        if rows == 0 {
            continue;
        }
        let used = bch.min(nch - y);
        let end = x + rows;
        most = most.max(end);
        let mut last_note = [0u8; 32];
        for r in 0..rows {
            let (pat, row) = ((x + r) / 64, (x + r) % 64);
            while grid.len() <= pat {
                grid.push(Pattern::new(64, width));
            }
            for c in 0..bch {
                let b = pr.bytes(5)?;
                if c >= used {
                    continue;
                }
                let mut cell = Cell::default();
                if b[0] > 0 && b[0] < 0x90 {
                    cell.note = ((b[0] >> 4) * 12 + (b[0] & 0x0F) + 13).min(NOTE_MAX);
                    last_note[c] = cell.note;
                }
                cell.instr = b[1];
                if b[2] != 0xFF {
                    (cell.vol, cell.volp) = (Vol::Volume, b[2].min(64));
                }
                let p = b[4];
                (cell.fx, cell.param) = match b[3] {
                    0x01 => (Fx::PortaUp, p),
                    0x02 => (Fx::PortaDown, p),
                    0x03 => (Fx::TonePorta, p),
                    0x04 | 0x14 | 0x15 => {
                        // Both nibbles set: a fine slide up.
                        let p = if p & 0xF0 != 0 && p & 0x0F != 0 && p & 0xF0 != 0xF0 { p | 0x0F } else { p };
                        (match b[3] {
                            0x04 => Fx::VolSlide,
                            0x14 => Fx::VibratoVol,
                            _ => Fx::TonePortaVol,
                        }, p)
                    }
                    0x05 => (Fx::Tremolo, p),
                    0x06 => (Fx::Vibrato, p),
                    0x07 => (Fx::S3mEx, 0x40 | (p & 3)),
                    0x08 => (Fx::S3mEx, 0x30 | (p & 3)),
                    0x09 => (Fx::Tempo, p),
                    0x0A => (Fx::Speed, p),
                    0x0B | 0x0C => {
                        // Jump to an order item's place in the song, or to the end of this one.
                        let target = if b[3] == 0x0B { items.get(usize::from(p)).map(|i| i.0) } else { Some(end) };
                        match target {
                            Some(t) => {
                                let brk = &mut grid[pat].cells[row * width + nch];
                                (brk.fx, brk.param) = (Fx::PatternBreak, (t % 64) as u8);
                                (Fx::PositionJump, (t / 64).min(255) as u8)
                            }
                            None => (Fx::None, 0),
                        }
                    }
                    0x0D | 0x16 => {
                        // An offset with no note plays the last one again.
                        if cell.note == 0 {
                            cell.note = last_note[c];
                        }
                        if b[3] == 0x0D {
                            (Fx::Offset, p)
                        } else {
                            // A percentage of the sample.
                            let len = samples.get(usize::from(b[1]).wrapping_sub(1)).map_or(0, |s| s.frames());
                            if len > 0 { (Fx::Offset, ((len * usize::from(p.min(100)) / 100) >> 8).min(255) as u8) } else { (Fx::None, 0) }
                        }
                    }
                    0x0E => (Fx::S3mEx, 0x80 | (p & 0x0F)),
                    0x0F => (Fx::Retrig, p),
                    0x10 => (Fx::S3mEx, 0xD0 | p.min(15)),
                    0x11 => (Fx::S3mEx, 0xC0 | p.min(15)),
                    0x12 => (Fx::S3mEx, 0xE0 | p.min(15)),
                    0x13 => (Fx::FineVibrato, p),
                    _ => (Fx::None, 0),
                };
                grid[pat].cells[row * width + y + c] = cell;
            }
        }
    }
    // The song ends with its last block's last row.
    if !most.is_multiple_of(64) {
        if let Some(p) = grid.get_mut(most / 64) {
            *p = p.with_rows(most % 64);
        }
    }
    let orders = (0..grid.len() as u16).collect();
    let mut channels: Vec<Channel> = pans[..nch].iter().map(|&p| Channel { pan: (p & 15) * 0x11, ..Default::default() }).collect();
    channels.push(Channel { name: "Jumps".into(), ..Default::default() });
    let mut m = Module {
        title,
        format: "plm".into(),
        format_name: "Disorder Tracker 2".into(),
        tracker: "Disorder Tracker 2".into(),
        kind: Kind::S3m,
        flags: Flags { old_effects: true, ..Default::default() },
        speed,
        tempo: u16::from(tempo),
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels,
        orders,
        patterns: grid.into_iter().map(Arc::new).collect(),
        samples,
        ..Default::default()
    };
    m.message = m.samples.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join("\n").trim_end().to_string();
    m.tidy();
    Ok(m)
}

// ---- Real Tracker ----
//
// Objects, each a 42-byte header (a tag, a name, a version, its size):
// RTMM the song, RTND a pattern (packed as XM's, two effects a cell),
// RTIN an instrument (XM's keymap and envelopes), RTSM a sample.

/// An object's header fields: as many bytes as it says it has, the
/// rest zero (an empty instrument's are none at all).
fn rtm_fields(r: &mut Reader, size: usize, want: usize) -> Result<Vec<u8>> {
    let mut b = r.bytes(size)?.to_vec();
    b.resize(b.len().max(want), 0);
    Ok(b)
}

fn rtm_object(r: &mut Reader) -> Result<([u8; 4], String, u16, usize)> {
    let id = r.tag()?;
    r.skip(1)?;
    let name = r.text(32)?;
    r.skip(1)?;
    let version = r.u16le()?;
    let size = usize::from(r.u16le()?);
    Ok((id, name, version, size))
}

pub fn probe_rtm(d: &[u8]) -> bool {
    if d.len() < 42 + 98 || &d[..4] != b"RTMM" || d[4] != 0x20 || d[37] != 0x1A {
        return false;
    }
    let version = u16::from_le_bytes([d[38], d[39]]);
    let nch = d[42 + 20 + 32 + 2];
    (0x100..=0x112).contains(&version) && u16::from_le_bytes([d[40], d[41]]) >= 98 && (1..=32).contains(&nch)
}

fn rtm_envelope(b: &[u8], pan: bool) -> Envelope {
    let count = usize::from(b[0]).min(12);
    let points: Vec<EnvPoint> = (0..count)
        .map(|i| {
            let p = &b[1 + i * 8..9 + i * 8];
            let x = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            let y = i32::from_le_bytes([p[4], p[5], p[6], p[7]]);
            EnvPoint { tick: x.min(u32::from(u16::MAX)) as u16, value: if pan { y.clamp(-32, 32) } else { y.clamp(0, 64) } as i8 }
        })
        .collect();
    let (sus, ls, le) = (b[97], b[98], b[99]);
    let flags = u16::from_le_bytes([b[100], b[101]]);
    let n = points.len() as u8;
    Envelope {
        enabled: flags & 1 != 0 && n > 0,
        sustain: (flags & 2 != 0 && sus < n).then_some((sus, sus)),
        looped: (flags & 4 != 0 && ls < n && le < n && ls <= le).then_some((ls, le)),
        points,
        ..Default::default()
    }
}

/// A Real Tracker effect: XM's, and some of its own past X.
fn rtm_effect(cmd: u8, param: u8) -> (Fx, u8) {
    match cmd {
        8 => (Fx::Panning, param.saturating_mul(2)),
        0..=33 => mod_effect(cmd, param, true),
        36 => (Fx::VolSlide, param),
        37 => (Fx::PortaUp, param),
        38 => (Fx::PortaDown, param),
        39 => (Fx::VibratoVol, param),
        40 if param > 0 => (Fx::Speed, param),
        _ => (Fx::None, 0),
    }
}

pub fn load_rtm(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let (_, title, version, size) = rtm_object(&mut r)?;
    let start = r.pos;
    let software = r.text(20)?;
    let artist = r.text(32)?;
    let flags = r.u16le()?;
    let nch = usize::from(r.u8()?);
    let nins = usize::from(r.u8()?);
    let nord = usize::from(r.u16le()?);
    let npat = usize::from(r.u16le()?);
    let speed = r.u8()?;
    let tempo = r.u8()?;
    let pans = r.bytes(32)?.to_vec();
    let extra = r.u32le()? as usize;
    let original = if size >= 130 { r.text(32)? } else { String::new() };
    if nch == 0 || nch > 32 || nord > 999 || npat > 999 || speed == 0 {
        return bad("its song header isn't a Real Tracker one");
    }
    r.seek(start + size)?;
    let mut er = Reader::new(r.bytes(extra)?);
    let mut orders = Vec::with_capacity(nord);
    for _ in 0..nord {
        orders.push(er.u16le()?);
    }
    let mut channels: Vec<Channel> = pans[..nch].iter().map(|&p| Channel { pan: ((i16::from(p as i8) + 64) * 2).clamp(0, 255) as u8, ..Default::default() }).collect();
    if flags & 2 != 0 {
        for c in channels.iter_mut() {
            c.name = er.text(16).unwrap_or_default();
        }
    }

    let mut patterns = Vec::with_capacity(npat);
    for _ in 0..npat {
        let (_, name, _, size) = rtm_object(&mut r)?;
        let head = rtm_fields(&mut r, size, 9)?;
        let rows = usize::from(u16::from_le_bytes([head[3], head[4]])).clamp(1, 1024);
        let packed = u32::from_le_bytes([head[5], head[6], head[7], head[8]]) as usize;
        let mut pr = Reader::new(r.bytes_upto(packed));
        let mut p = Pattern::new(rows, nch);
        p.name = name;
        let (mut row, mut c) = (0usize, 0usize);
        while row < rows {
            let Ok(b) = pr.u8() else { break };
            if b == 0 {
                (row, c) = (row + 1, 0);
                continue;
            }
            if b & 1 != 0 {
                c = usize::from(pr.u8()?);
            }
            if c >= nch {
                return bad("a pattern names a channel past the song's");
            }
            let cell = &mut p.cells[row * nch + c];
            if b & 2 != 0 {
                cell.note = match pr.u8()? {
                    0xFE => NOTE_OFF,
                    n if n < 120 => (n + 13).min(NOTE_MAX),
                    _ => 0,
                };
            }
            if b & 4 != 0 {
                cell.instr = pr.u8()?;
            }
            let mut get = |bit: u8| if b & bit != 0 { pr.u8() } else { Ok(0) };
            let (c1, p1, c2, p2) = (get(0x08)?, get(0x10)?, get(0x20)?, get(0x40)?);
            if c1 != 0 || p1 != 0 {
                (cell.fx, cell.param) = rtm_effect(c1, p1);
            }
            if c2 != 0 || p2 != 0 {
                let (fx, param) = rtm_effect(c2, p2);
                if fx != Fx::None {
                    second_effect(cell, fx, param);
                }
            }
            c += 1;
        }
        patterns.push(Arc::new(p));
    }

    let mut instruments = Vec::with_capacity(nins);
    let mut samples: Vec<Sample> = Vec::new();
    for _ in 0..nins {
        let (_, name, _, size) = rtm_object(&mut r)?;
        let head = rtm_fields(&mut r, size, 341)?;
        let nsmp = usize::from(head[0]);
        let iflags = u16::from_le_bytes([head[1], head[2]]);
        let keymap = &head[3..123];
        let vib = AutoVibrato { kind: head[327], sweep: head[328], depth: head[329].saturating_mul(2), speed: head[330] / 2 };
        let fadeout = u32::from(u16::from_le_bytes([head[331], head[332]]));
        let first = samples.len();
        for _ in 0..nsmp {
            let (_, sname, _, size) = rtm_object(&mut r)?;
            let sh = rtm_fields(&mut r, size, 26)?;
            let le32 = |i: usize| u32::from_le_bytes([sh[i], sh[i + 1], sh[i + 2], sh[i + 3]]);
            let sflags = u16::from_le_bytes([sh[0], sh[1]]);
            let (global, vol, len, ltype) = (sh[2], sh[3], le32(4) as usize, sh[8]);
            let (ls, le, rate, base, pan) = (le32(12), le32(16), le32(20), sh[24], sh[25] as i8);
            let b16 = sflags & 2 != 0;
            let delta = sflags & 4 != 0;
            let fmt = match (b16, delta) {
                (true, true) => Pcm::D16le,
                (true, false) => Pcm::S16le,
                (false, true) => Pcm::D8,
                (false, false) => Pcm::S8,
            };
            let div = if b16 { 2 } else { 1 };
            let pcm = decode(r.bytes_upto(len), fmt, len / div as usize);
            let frames = pcm.len() as u32;
            // The rate is for the sample's base note; 48 is C-5 here.
            let c5 = (f64::from(rate.max(1)) * 2f64.powf((48.0 - f64::from(base)) / 12.0)).round().clamp(1.0, 2_000_000.0) as u32;
            samples.push(Sample {
                name: sname,
                wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
                c5,
                volume: vol.min(64),
                global_volume: global.min(64),
                pan: (iflags & 1 != 0).then(|| ((i16::from(pan) + 64) * 2).clamp(0, 255) as u8),
                looped: match ltype {
                    0 => Loop::default(),
                    2 => Loop { kind: LoopKind::PingPong, start: ls / div, end: le / div },
                    _ => Loop::forward(ls / div, le / div),
                }
                .clamped(frames),
                vibrato: vib,
                ..Sample::empty()
            });
        }
        instruments.push(Instrument {
            name,
            keymap: (0..120).map(|n| (n as u8, if usize::from(keymap[n]) < nsmp { (first + usize::from(keymap[n]) + 1) as u16 } else { 0 })).collect(),
            volume_env: rtm_envelope(&head[123..225], false),
            pan_env: rtm_envelope(&head[225..327], true),
            fadeout,
            global_volume: if iflags & 2 != 0 { 0 } else { 128 },
            ..Default::default()
        });
    }

    let mut m = Module {
        title: if title.is_empty() && version >= 0x112 { original.clone() } else { title },
        artist,
        message: if version >= 0x112 && !original.is_empty() { original } else { String::new() },
        format: "rtm".into(),
        format_name: format!("Real Tracker {}.{:02x}", version >> 8, version & 0xFF),
        tracker: if software.is_empty() { "Real Tracker".into() } else { software },
        kind: Kind::Xm,
        flags: Flags { linear: flags & 1 != 0, instruments: true, ..Default::default() },
        speed,
        tempo: u16::from(tempo),
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels,
        orders,
        patterns,
        samples,
        instruments,
        ..Default::default()
    };
    if m.message == m.title {
        m.message.clear();
    }
    m.tidy();
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xmf_notes_and_volumes() {
        let mut d = vec![0u8; 1 + 256 * 16 + 256 + 2 + 1 + 64 * 6 + 4];
        d[0] = 4;
        d[10] = 4;
        d[13] = 255;
        d[15..17].copy_from_slice(&8363u16.to_le_bytes());
        let ord = 1 + 256 * 16;
        d[ord + 1..ord + 256].fill(0xFF);
        let pat = ord + 256 + 2 + 1;
        // Note 25 is middle C; volume 0x80 of 255.
        d[pat..pat + 6].copy_from_slice(&[25, 1, 0x0C, 0, 0, 0x80]);
        assert!(probe_xmf(&d));
        let m = load_xmf(&d).unwrap();
        let c = m.patterns[0].cell(0, 0);
        assert_eq!((c.note, c.instr, c.fx, c.param), (61, 1, Fx::Volume, 32));
        assert_eq!((m.samples[0].volume, m.samples[0].frames(), m.samples[0].c5), (64, 4, 8363));
    }
}
