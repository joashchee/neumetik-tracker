//! UltraTracker (.ult), Farandole Composer (.far), Imago Orpheus (.imf),
//! and Unreal's packages (.umx, which hold an IT, S3M, XM or MOD).

use std::sync::Arc;

use crate::io::{bad, decode, Pcm, Reader, Result};
use crate::model::*;

// ---- UltraTracker ----

pub fn probe_ult(d: &[u8]) -> bool {
    d.len() >= 48 && d.starts_with(b"MAS_UTrack_V00") && (b'1'..=b'4').contains(&d[14])
}

fn ult_effect(cmd: u8, p: u8) -> (Fx, u8, Option<(Vol, u8)>) {
    match cmd {
        0 => (if p != 0 { Fx::Arpeggio } else { Fx::None }, p, None),
        1 => (Fx::PortaUp, p, None),
        2 => (Fx::PortaDown, p, None),
        3 => (Fx::TonePorta, p, None),
        4 => (Fx::Vibrato, p, None),
        7 => (Fx::Tremolo, p, None),
        9 => (Fx::Offset, p.saturating_mul(4), None),
        0xA => (Fx::VolSlide, p, None),
        0xB => (Fx::None, 0, Some((Vol::Panning, ((p & 15) as u16 * 64 / 15) as u8))),
        0xC => (Fx::None, 0, Some((Vol::Volume, p / 4))),
        0xD => (Fx::PatternBreak, (p >> 4) * 10 + (p & 15), None),
        0xE => match p >> 4 {
            1 => (Fx::ModEx, 0x10 | (p & 15), None),
            2 => (Fx::ModEx, 0x20 | (p & 15), None),
            8 => (Fx::ModEx, 0xD0 | (p & 15), None),
            9 => (Fx::ModEx, 0x90 | (p & 15), None),
            0xA => (Fx::ModEx, 0xA0 | (p & 15), None),
            0xB => (Fx::ModEx, 0xB0 | (p & 15), None),
            0xC => (Fx::ModEx, 0xC0 | (p & 15), None),
            _ => (Fx::None, 0, None),
        },
        0xF if p == 0 => (Fx::None, 0, None),
        0xF if p < 0x30 => (Fx::Speed, p, None),
        0xF => (Fx::Tempo, p, None),
        _ => (Fx::None, 0, None),
    }
}

pub fn load_ult(d: &[u8]) -> Result<Module> {
    let version = d[14] - b'0';
    let mut r = Reader::at(d, 15);
    let title = r.text(32)?;
    let lines = usize::from(r.u8()?);
    let msg = r.bytes(lines * 32)?;
    let nsmp = usize::from(r.u8()?);
    let mut heads = Vec::new();
    for _ in 0..nsmp {
        let name = r.text(32)?;
        let filename = r.text(12)?;
        let ls = r.u32le()?;
        let le = r.u32le()?;
        let ss = r.u32le()?;
        let se = r.u32le()?;
        let vol = r.u8()?;
        let flags = r.u8()?;
        let c2 = if version >= 4 { r.u16le()? } else { 8363 };
        let ft = r.i16le()?;
        heads.push((name, filename, ls, le, se.saturating_sub(ss) as usize, vol, flags, c2, ft));
    }
    let ord = r.bytes(256)?.to_vec();
    let nch = usize::from(r.u8()?) + 1;
    let npat = usize::from(r.u8()?) + 1;
    let pans: Vec<u8> = if version >= 3 { r.bytes(nch)?.iter().map(|&p| (p & 15) * 17).collect() } else { (0..nch).map(super::common::amiga_pan).collect() };
    let mut patterns: Vec<Pattern> = (0..npat).map(|_| Pattern::new(64, nch)).collect();
    for ch in 0..nch {
        for pat in patterns.iter_mut() {
            let mut row = 0;
            while row < 64 {
                let first = r.u8()?;
                let (repeat, note) = if first == 0xFC { (usize::from(r.u8()?), r.u8()?) } else { (1, first) };
                let instr = r.u8()?;
                let cmd = r.u8()?;
                let p1 = r.u8()?;
                let p2 = r.u8()?;
                let mut cell = Cell { note: if note > 0 && note < 0x61 { (note + 24).min(NOTE_MAX) } else { 0 }, instr, ..Default::default() };
                for (c, p) in [(cmd & 15, p1), (cmd >> 4, p2)] {
                    let (fx, param, vol) = ult_effect(c, p);
                    if let Some((v, vp)) = vol {
                        if cell.vol == Vol::None {
                            (cell.vol, cell.volp) = (v, vp);
                        }
                    }
                    if fx != Fx::None && cell.fx == Fx::None {
                        (cell.fx, cell.param) = (fx, param);
                    }
                }
                for _ in 0..repeat.max(1) {
                    if row < 64 {
                        pat.cells[row * nch + ch] = cell;
                    }
                    row += 1;
                }
            }
        }
    }
    let samples = heads
        .into_iter()
        .map(|(name, filename, ls, le, len, vol, flags, c2, ft)| {
            let b16 = flags & 4 != 0;
            let pcm = decode(r.bytes_upto(len * if b16 { 2 } else { 1 }), if b16 { Pcm::S16le } else { Pcm::S8 }, len);
            let kind = if flags & 8 == 0 {
                LoopKind::Off
            } else if flags & 16 != 0 {
                LoopKind::PingPong
            } else {
                LoopKind::Forward
            };
            Sample {
                name,
                filename,
                wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
                c5: (f64::from(c2.max(1000)) * 2f64.powf(f64::from(ft) / (12.0 * 32768.0))) as u32,
                volume: vol / 4,
                looped: Loop { kind, start: ls, end: le },
                ..Sample::empty()
            }
        })
        .collect();
    let mut m = Module {
        title,
        message: msg.chunks(32).map(crate::io::text).collect::<Vec<_>>().join("\n").trim_end().to_string(),
        format: "ult".into(),
        format_name: "UltraTracker".into(),
        tracker: format!("UltraTracker (format {version})"),
        kind: Kind::Mod,
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 40,
        separation: 128,
        channels: pans.into_iter().map(|pan| Channel { pan, ..Default::default() }).collect(),
        orders: ord.iter().take_while(|&&o| o != 255).map(|&o| u16::from(o)).collect(),
        patterns: patterns.into_iter().map(Arc::new).collect(),
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Farandole Composer ----

pub fn probe_far(d: &[u8]) -> bool {
    d.len() >= 98 && &d[..4] == b"FAR\xFE" && &d[44..47] == b"\r\n\x1A"
}

pub fn load_far(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 4);
    let title = r.text(40)?;
    r.seek(47)?;
    let header = usize::from(r.u16le()?);
    r.seek(0x4B)?;
    let tempo = r.u8()?;
    let pans: Vec<u8> = r.bytes(16)?.iter().map(|&p| (p & 15) * 17).collect();
    r.seek(0x60)?;
    let text_len = usize::from(r.u16le()?);
    let text = r.bytes(text_len)?;
    let ord = r.bytes(256)?.to_vec();
    let _npat = r.u8()?;
    let songlen = usize::from(r.u8()?);
    let restart = r.u8()?;
    let mut plens = Vec::with_capacity(256);
    for _ in 0..256 {
        plens.push(usize::from(r.u16le()?));
    }
    r.seek(header)?;
    let mut patterns = Vec::new();
    for &len in &plens {
        if len < 2 {
            patterns.push(Arc::new(Pattern::new(64, 16)));
            continue;
        }
        let body = r.bytes_upto(len);
        let rows = ((len - 2) / 64).clamp(1, 256);
        let brk = usize::from(body.first().copied().unwrap_or(0));
        let mut p = Pattern::new(rows, 16);
        for row in 0..rows {
            for c in 0..16 {
                let at = 2 + (row * 16 + c) * 4;
                let Some(b) = body.get(at..at + 4) else { break };
                let mut cell = Cell::default();
                if b[0] > 0 {
                    cell.note = b[0].saturating_add(48).min(NOTE_MAX);
                    cell.instr = b[1].saturating_add(1);
                }
                if b[2] > 0 {
                    (cell.vol, cell.volp) = (Vol::Volume, ((u16::from(b[2] & 15)) * 64 / 15) as u8);
                }
                let (cmd, v) = (b[3] >> 4, b[3] & 15);
                (cell.fx, cell.param) = match cmd {
                    1 => (Fx::PortaUp, v),
                    2 => (Fx::PortaDown, v),
                    3 => (Fx::TonePorta, v.max(1)),
                    4 => (Fx::Retrig, v),
                    5 => (Fx::Vibrato, v),
                    6 => (Fx::Vibrato, v << 4),
                    7 => (Fx::VolSlide, v << 4),
                    8 => (Fx::VolSlide, v),
                    0xB => (Fx::Panning, v * 17),
                    0xF => (Fx::Speed, v.max(1)),
                    _ => (Fx::None, 0),
                };
                p.cells[row * 16 + c] = cell;
            }
        }
        // The pattern ends after its break row.
        if brk > 0 && brk + 1 < rows {
            let c = &mut p.cells[(brk + 1) * 16 + 15];
            if c.fx == Fx::None {
                (c.fx, c.param) = (Fx::PatternBreak, 0);
            }
        }
        patterns.push(Arc::new(p));
    }
    let map = r.bytes(8).map(|b| b.to_vec()).unwrap_or_else(|_| vec![0; 8]);
    let mut samples = Vec::new();
    for i in 0..64 {
        if map[i / 8] & (1 << (i % 8)) == 0 {
            samples.push(Sample::empty());
            continue;
        }
        // A file cut short keeps the samples it has.
        let Ok(h) = r.bytes(48) else { break };
        let mut h = Reader::new(h);
        let name = h.text(32)?;
        let len = h.u32le()? as usize;
        let _ft = h.u8()?;
        let vol = h.u8()?;
        let ls = h.u32le()?;
        let le = h.u32le()?;
        let kind = h.u8()?;
        let lp = h.u8()?;
        let b16 = kind & 1 != 0;
        let pcm = decode(r.bytes_upto(len), if b16 { Pcm::S16le } else { Pcm::S8 }, len / if b16 { 2 } else { 1 });
        let div = if b16 { 2 } else { 1 };
        samples.push(Sample {
            name,
            wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
            volume: (u16::from(vol & 15) * 64 / 15) as u8,
            looped: if lp & 8 != 0 && le > ls { Loop::forward(ls / div, le / div) } else { Loop::default() },
            ..Sample::empty()
        });
    }
    let used = ord[..songlen.min(256)].iter().copied().max().map_or(0, |m| usize::from(m) + 1);
    patterns.truncate(used.max(1));
    while samples.last().is_some_and(|s| s.wave.is_empty() && s.name.is_empty()) {
        samples.pop();
    }
    let mut m = Module {
        title,
        message: crate::io::message(text),
        format: "far".into(),
        format_name: "Farandole Composer".into(),
        tracker: "Farandole Composer".into(),
        kind: Kind::Mod,
        speed: if tempo == 0 { 4 } else { tempo },
        tempo: 80,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        restart: u16::from(restart),
        channels: pans.into_iter().map(|pan| Channel { pan, ..Default::default() }).collect(),
        orders: ord[..songlen.min(256)].iter().map(|&o| u16::from(o)).collect(),
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Imago Orpheus ----

pub fn probe_imf(d: &[u8]) -> bool {
    d.len() >= 0x240 && &d[0x3C..0x40] == b"IM10"
}

fn imf_effect(e: u8, p: u8) -> (Fx, u8) {
    match e {
        0x01 => (Fx::Speed, p),
        0x02 => (Fx::Tempo, p),
        0x03 => (Fx::TonePorta, p),
        0x04 => (Fx::TonePortaVol, p),
        0x05 => (Fx::Vibrato, p),
        0x06 => (Fx::VibratoVol, p),
        0x07 => (Fx::FineVibrato, p),
        0x08 => (Fx::Tremolo, p),
        0x09 => (Fx::Arpeggio, p),
        0x0A => (Fx::Panning, p),
        0x0B => (Fx::PanningSlide, p),
        0x0C => (Fx::Volume, p.min(64)),
        0x0D => (Fx::VolSlide, p),
        0x0E => (Fx::VolSlide, if p >> 4 != 0 { (p & 0xF0) | 0x0F } else { 0xF0 | (p & 15) }),
        0x0F => (Fx::SetFinetune, p),
        0x10 => (Fx::NoteSlideUp, p),
        0x11 => (Fx::NoteSlideDown, p),
        0x12 => (Fx::PortaUp, p),
        0x13 => (Fx::PortaDown, p),
        0x14 => (Fx::PortaUp, 0xF0 | (p >> 4).min(15)),
        0x15 => (Fx::PortaDown, 0xF0 | (p >> 4).min(15)),
        0x16 => (Fx::Midi, p >> 1),
        0x18 => (Fx::Offset, p),
        0x1A => (Fx::KeyOff, p),
        0x1B => (Fx::Retrig, p),
        0x1C => (Fx::Tremor, p),
        0x1D => (Fx::PositionJump, p),
        0x1E => (Fx::PatternBreak, p),
        0x1F => (Fx::GlobalVolume, p.min(128)),
        0x20 => (Fx::GlobalVolSlide, p),
        0x21 => (Fx::S3mEx, p),
        _ => (Fx::None, 0),
    }
}

fn imf_env(r: &mut Reader, pan: bool) -> Result<Envelope> {
    let mut pts = Vec::new();
    for _ in 0..16 {
        pts.push((r.u16le()?, r.u16le()?));
    }
    Ok(Envelope { points: pts.into_iter().map(|(t, v)| EnvPoint { tick: t, value: if pan { (v.min(64) as i16 - 32) as i8 } else { v.min(64) as i8 } }).collect(), ..Default::default() })
}

pub fn load_imf(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let title = r.text(32)?;
    let ordnum = usize::from(r.u16le()?);
    let patnum = usize::from(r.u16le()?);
    let insnum = usize::from(r.u16le()?);
    let flags = r.u16le()?;
    r.skip(8)?;
    let speed = r.u8()?;
    let bpm = r.u8()?;
    let mv = r.u8()?;
    let _amp = r.u8()?;
    r.seek(0x40)?;
    let mut chans = Vec::new();
    let mut nch = 0;
    for i in 0..32 {
        let name = r.text(12)?;
        r.skip(2)?;
        let pan = r.u8()?;
        let status = r.u8()?;
        if status < 2 {
            nch = i + 1;
        }
        chans.push(Channel { name, pan, muted: status == 1, ..Default::default() });
    }
    chans.truncate(nch.max(1));
    let orders: Vec<u16> = r.bytes(256)?[..ordnum.min(256)].iter().map(|&o| if o == 0xFF { ORDER_SKIP } else { u16::from(o) }).collect();
    let mut patterns = Vec::new();
    for _ in 0..patnum {
        let len = usize::from(r.u16le()?);
        let rows = usize::from(r.u16le()?).clamp(1, 256);
        let body = r.bytes(len.saturating_sub(4))?;
        let mut p = Reader::new(body);
        let mut pat = Pattern::new(rows, chans.len());
        let mut row = 0;
        while row < rows && !p.eof() {
            let mask = p.u8()?;
            if mask == 0 {
                row += 1;
                continue;
            }
            let ch = usize::from(mask & 31);
            let mut c = Cell::default();
            if mask & 32 != 0 {
                let n = p.u8()?;
                c.instr = p.u8()?;
                c.note = match n {
                    0xA0 => NOTE_OFF,
                    0xFF => 0,
                    n => ((n >> 4) * 12 + (n & 15) + 1 + 12).min(NOTE_MAX),
                };
            }
            let mut fx = Vec::new();
            if mask & 64 != 0 {
                fx.push((p.u8()?, p.u8()?));
            }
            if mask & 128 != 0 {
                fx.push((p.u8()?, p.u8()?));
            }
            for (e, v) in fx {
                let (f, v) = imf_effect(e, v);
                if f == Fx::Volume && c.vol == Vol::None {
                    (c.vol, c.volp) = (Vol::Volume, v);
                } else if f != Fx::None && c.fx == Fx::None {
                    (c.fx, c.param) = (f, v);
                }
            }
            if let Some(s) = pat.cell_mut(row, ch) {
                *s = c;
            }
        }
        patterns.push(Arc::new(pat));
    }
    let mut instruments = Vec::new();
    let mut samples: Vec<Sample> = Vec::new();
    for _ in 0..insnum {
        let start = r.pos;
        let name = r.text(32)?;
        let keymap = r.bytes(120)?.to_vec();
        r.skip(8)?;
        let mut envs = [imf_env(&mut r, false)?, imf_env(&mut r, true)?, imf_env(&mut r, true)?];
        for e in envs.iter_mut() {
            let (n, sus, ls, le, f) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?, r.u8()?);
            r.skip(3)?;
            e.points.truncate(usize::from(n));
            let n = e.points.len() as u8;
            e.enabled = f & 1 != 0 && n > 0;
            e.sustain = (f & 2 != 0 && sus < n).then_some((sus, sus));
            e.looped = (f & 4 != 0 && ls <= le && le < n).then_some((ls, le));
        }
        let fadeout = r.u16le()?;
        let nsmp = usize::from(r.u16le()?);
        r.seek(start + 384)?;
        let first = samples.len();
        for _ in 0..nsmp {
            let sstart = r.pos;
            let filename = r.text(13)?;
            r.skip(3)?;
            let len = r.u32le()? as usize;
            let ls = r.u32le()?;
            let le = r.u32le()?;
            let c5 = r.u32le()?;
            let vol = r.u8()?;
            let pan = r.u8()?;
            r.skip(14)?;
            let sf = r.u8()?;
            r.seek(sstart + 64)?;
            let b16 = sf & 4 != 0;
            let pcm = decode(r.bytes_upto(len), if b16 { Pcm::S16le } else { Pcm::S8 }, len / if b16 { 2 } else { 1 });
            let div = if b16 { 2 } else { 1 };
            samples.push(Sample {
                name: filename.clone(),
                filename,
                wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
                c5: c5.clamp(1000, 500_000),
                volume: vol.min(64),
                pan: (sf & 8 != 0).then_some(pan),
                looped: if sf & 1 != 0 && le > ls { Loop { kind: if sf & 2 != 0 { LoopKind::PingPong } else { LoopKind::Forward }, start: ls / div, end: le / div } } else { Loop::default() },
                ..Sample::empty()
            });
        }
        let [vol_env, pan_env, filter_env] = envs;
        instruments.push(Instrument {
            name,
            keymap: (0..120).map(|n| (n as u8, if n < 12 || nsmp == 0 { 0 } else { keymap.get(n - 12).map_or(0, |&s| if usize::from(s) < nsmp { (first + usize::from(s) + 1) as u16 } else { 0 }) })).collect(),
            volume_env: vol_env,
            pan_env,
            pitch_env: Envelope { filter: true, ..filter_env },
            fadeout: u32::from(fadeout) * 64,
            global_volume: 128,
            ..Default::default()
        });
    }
    let mut m = Module {
        title,
        format: "imf".into(),
        format_name: "Imago Orpheus".into(),
        tracker: "Imago Orpheus".into(),
        kind: Kind::It,
        flags: Flags { linear: flags & 1 != 0, instruments: true, ..Default::default() },
        speed: speed.max(1),
        tempo: u16::from(bpm.max(32)),
        global_volume: 128,
        mix_volume: mv.clamp(16, 128),
        separation: 128,
        channels: chans,
        orders,
        patterns,
        samples,
        instruments,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Unreal packages ----

pub fn probe_umx(d: &[u8]) -> bool {
    d.len() >= 64 && d[..4] == [0xC1, 0x83, 0x2A, 0x9E]
}

/// The module inside an Unreal package: found by its own mark.
pub fn umx_inner(d: &[u8]) -> Option<(usize, &'static str)> {
    for i in 0..d.len().saturating_sub(64) {
        if &d[i..i + 4] == b"IMPM" {
            return Some((i, "it"));
        }
        if d[i..].starts_with(b"Extended Module: ") {
            return Some((i, "xm"));
        }
        if i >= 44 && &d[i..i + 4] == b"SCRM" && d.get(i - 44 + 29) == Some(&16) {
            return Some((i - 44, "s3m"));
        }
        if i >= 1080 && (&d[i..i + 4] == b"M.K." || &d[i..i + 4] == b"M!K!") {
            return Some((i - 1080, "mod"));
        }
    }
    None
}

pub fn load_umx(d: &[u8]) -> Result<Module> {
    let Some((at, kind)) = umx_inner(d) else { return bad("the package has no music in it") };
    let inner = &d[at..];
    let mut m = match kind {
        "it" => super::it::load(inner)?,
        "xm" => super::xm::load(inner)?,
        "s3m" => super::s3m::load(inner)?,
        _ => super::protracker::load(inner, false)?,
    };
    m.format_name = format!("{} (in an Unreal package)", m.format_name);
    Ok(m)
}
