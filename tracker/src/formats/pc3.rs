//! A fourth lot of the PC's trackers: Cubic Player's tiny XM (.mxm),
//! MikMod's UNITRK (.uni), Extreme's Tracker and Velvet Studio (.ams),
//! Digitrakker (.mdl) and X-Tracker (.dmf).

use std::sync::Arc;

use super::common::{mod_effect, relative_c5};
use crate::io::{bad, decode, Pcm, Reader, Result};
use crate::model::*;

fn le32(d: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]])
}

// ---- Cubic Tiny XM ----
//
// An XM made small for Cubic Player's demos: "MXM\0", counts as 32-bit
// numbers, the channels' panning, 256 orders, where each instrument and
// pattern starts; instruments of 256 bytes (a keymap, envelopes as
// segments of so many ticks), each with 16-byte sample headers; patterns
// packed by channel; the 8-bit samples, then the 16-bit ones, laid end to
// end as they were in the Gravis Ultrasound's memory. The cells are
// Cubic Player's own copy of XM's, its E commands made 36 + x.

const MXM_HEADER: usize = 48 + 32 + 256 + 128 * 4 + 256 * 4;

pub fn probe_mxm(d: &[u8]) -> bool {
    if d.len() < MXM_HEADER || &d[..4] != b"MXM\0" {
        return false;
    }
    let (nord, restart, nch, npat, nins) = (le32(d, 4), le32(d, 8), le32(d, 12), le32(d, 16), le32(d, 20));
    (1..=256).contains(&nord) && restart < nord.max(1) && (1..=32).contains(&nch) && npat <= 256 && nins <= 128
}

/// A tiny XM envelope: up to 12 segments of (ticks to the next point, a
/// value 0 to 64), `count` of them, and the points sustained and looped
/// (0xFF for none). It always plays: a single point holds its value.
fn mxm_envelope(segs: &[(u16, u16)], count: u8, sustain: u8, ls: u8, le: u8, pan: bool) -> Envelope {
    let n = usize::from(count).min(segs.len() - 1);
    let mut tick = 0u16;
    let points: Vec<EnvPoint> = (0..=n)
        .map(|i| {
            let p = EnvPoint { tick, value: if pan { (segs[i].1.min(64) as i16 - 32) as i8 } else { segs[i].1.min(64) as i8 } };
            tick = tick.saturating_add(segs[i].0);
            p
        })
        .collect();
    let last = n as u8;
    Envelope { enabled: true, sustain: (sustain <= last).then_some((sustain, sustain)), looped: (le <= last && ls <= le).then_some((ls, le)), points, ..Default::default() }
}

pub fn load_mxm(d: &[u8]) -> Result<Module> {
    if !probe_mxm(d) {
        return bad("its header isn't a tiny XM's");
    }
    let (nord, restart, nch, npat, nins) = (le32(d, 4) as usize, le32(d, 8) as u16, le32(d, 12) as usize, le32(d, 16) as usize, le32(d, 20) as usize);
    let (speed, tempo, opt) = (d[24], d[25], u16::from_le_bytes([d[26], d[27]]));
    let (start, bytes8, bytes16) = (le32(d, 28) as usize, le32(d, 32) as usize, le32(d, 36) as usize);
    let delta = opt & 4 != 0;
    let pans = &d[48..80];
    // Orders past the patterns play an empty one, the last.
    let orders: Vec<u16> = d[80..80 + nord].iter().map(|&o| u16::from(o).min(npat as u16)).collect();
    let ins_at = |i: usize| le32(d, 336 + i * 4) as usize;
    let pat_at = |i: usize| le32(d, 848 + i * 4) as usize;

    let mut patterns = Vec::with_capacity(npat + 1);
    for i in 0..npat {
        let mut r = Reader::at(d, 0);
        r.seek(pat_at(i))?;
        let rows = (r.u32le()? as usize).clamp(1, 1024);
        let mut p = Pattern::new(rows, nch);
        'rows: for row in 0..rows {
            loop {
                let Ok(pack) = r.u8() else { break 'rows };
                if pack == 0 {
                    break;
                }
                let ch = usize::from(pack & 0x1F);
                let mut cell = Cell::default();
                if pack & 0x20 != 0 {
                    let (n, ins) = (r.u8()?, r.u8()?);
                    cell.note = match n {
                        97 => NOTE_OFF,
                        1..=96 => n + 12,
                        _ => 0,
                    };
                    cell.instr = ins;
                }
                if pack & 0x40 != 0 {
                    (cell.vol, cell.volp) = super::xm::volume_column(r.u8()?);
                }
                if pack & 0x80 != 0 {
                    let (fx, param) = (r.u8()?, r.u8()?);
                    (cell.fx, cell.param) = match fx {
                        36..=51 => (Fx::ModEx, (fx - 36) << 4 | (param & 0x0F)),
                        _ => mod_effect(fx, param, true),
                    };
                }
                if let Some(c) = p.cell_mut(row, ch) {
                    *c = cell;
                }
            }
        }
        patterns.push(Arc::new(p));
    }
    patterns.push(Arc::new(Pattern::new(64, nch)));

    // Two layouts of sample header: Cubic Player's (a loop start and a
    // length, the samples laid end to end) and a later one of 24-bit
    // addresses in the card's memory (a start, a loop start, an end
    // that's in the sample), told apart by whether those addresses
    // follow on from each other.
    let mut heads: Vec<(usize, &[u8], usize)> = Vec::with_capacity(nins);
    for i in 0..nins {
        let at = ins_at(i);
        let h = d.get(at..at + 256).ok_or_else(|| crate::io::Error::Bad("an instrument is past the end".into()))?;
        let nsmp = (le32(h, 0) as usize).min(256);
        if d.len() < at + 256 + nsmp * 16 {
            return bad("an instrument's samples are past the end");
        }
        heads.push((at, h, nsmp));
    }
    let u24 = |s: &[u8], i: usize| u32::from(s[i]) | u32::from(s[i + 1]) << 8 | u32::from(s[i + 2]) << 16;
    let all: Vec<&[u8]> = heads.iter().flat_map(|&(at, _, n)| (0..n).map(move |k| &d[at + 256 + k * 16..at + 272 + k * 16])).collect();
    let mut next = 0u32;
    let addressed = !all.is_empty()
        && all.iter().all(|s| {
            let (from, ls, end) = (u24(s, 0), u24(s, 3), u24(s, 6));
            let ok = from == next && from <= end && (from..=end + 1).contains(&ls) && s[14] == 0 && s[15] == 0;
            next = end + 1;
            ok
        });

    let (mut at8, mut at16) = (start, start.saturating_add(bytes8));
    let (end8, end16) = (at16, at16.saturating_add(bytes16 * 2));
    let mut instruments = Vec::with_capacity(nins);
    let mut samples: Vec<Sample> = Vec::new();
    for (at, h, nsmp) in heads {
        let keymap = &h[4..100];
        let fade = u16::from_le_bytes([h[100], h[101]]);
        let vib = AutoVibrato {
            kind: match h[102] {
                1 => 4,
                2 => 2,
                3 => 1,
                _ => 0,
            },
            sweep: h[103],
            depth: h[104],
            speed: h[105],
        };
        let segs = |at: usize| -> Vec<(u16, u16)> { (0..12).map(|k| (u16::from_le_bytes([h[at + k * 4], h[at + k * 4 + 1]]), u16::from_le_bytes([h[at + k * 4 + 2], h[at + k * 4 + 3]]))).collect() };
        let volume_env = mxm_envelope(&segs(110), h[106], h[107], h[108], h[109], false);
        let pan_env = mxm_envelope(&segs(162), h[158], h[159], h[160], h[161], true);
        let first = samples.len();
        for k in 0..nsmp {
            let s = &d[at + 256 + k * 16..at + 272 + k * 16];
            let (frames, ls, at_mode) = if addressed {
                let from = u24(s, 0);
                (u24(s, 6) - from + 1, u24(s, 3) - from, 9)
            } else {
                (le32(s, 4), le32(s, 0), 8)
            };
            let (mode, vol, pan) = (s[at_mode], s[at_mode + 1], s[at_mode + 2]);
            let rel = i16::from_le_bytes([s[at_mode + 3], s[at_mode + 4]]);
            let b16 = mode & 4 != 0;
            let frames = frames as usize;
            let fmt = match (b16, delta) {
                (true, true) => Pcm::D16le,
                (true, false) => Pcm::S16le,
                (false, true) => Pcm::D8,
                (false, false) => Pcm::S8,
            };
            let (at, end) = if b16 { (&mut at16, end16) } else { (&mut at8, end8) };
            let bytes = d.get(*at..end.min(d.len())).unwrap_or(&[]);
            let pcm = decode(&bytes[..bytes.len().min(frames * fmt.bytes())], fmt, frames);
            *at = at.saturating_add(frames * fmt.bytes());
            let kind = if mode & 0x08 == 0 {
                LoopKind::Off
            } else if mode & 0x10 != 0 {
                LoopKind::PingPong
            } else {
                LoopKind::Forward
            };
            samples.push(Sample {
                wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
                c5: (8363.0 * 2f64.powf(f64::from(rel) / 256.0 / 12.0)).round().clamp(1.0, 2_000_000.0) as u32,
                volume: vol.min(64),
                pan: Some(pan),
                looped: Loop { kind, start: ls, end: frames as u32 },
                vibrato: vib,
                ..Sample::empty()
            });
        }
        instruments.push(Instrument {
            keymap: (0..120)
                .map(|n| {
                    let k = n - 12;
                    let s = if (0..96).contains(&k) { usize::from(keymap[k as usize]) } else { usize::MAX };
                    (n as u8, if s < nsmp { (first + s + 1) as u16 } else { 0 })
                })
                .collect(),
            volume_env,
            pan_env,
            fadeout: u32::from(fade) * 2,
            global_volume: 128,
            ..Default::default()
        });
    }

    let mut m = Module {
        format: "mxm".into(),
        format_name: "Cubic Tiny XM".into(),
        tracker: "Cubic Player's MXM converter".into(),
        kind: Kind::Xm,
        flags: Flags { linear: opt & 1 != 0, instruments: true, ..Default::default() },
        speed,
        tempo: u16::from(tempo),
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        restart,
        channels: pans[..nch].iter().map(|&p| Channel { pan: (p & 15) * 0x11, ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        instruments,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- MikMod UNITRK ----
//
// MikMod's own module, as its converter wrote it (UN04 and UN05): the
// counts, 256 orders, the channels' panning, the title, the format it
// was converted from and a comment; instruments with their samples' headers
// (XM's, the envelopes' values four times over); each pattern's rows
// and the track each of its channels plays; the tracks, rows of MikMod's
// commands (a note, an instrument, the effects of whichever tracker the
// song came from), each row's first byte its length and how many times
// it's played; the samples.

pub fn probe_uni(d: &[u8]) -> bool {
    d.len() >= 32 && &d[..3] == b"UN0" && matches!(d[3], b'4' | b'5') && (1..=32).contains(&d[4])
}

/// A MikMod string: a 16-bit length, then the bytes.
fn uni_string(r: &mut Reader) -> Result<String> {
    let n = usize::from(r.u16le()?);
    r.text(n)
}

/// One of MikMod's commands, numbered as MikMod 3 numbers them, read
/// into the cell. A volume goes to the volume column, the rest to the
/// effect, the first there kept.
fn uni_effect(cell: &mut Cell, op: u8, p: u8) {
    let (fx, param) = match op {
        3..=18 if op == 3 && p == 0 => return,
        // ProTracker's, with the break's row a number already.
        16 => (Fx::PatternBreak, p),
        3..=18 => mod_effect(op - 3, p, false),
        19 => (Fx::Speed, p),
        20 => (Fx::VolSlide, p),
        21 => (Fx::PortaDown, p),
        22 => (Fx::PortaUp, p),
        23 => (Fx::Tremor, p),
        24 => (Fx::Retrig, p),
        25 => (Fx::Tremolo, p),
        26 => (Fx::Tempo, p),
        27 => (Fx::FineVibrato, p),
        33 => (Fx::VolSlide, p),
        38 => (Fx::GlobalVolume, p.min(64) * 2),
        39 => (Fx::GlobalVolSlide, p),
        41 => (Fx::PanningSlide, p),
        _ => return,
    };
    if fx == Fx::None {
        return;
    }
    if fx == Fx::Volume && cell.vol == Vol::None {
        (cell.vol, cell.volp) = (Vol::Volume, param.min(64));
    } else if cell.fx == Fx::None {
        (cell.fx, cell.param) = (fx, param);
    } else if cell.fx == Fx::Volume && cell.vol == Vol::None {
        (cell.vol, cell.volp) = (Vol::Volume, cell.param.min(64));
        (cell.fx, cell.param) = (fx, param);
    }
}

/// A track's rows, `rows` of them.
fn uni_track(t: &[u8], rows: usize) -> Vec<Cell> {
    let mut out = Vec::with_capacity(rows);
    let mut at = 0;
    while out.len() < rows {
        let Some(&c) = t.get(at) else { break };
        if c == 0 {
            break;
        }
        let len = usize::from(c & 0x1F).max(1);
        let row = t.get(at + 1..(at + len).min(t.len())).unwrap_or(&[]);
        let mut cell = Cell::default();
        let mut i = 0;
        while i < row.len() {
            // The UN04 and UN05 numbers past 24, as MikMod 3 numbers them.
            let op = match row[i] {
                25 => 26,
                26 => 33,
                27 => 38,
                28 => 39,
                29 => 41,
                o => o,
            };
            let v = row.get(i + 1).copied().unwrap_or(0);
            match op {
                1 => cell.note = if v < 96 { v + 13 } else { NOTE_OFF },
                2 => cell.instr = v.saturating_add(1),
                _ => uni_effect(&mut cell, op, v),
            }
            i += 2;
        }
        for _ in 0..=c >> 5 {
            out.push(cell);
        }
        at += len;
    }
    out.resize(rows, Cell::default());
    out
}

fn uni_envelope(flags: u8, count: u8, sustain: u8, ls: u8, le: u8, points: &[(i16, i16)], pan: bool) -> Envelope {
    let n = usize::from(count).min(points.len());
    let pts: Vec<EnvPoint> = points[..n]
        .iter()
        .map(|&(t, v)| {
            let v = (v / 4).clamp(0, 64);
            EnvPoint { tick: t.max(0) as u16, value: if pan { (v - 32) as i8 } else { v as i8 } }
        })
        .collect();
    let n = pts.len() as u8;
    Envelope {
        enabled: flags & 1 != 0 && n > 0,
        sustain: (flags & 2 != 0 && sustain < n).then_some((sustain, sustain)),
        looped: (flags & 4 != 0 && ls <= le && le < n).then_some((ls, le)),
        points: pts,
        ..Default::default()
    }
}

pub fn load_uni(d: &[u8]) -> Result<Module> {
    let v5 = d.get(3) == Some(&b'5');
    let mut r = Reader::at(d, 4);
    let nch = usize::from(r.u8()?);
    let npos = usize::from(r.u16le()?);
    let restart = if v5 { r.u16le()? } else { 0 };
    let npat = usize::from(r.u16le()?);
    let ntrk = usize::from(r.u16le()?);
    let nins = usize::from(r.u16le()?);
    let (speed, tempo) = (r.u8()?, r.u8()?);
    let positions = r.bytes(256)?;
    let pans = r.bytes(32)?.to_vec();
    let flags = r.u8()?;
    if nch == 0 || nch > 32 || npos > 256 {
        return bad("its header isn't a UNITRK one");
    }
    let title = uni_string(&mut r)?;
    let original = uni_string(&mut r)?;
    let comment = uni_string(&mut r)?;
    // UN04's sample flags are fewer, and numbered otherwise.
    let xm_periods = flags & 1 != 0;

    struct Head {
        name: String,
        c5: u32,
        volume: u8,
        pan: Option<u8>,
        len: usize,
        looped: Loop,
        pcm: Pcm,
        vibrato: AutoVibrato,
    }
    let mut instruments = Vec::with_capacity(nins);
    let mut heads: Vec<Head> = Vec::new();
    let mut plain = true;
    for i in 0..nins {
        let nsmp = usize::from(r.u8()?);
        let keymap = r.bytes(96)?.to_vec();
        let mut envs = Vec::with_capacity(2);
        for pan in [false, true] {
            let (fl, n, sus, ls, le) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?, r.u8()?);
            let pts: Vec<(i16, i16)> = (0..12).map(|_| Ok((r.i16le()?, r.i16le()?))).collect::<Result<_>>()?;
            envs.push(uni_envelope(fl, n, sus, ls, le, &pts, pan));
        }
        let vib = AutoVibrato { kind: r.u8()?, sweep: r.u8()?, depth: r.u8()?, speed: r.u8()? };
        let fade = r.u16le()?;
        let name = uni_string(&mut r)?;
        let first = heads.len();
        for _ in 0..nsmp {
            let rate = r.u16le()?;
            let transpose = r.i8()?;
            let volume = r.u8()?;
            let pan = r.u8()?;
            let len = r.u32le()? as usize;
            let (ls, le) = (r.u32le()?, r.u32le()?);
            let sf = r.u16le()?;
            let sname = uni_string(&mut r)?;
            // (signed, 16-bit, big-endian, delta, loop, ping-pong, own panning)
            let (signed, b16, be, delta, lp, bidi, own) = if v5 {
                (sf & 2 != 0, sf & 1 != 0, sf & 8 != 0, sf & 4 != 0, sf & 16 != 0, sf & 32 != 0, sf & 64 != 0)
            } else {
                (sf & 32 != 0, false, false, sf & 4 != 0, sf & 8 != 0, sf & 16 != 0, sf & 64 != 0)
            };
            let pcm = match (b16, delta, signed, be) {
                (false, true, ..) => Pcm::D8,
                (false, false, true, _) => Pcm::S8,
                (false, false, false, _) => Pcm::U8,
                (true, true, _, false) => Pcm::D16le,
                (true, true, _, true) => Pcm::D16be,
                (true, false, true, false) => Pcm::S16le,
                (true, false, false, false) => Pcm::U16le,
                (true, false, _, true) => Pcm::S16be,
            };
            // With FastTracker 2's periods the rate is the finetune, 128 none.
            let c5 = if xm_periods { relative_c5(transpose, (i32::from(rate) - 128).clamp(-128, 127) as i8) } else { (f64::from(rate.max(1)) * 2f64.powf(f64::from(transpose) / 12.0)).round() as u32 };
            let kind = match (lp, bidi) {
                (false, _) => LoopKind::Off,
                (true, true) => LoopKind::PingPong,
                (true, false) => LoopKind::Forward,
            };
            heads.push(Head { name: sname, c5, volume, pan: own.then_some(pan), len, looped: Loop { kind, start: ls, end: le }, pcm, vibrato: vib });
        }
        let trivial = nsmp == 1 && !envs[0].enabled && !envs[1].enabled && keymap.iter().all(|&k| k == 0);
        plain &= trivial && first == i;
        let mut env = envs.into_iter();
        instruments.push(Instrument {
            name,
            keymap: (0..120)
                .map(|n| {
                    let k = n - 12;
                    let s = if (0..96).contains(&k) { usize::from(keymap[k as usize]) } else { usize::MAX };
                    (n as u8, if s < nsmp { (first + s + 1) as u16 } else { 0 })
                })
                .collect(),
            volume_env: env.next().unwrap_or_default(),
            pan_env: env.next().unwrap_or_default(),
            fadeout: u32::from(fade) * 2,
            global_volume: 128,
            ..Default::default()
        });
    }
    let rows: Vec<usize> = (0..npat).map(|_| r.u16le().map(usize::from)).collect::<Result<_>>()?;
    let which: Vec<usize> = (0..npat * nch).map(|_| r.u16le().map(usize::from)).collect::<Result<_>>()?;
    let mut tracks = Vec::with_capacity(ntrk);
    for _ in 0..ntrk {
        let n = usize::from(r.u16le()?);
        tracks.push(r.bytes(n)?);
    }
    let mut patterns = Vec::with_capacity(npat);
    for (p, &n) in rows.iter().enumerate() {
        let n = n.clamp(1, 1024);
        let mut pat = Pattern::new(n, nch);
        for c in 0..nch {
            let Some(t) = tracks.get(which[p * nch + c]) else { continue };
            for (row, cell) in uni_track(t, n).into_iter().enumerate() {
                pat.cells[row * nch + c] = cell;
            }
        }
        patterns.push(Arc::new(pat));
    }
    let mut samples = Vec::with_capacity(heads.len());
    for h in heads {
        let pcm = decode(r.bytes_upto(h.len * h.pcm.bytes()), h.pcm, h.len);
        samples.push(Sample {
            name: h.name,
            wave: Arc::new(Wave::new(if h.pcm.is_16() { 16 } else { 8 }, 1, pcm)),
            c5: h.c5,
            volume: h.volume.min(64),
            pan: h.pan,
            looped: h.looped,
            vibrato: h.vibrato,
            ..Sample::empty()
        });
    }
    let lower = original.to_ascii_lowercase();
    let kind = if lower.contains("fasttracker") || lower.contains("xm") {
        Kind::Xm
    } else if lower.contains("scream") || lower.contains("s3m") || lower.contains("impulse") {
        Kind::S3m
    } else {
        Kind::Mod
    };
    let mut m = Module {
        title,
        message: comment,
        format: "uni".into(),
        format_name: "MikMod UNITRK".into(),
        tracker: if original.is_empty() { "MikMod".into() } else { format!("MikMod (from {original})") },
        kind,
        flags: Flags { linear: flags & 2 != 0, instruments: !plain, ..Default::default() },
        speed,
        tempo: u16::from(tempo),
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        restart,
        channels: pans[..nch].iter().map(|&p| Channel { pan: p, ..Default::default() }).collect(),
        orders: positions[..npos].iter().map(|&o| if o == 255 { ORDER_END } else { u16::from(o) }).collect(),
        patterns,
        samples,
        instruments: if plain { Vec::new() } else { instruments },
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Extreme's Tracker and Velvet Studio ----
//
// Both .ams, both by Extreme (who became Velvet Development): Extreme's
// Tracker's "Extreme" (1.x) and Velvet Studio's "AMShdr" (2.0 to 2.02),
// which adds instruments with envelopes and shadow samples (another
// instrument's sample data with settings of their own). Their patterns
// are packed alike: a byte a cell naming its channel, a note and an
// instrument, then any number of effects, MOD's and their own past
// them. Samples may be packed: runs, then bit planes, then deltas.

pub fn probe_ams(d: &[u8]) -> bool {
    d.len() >= 18 && &d[..7] == b"Extreme" && d[8] == 1
}

pub fn probe_ams2(d: &[u8]) -> bool {
    if d.len() < 16 || &d[..7] != b"AMShdr\x1A" {
        return false;
    }
    let at = 8 + usize::from(d[7]);
    d.len() >= at + 7 && d[at + 1] == 2 && d[at] <= 2
}

/// A string with its length before it.
fn ams_string(r: &mut Reader) -> Result<String> {
    r.pstring()
}

/// A packed sample: a run-length pass (the pack byte, a count and the
/// byte repeated), its bits laid out by plane, and deltas of the
/// magnitude-and-sign kind.
fn ams_unpack(src: &[u8], out_len: usize, pack: u8) -> Vec<u8> {
    let mut temp = Vec::with_capacity(out_len);
    let mut i = 0;
    while i < src.len() && temp.len() < out_len {
        let ch = src[i];
        i += 1;
        if i < src.len() && ch == pack {
            let count = usize::from(src[i]).min(out_len - temp.len());
            i += 1;
            if i < src.len() && count > 0 {
                let v = src[i];
                i += 1;
                temp.extend(std::iter::repeat_n(v, count));
            } else {
                temp.push(pack);
            }
        } else {
            temp.push(ch);
        }
    }
    let n = temp.len();
    let mut dst = vec![0u8; out_len];
    if out_len > 0 {
        let (mut bitcount, mut k) = (0x80u16, 0usize);
        for &al in &temp {
            // Shifted on only when the output wraps round within this byte.
            let mut dh = 0u16;
            for count in 0..8u16 {
                let bl = u16::from(al) & bitcount;
                let bl = (bl | bl << 8) >> ((dh + 8 - count) & 7);
                bitcount = ((bitcount | bitcount << 8) >> 1) & 0xFF;
                dst[k] |= bl as u8;
                k += 1;
                if k >= out_len {
                    k = 0;
                    dh += 1;
                }
            }
            bitcount = ((bitcount | bitcount << 8) >> (dh & 15)) & 0xFF;
        }
    }
    let mut old = 0i8;
    for b in dst.iter_mut().take(n) {
        let mut pos = i32::from(*b);
        if pos != 128 && pos & 0x80 != 0 {
            pos = -(pos & 0x7F);
        }
        old = old.wrapping_sub(pos as i8);
        *b = old as u8;
    }
    dst
}

/// A sample's data: packed (the size it unpacks to, the packed size and
/// the pack byte first) or plain, `frames` of them.
fn ams_sample_data(r: &mut Reader, frames: usize, b16: bool, packed: bool) -> Vec<i16> {
    let bytes = frames * if b16 { 2 } else { 1 };
    if packed {
        let head = r.bytes_upto(9);
        if head.len() < 9 {
            return Vec::new();
        }
        let size = le32(head, 4) as usize;
        let raw = ams_unpack(r.bytes_upto(size), bytes, head[8]);
        decode(&raw, if b16 { Pcm::S16le } else { Pcm::S8 }, frames)
    } else {
        decode(r.bytes_upto(bytes), if b16 { Pcm::S16le } else { Pcm::S8 }, frames)
    }
}

/// An effect read into a cell: the first goes to the effect, a later
/// one to the volume column when it can go there.
fn ams_place(cell: &mut Cell, fx: Fx, p: u8) {
    if fx == Fx::None {
        return;
    }
    if cell.fx == Fx::None {
        (cell.fx, cell.param) = (fx, p);
        return;
    }
    if cell.vol != Vol::None {
        return;
    }
    (cell.vol, cell.volp) = match fx {
        Fx::VolSlide if p & 0x0F == 0 => (Vol::VolSlideUp, (p >> 4).min(9)),
        Fx::VolSlide if p >> 4 == 0 => (Vol::VolSlideDown, p.min(9)),
        Fx::Panning => (Vol::Panning, (u16::from(p) * 64 / 255) as u8),
        Fx::TonePorta => (Vol::TonePorta, (p / 16).min(9)),
        Fx::Vibrato => (Vol::VibratoDepth, (p & 0x0F).min(9)),
        _ => return,
    };
}

/// An effect, MOD's (below 0x10) or the trackers' own.
fn ams_effect(cell: &mut Cell, e: u8, p: u8) {
    let (fx, param) = match e {
        0x08 => (Fx::Panning, (p & 0x0F) * 0x11),
        0x0C => {
            if cell.vol == Vol::None {
                (cell.vol, cell.volp) = (Vol::Volume, ((u16::from(p) + 1) / 2).min(64) as u8);
            }
            return;
        }
        0x0E if p == 0x80 => return,
        0x0E => super::common::mod_ex_to_s3m(p),
        0x00..=0x0F => mod_effect(e, p, false),
        // Play forwards or backwards.
        0x10 if p <= 1 => (Fx::S3mEx, 0x9E | p),
        0x11 => (Fx::PortaUp, 0xE0 | p.min(15)),
        0x12 => (Fx::PortaDown, 0xE0 | p.min(15)),
        0x13 => (Fx::Retrig, p),
        0x15 => (Fx::TonePortaVol, 0xF0 | ((u16::from(p) + 1).min(16) as u8 / 2)),
        0x16 => (Fx::VibratoVol, 0xF0 | ((u16::from(p) + 1).min(16) as u8 / 2)),
        0x18 => (Fx::PanningSlide, p),
        0x1A => (Fx::VolSlide, p),
        0x1C => (Fx::ChannelVolume, ((u16::from(p) + 1) / 2).min(64) as u8),
        0x1D => (Fx::PatternBreak, p),
        0x1E => match p >> 4 {
            0x1 => (Fx::PortaUp, 0xF0 | (p & 15)),
            0x2 => (Fx::PortaDown, 0xF0 | (p & 15)),
            0xA => (Fx::VolSlide, ((((p & 15) + 1) / 2) << 4) | 0x0F),
            0xB => (Fx::VolSlide, 0xF0 | (((p & 15) + 1) / 2)),
            _ => return,
        },
        0x20 => (Fx::KeyOff, p),
        0x21 => (Fx::PortaUp, p),
        0x22 => (Fx::PortaDown, p),
        0x2A => (Fx::GlobalVolSlide, p),
        0x2C => (Fx::GlobalVolume, p.min(128)),
        _ => return,
    };
    ams_place(cell, fx, param);
}

/// A pattern's packed cells; `new` is Velvet Studio's notes.
fn ams_pattern(data: &[u8], rows: usize, nch: usize, new: bool) -> Pattern {
    let mut p = Pattern::new(rows, nch);
    let mut r = Reader::new(data);
    let mut dummy = Cell::default();
    'rows: for row in 0..rows {
        loop {
            let Ok(flags) = r.u8() else { break 'rows };
            if flags == 0xFF {
                break;
            }
            let ch = usize::from(flags & 0x1F);
            let cell = if ch < nch { &mut p.cells[row * nch + ch] } else { &mut dummy };
            let mut more = true;
            if flags & 0x40 == 0 {
                let Ok(n) = r.u8() else { break 'rows };
                more = n & 0x80 != 0;
                let n = n & 0x7F;
                cell.note = match n {
                    1 => NOTE_OFF,
                    2..=121 if new => n - 1,
                    12..=108 if !new => n + 13,
                    _ => cell.note,
                };
                cell.instr = r.u8().unwrap_or(0);
            }
            while more {
                let Ok(c) = r.u8() else { break 'rows };
                more = c & 0x80 != 0;
                let e = c & 0x3F;
                if c & 0x40 != 0 {
                    if cell.vol == Vol::None {
                        (cell.vol, cell.volp) = (Vol::Volume, e);
                    }
                } else {
                    let Ok(param) = r.u8() else { break 'rows };
                    ams_effect(cell, e, param);
                }
            }
            if flags & 0x80 != 0 {
                break;
            }
        }
    }
    p
}

/// The song message: Extreme's Tracker's (a high bit a run of spaces,
/// lines of 76 characters) or Velvet Studio's (0xFF, a byte and a count;
/// lines of 74).
fn ams_message(b: &[u8], new: bool) -> String {
    let mut text = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        i += 1;
        if !new && c & 0x80 != 0 {
            text.extend(std::iter::repeat_n(b' ', usize::from(c & 0x7F)));
        } else if new && c == 0xFF && b.len() - i >= 2 {
            text.extend(std::iter::repeat_n(b[i], usize::from(b[i + 1])));
            i += 2;
        } else {
            text.push(c);
        }
    }
    let width = if new { 74 } else { 76 };
    let lines: Vec<String> = text.chunks(width).map(|l| crate::io::message(l).trim_end().to_string()).collect();
    lines.join("\n").trim_end().to_string()
}

/// A rate from a base rate (half of it written), a MOD finetune nibble
/// and a transpose.
fn ams_c5(rate: u16, ft: u8, transpose: i8) -> u32 {
    let base = 2.0 * f64::from(if rate == 0 { 8363 } else { rate });
    let ft = f64::from(((ft & 0x0F) as i8) << 4 >> 4);
    (base * 2f64.powf((f64::from(transpose) + ft / 8.0) / 12.0)).round().clamp(1.0, 2_000_000.0) as u32
}

pub fn load_ams(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 7);
    let (vlo, vhi) = (r.u8()?, r.u8()?);
    let nch = usize::from(r.u8()? & 0x1F) + 1;
    let nsmp = usize::from(r.u8()?);
    let npat = usize::from(r.u16le()?);
    let nord = usize::from(r.u16le()?);
    r.skip(1)?;
    let extra = usize::from(r.u16le()?);
    r.skip(extra)?;
    let mut heads = Vec::with_capacity(nsmp);
    for _ in 0..nsmp {
        let h = r.bytes(17)?;
        heads.push((le32(h, 0), le32(h, 4), le32(h, 8), h[12], u16::from_le_bytes([h[13], h[14]]), h[15], h[16]));
    }
    let title = ams_string(&mut r)?;
    let names: Vec<String> = (0..nsmp).map(|_| ams_string(&mut r)).collect::<Result<_>>()?;
    let chnames: Vec<String> = (0..nch).map(|_| ams_string(&mut r)).collect::<Result<_>>()?;
    let patnames: Vec<String> = (0..npat).map(|_| ams_string(&mut r)).collect::<Result<_>>()?;
    let mlen = usize::from(r.u16le()?);
    let message = ams_message(r.bytes(mlen)?, false);
    let orders: Vec<u16> = (0..nord).map(|_| r.u16le()).collect::<Result<_>>()?;
    let mut patterns = Vec::with_capacity(npat);
    for name in patnames {
        let Ok(len) = r.u32le() else { break };
        let mut p = ams_pattern(r.bytes_upto(len as usize), 64, nch, false);
        p.name = name;
        patterns.push(Arc::new(p));
    }
    let mut samples = Vec::with_capacity(nsmp);
    for ((len, ls, le, pan_ft, rate, vol, flags), name) in heads.into_iter().zip(names) {
        let b16 = flags & 0x84 != 0;
        let pcm = ams_sample_data(&mut r, len as usize, b16, flags & 3 != 0);
        samples.push(Sample {
            name,
            wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
            c5: ams_c5(rate, pan_ft, 0),
            volume: ((u16::from(vol.min(127)) * 64 + 63) / 127) as u8,
            pan: (pan_ft & 0xF0 != 0).then_some(pan_ft & 0xF0),
            looped: if ls < le { Loop::forward(ls, le) } else { Loop::default() },
            ..Sample::empty()
        });
    }
    let mut m = Module {
        title,
        message,
        format: "ams".into(),
        format_name: "Extreme's Tracker".into(),
        tracker: format!("Extreme's Tracker {vhi}.{vlo}"),
        kind: Kind::It,
        flags: Flags { old_effects: true, compat_gxx: true, ..Default::default() },
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels: chnames.into_iter().enumerate().map(|(c, name)| Channel { pan: super::common::amiga_pan(c), name, ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

/// A Velvet Studio envelope as written: its points (tick, value), and
/// the sustain and loop points; the flags come with the instrument.
type Ams2Envelope = (Vec<(u16, u8)>, u8, u8, u8);

/// A Velvet Studio envelope: each point a 9-bit tick step and a value.
fn ams2_envelope(r: &mut Reader) -> Result<Ams2Envelope> {
    let (_speed, sus, ls, le, n) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?, r.u8()?);
    let data = r.bytes(usize::from(n) * 3)?;
    let mut pts = Vec::with_capacity(usize::from(n).min(64));
    let mut tick = 0u16;
    for (i, p) in data.chunks_exact(3).take(64).enumerate() {
        if i > 0 {
            tick = tick.saturating_add((u16::from(p[0]) | u16::from(p[1] & 1) << 8).max(1));
        }
        pts.push((tick, p[2]));
    }
    Ok((pts, sus, ls, le))
}

pub fn load_ams2(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 7);
    let title = ams_string(&mut r)?;
    let (vlo, vhi) = (r.u8()?, r.u8()?);
    let nins = usize::from(r.u8()?);
    let npat = usize::from(r.u16le()?);
    let nord = usize::from(r.u16le()?);
    let (tempo, speed, hflags) = if vlo >= 2 {
        let t = r.u16le()?.max(32 << 8);
        let s = r.u8()?.max(1);
        r.skip(3)?;
        (((u32::from(t) + 128) >> 8) as u16, s, r.u16le()?)
    } else {
        (u16::from(r.u8()?.max(32)), r.u8()?.max(1), u16::from(r.u8()?))
    };
    struct Head {
        sample: Sample,
        frames: usize,
        b16: bool,
        packed: bool,
        /// The instrument (1-based) and sample its data is shared with.
        shadow: Option<(usize, usize)>,
    }
    let mut heads: Vec<Head> = Vec::new();
    let mut firsts = Vec::with_capacity(nins);
    let mut instruments = Vec::with_capacity(nins);
    for _ in 0..nins {
        let name = ams_string(&mut r)?;
        let nsmp = usize::from(r.u8()?);
        let first = heads.len();
        firsts.push(first);
        if nsmp == 0 {
            instruments.push(Instrument { name, keymap: (0..120).map(|n| (n as u8, 0)).collect(), global_volume: 128, ..Default::default() });
            continue;
        }
        let mut keys = [0u8; 120];
        if vlo == 0 {
            keys[12..108].copy_from_slice(r.bytes(96)?);
        } else {
            keys.copy_from_slice(r.bytes(120)?);
        }
        let envs = [ams2_envelope(&mut r)?, ams2_envelope(&mut r)?, ams2_envelope(&mut r)?];
        let shadow = usize::from(r.u8()?);
        let vf = r.u16le()?;
        let eflags = r.u16le()?;
        let amp = 1i32 << ((vf & 0x3000) >> 12);
        let env = |which: usize| -> Envelope {
            let (pts, sus, ls, le) = &envs[which];
            if pts.len() <= 1 {
                return Envelope::default();
            }
            let f = eflags >> (which * 3);
            let points: Vec<EnvPoint> = pts
                .iter()
                .map(|&(tick, v)| EnvPoint {
                    tick,
                    value: match which {
                        0 => ((u32::from(v) * 64 + 64) / 127).min(64) as i8,
                        1 => (((u32::from(v) * 64 + 128) / 255).min(64) as i32 - 32) as i8,
                        _ => (i32::from(v.wrapping_sub(128) as i8) * amp).div_euclid(255).clamp(-32, 32) as i8,
                    },
                })
                .collect();
            let n = points.len() as u8;
            let (mut looped, mut sustain) = ((f & 1 != 0).then_some((*ls, *le)), (f & 2 != 0).then_some((*sus, *sus)));
            // "Break envelope": its loop ends at a note off, as a sustain loop does.
            if f & 2 == 0 && f & 1 != 0 && eflags & (1 << (9 - which * 2)) != 0 {
                sustain = looped.take();
            }
            let ok = |p: Option<(u8, u8)>| p.filter(|&(a, b)| a <= b && b < n);
            Envelope { enabled: f & 4 != 0, looped: ok(looped), sustain: ok(sustain), points, ..Default::default() }
        };
        let (volume_env, pan_env, pitch_env) = (env(0), env(1), env(2));
        for k in 0..nsmp {
            let sname = ams_string(&mut r)?;
            let h = r.bytes(20)?;
            let (len, ls, le) = (le32(h, 0), le32(h, 4), le32(h, 8));
            let (pan_ft, c4, rel, vol, flags) = (h[14], u16::from_le_bytes([h[15], h[16]]), h[17] as i8, h[18], h[19]);
            let kind = match (flags & 8 != 0 && ls < le, flags & 0x10 != 0) {
                (false, _) => LoopKind::Off,
                (true, true) => LoopKind::PingPong,
                (true, false) => LoopKind::Forward,
            };
            heads.push(Head {
                sample: Sample {
                    name: sname,
                    c5: ams_c5(c4, pan_ft, rel),
                    volume: ((u16::from(vol.min(127)) * 64 + 63) / 127) as u8,
                    pan: (pan_ft & 0xF0 != 0).then_some(pan_ft & 0xF0),
                    looped: Loop { kind, start: ls.min(len), end: le.min(len) },
                    ..Sample::empty()
                },
                frames: len as usize,
                b16: flags & 4 != 0,
                packed: flags & 3 != 0,
                shadow: (shadow != 0).then_some((shadow, k)),
            });
        }
        instruments.push(Instrument {
            name,
            keymap: (0..120).map(|n| (n as u8, if usize::from(keys[n]) < nsmp { (first + usize::from(keys[n]) + 1) as u16 } else { 0 })).collect(),
            volume_env,
            pan_env,
            pitch_env,
            fadeout: u32::from(vf & 0xFFF) * 2,
            global_volume: 128,
            ..Default::default()
        });
    }
    let artist = ams_string(&mut r)?;
    let chnames: Vec<String> = (0..32).map(|_| ams_string(&mut r)).collect::<Result<_>>()?;
    let packed_len = r.u32le()? as usize;
    r.skip(7)?;
    let message = ams_message(r.bytes(packed_len.saturating_sub(11))?, true);
    let orders: Vec<u16> = (0..nord).map(|_| r.u16le()).collect::<Result<_>>()?;
    let mut patterns = Vec::with_capacity(npat);
    for _ in 0..npat {
        let Ok(len) = r.u32le() else { break };
        let mut pr = Reader::new(r.bytes_upto(len as usize));
        let rows = usize::from(pr.u8().unwrap_or(63)) + 1;
        pr.skip(1).ok();
        let name = ams_string(&mut pr).unwrap_or_default();
        let mut p = ams_pattern(&pr.data[pr.pos.min(pr.data.len())..], rows, 32, true);
        p.name = name;
        patterns.push(Arc::new(p));
    }
    let mut waves: Vec<Option<Arc<Wave>>> = Vec::with_capacity(heads.len());
    for h in &heads {
        waves.push(if h.shadow.is_none() {
            let pcm = ams_sample_data(&mut r, h.frames, h.b16, h.packed);
            Some(Arc::new(Wave::new(if h.b16 { 16 } else { 8 }, 1, pcm)))
        } else {
            None
        });
    }
    let mut samples = Vec::with_capacity(heads.len());
    for (i, h) in heads.into_iter().enumerate() {
        let from = h.shadow.and_then(|(ins, k)| firsts.get(ins - 1).map(|f| f + k)).and_then(|s| waves.get(s).cloned().flatten());
        let wave = waves[i].clone().or(from).unwrap_or_default();
        samples.push(Sample { wave, ..h.sample });
    }
    // Velvet Studio always has 32 channels: as many as are written to.
    let used = patterns.iter().flat_map(|p| (0..32).filter(move |&c| (0..p.rows).any(|r| !p.cell(r, c).is_empty()))).max().map_or(1, |c| c + 1);
    let mut m = Module {
        title,
        artist,
        message,
        format: "ams2".into(),
        format_name: "Velvet Studio".into(),
        tracker: format!("Velvet Studio {vhi}.{vlo:02}"),
        kind: Kind::It,
        flags: Flags { linear: hflags & 0x40 != 0, instruments: true, old_effects: true, compat_gxx: true, ..Default::default() },
        speed,
        tempo,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels: chnames.into_iter().take(used).enumerate().map(|(c, name)| Channel { pan: super::common::amiga_pan(c), name, ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        instruments,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

/// Where an effect goes in the volume column, when it can.
fn vol_form(fx: Fx, p: u8) -> Option<(Vol, u8)> {
    Some(match fx {
        Fx::Volume => (Vol::Volume, p.min(64)),
        Fx::VolSlide if p & 0x0F == 0 => (Vol::VolSlideUp, (p >> 4).min(9)),
        Fx::VolSlide if p >> 4 == 0 => (Vol::VolSlideDown, p.min(9)),
        Fx::VolSlide if p & 0x0F == 0x0F && p >> 4 != 0 => (Vol::FineVolUp, (p >> 4).min(9)),
        Fx::VolSlide if p >> 4 == 0x0F && p & 0x0F != 0 => (Vol::FineVolDown, (p & 0x0F).min(9)),
        Fx::Panning => (Vol::Panning, (u16::from(p) * 64 / 255) as u8),
        Fx::TonePorta => (Vol::TonePorta, (p / 16).min(9)),
        Fx::Vibrato => (Vol::VibratoDepth, (p & 0x0F).min(9)),
        Fx::PortaUp if p < 0xE0 => (Vol::PortaUp, (p / 4).min(9)),
        Fx::PortaDown if p < 0xE0 => (Vol::PortaDown, (p / 4).min(9)),
        _ => return None,
    })
}

/// Two effects in one cell: the first to the effect and the second to
/// the volume column, or the other way round, or the one that matters
/// more when neither fits there.
fn two_effects(cell: &mut Cell, (f1, p1): (Fx, u8), (f2, p2): (Fx, u8)) {
    if f1 == Fx::None || (f1 == f2 && f1 != Fx::S3mEx) {
        (cell.fx, cell.param) = (f2, p2);
        return;
    }
    if f2 == Fx::None {
        (cell.fx, cell.param) = (f1, p1);
        return;
    }
    if cell.vol == Vol::None {
        if let Some((v, x)) = vol_form(f2, p2) {
            (cell.fx, cell.param, cell.vol, cell.volp) = (f1, p1, v, x);
            return;
        }
        if let Some((v, x)) = vol_form(f1, p1) {
            (cell.fx, cell.param, cell.vol, cell.volp) = (f2, p2, v, x);
            return;
        }
    }
    // Speed, tempo, jumps and breaks before the rest.
    let weight = |f: Fx| matches!(f, Fx::PositionJump | Fx::PatternBreak | Fx::Speed | Fx::Tempo | Fx::Offset);
    (cell.fx, cell.param) = if weight(f2) && !weight(f1) { (f2, p2) } else { (f1, p1) };
}

// ---- Digitrakker ----
//
// "DMDL" and a version, then chunks of a two-letter name and a length:
// IN the song (title, orders, the channels), ME the message, PA the
// patterns (a track for each channel), TR the tracks, II instruments
// (a sample for each range of notes, with its volume, panning and
// vibrato), VE, PE and FE envelopes, IS the samples' headers, SA their
// data, which may be packed: a delta a point in bits of their own.

pub fn probe_mdl(d: &[u8]) -> bool {
    d.len() >= 11 && &d[..4] == b"DMDL" && d[4] < 0x20 && &d[5..7] == b"IN"
}

/// An MDL effect (the second column's G to L as 0x10 to 0x15).
fn mdl_effect(cmd: u8, p: u8) -> (Fx, u8) {
    match cmd {
        0x01 => (Fx::PortaUp, p),
        0x02 => (Fx::PortaDown, p),
        0x03 => (Fx::TonePorta, p),
        0x04 => (Fx::Vibrato, p),
        0x05 => (Fx::Arpeggio, p),
        0x07 => (Fx::Tempo, p.max(32)),
        0x08 => (Fx::Panning, (p & 0x7F) * 2),
        // An envelope on: there's one of each here.
        0x09 if p < 0xC0 => (Fx::S3mEx, 0x78 + (p >> 6) * 2),
        0x0B => (Fx::PositionJump, p),
        0x0C => (Fx::GlobalVolume, ((u16::from(p) + 1) / 2) as u8),
        0x0D => (Fx::PatternBreak, 10 * (p >> 4) + (p & 0x0F)),
        0x0E => {
            let x = p & 0x0F;
            match p >> 4 {
                0x1 => (Fx::PanningSlide, x.min(0x0E) << 4 | 0x0F),
                0x2 => (Fx::PanningSlide, 0xF0 | x.min(0x0E)),
                0x4 => (Fx::S3mEx, 0x30 | x),
                0x5 => (Fx::SetFinetune, x),
                0x6 => (Fx::S3mEx, 0xB0 | x),
                0x7 => (Fx::S3mEx, 0x40 | x),
                0x9 => (Fx::Retrig, x),
                0xA => (Fx::GlobalVolSlide, 0xF0 & ((x + 1) << 3)),
                0xB => (Fx::GlobalVolSlide, (x + 1) >> 1),
                0xC..=0xE => (Fx::S3mEx, p),
                0xF => (Fx::Offset, p),
                _ => (Fx::None, 0),
            }
        }
        0x0F => (Fx::Speed, p),
        // Four times finer than XM's; E0 to EF finer still, on the first tick.
        0x10 => (
            Fx::VolSlide,
            match p {
                0..=0xDF => (p >> 2).min(15) << 4,
                0xE0..=0xEF => (p & 0x0F) << 2 | 0x0F,
                _ => p << 4 | 0x0F,
            },
        ),
        0x11 => (
            Fx::VolSlide,
            match p {
                0..=0xDF => (p >> 2).min(15),
                0xE0..=0xEF => (p & 0x0F) >> 2 | 0xF0,
                _ => p,
            },
        ),
        0x12 => (Fx::Retrig, p),
        0x13 => (Fx::Tremolo, p),
        0x14 => (Fx::Tremor, p),
        _ => (Fx::None, 0),
    }
}

/// A cell's volume and two effects. EFx in the first column and its
/// second column's parameter make one offset, xyy × 256.
fn mdl_cell(cell: &mut Cell, vol: u8, c1: u8, c2: u8, p1: u8, p2: u8) {
    let c2 = if (1..=6).contains(&c2) { c2 + 15 } else { c2 };
    let (mut e1, mut q1) = mdl_effect(c1, p1);
    let (mut e2, mut q2) = mdl_effect(c2, p2);
    let mut offset = None;
    if e1 == Fx::Offset {
        offset = Some(u32::from(q1 & 0x0F) << 8 | u32::from(p2));
        q1 = if q1 & 0x0F != 0 { 0xFF } else { p2 };
        if e2 == Fx::Offset {
            e2 = Fx::None;
        }
    } else if e2 == Fx::Offset {
        offset = Some(u32::from(q2 & 0x0F) << 8);
        q2 = if q2 & 0x0F != 0 { 0xFF } else { 0 };
    }
    if let Some(o) = offset.filter(|&o| o > 0xFF) {
        let other = if e1 == Fx::Offset { e2 } else { e1 };
        if !matches!(other, Fx::PositionJump | Fx::PatternBreak | Fx::Speed | Fx::Tempo) {
            (cell.fx, cell.param, cell.vol, cell.volp) = (Fx::Offset, (o & 0xFF) as u8, Vol::Offset, (o >> 8) as u8);
            return;
        }
        if e1 == Fx::Offset {
            e1 = Fx::None;
        } else {
            e2 = Fx::None;
        }
    }
    if vol != 0 {
        (cell.vol, cell.volp) = (Vol::Volume, ((u16::from(vol) + 2) / 4).min(64) as u8);
    }
    two_effects(cell, (e1, q1), (e2, q2));
}

/// A track's cells, `rows` of them.
fn mdl_track(t: &[u8], rows: usize) -> Vec<Cell> {
    let mut out = vec![Cell::default(); rows];
    let mut r = Reader::new(t);
    let mut row = 0;
    while row < rows {
        let Ok(b) = r.u8() else { break };
        let x = usize::from(b >> 2);
        match b & 3 {
            0 => row += x + 1,
            1 => {
                if row > 0 {
                    let prev = out[row - 1];
                    for _ in 0..=x {
                        if row >= rows {
                            break;
                        }
                        out[row] = prev;
                        row += 1;
                    }
                } else {
                    row += 1;
                }
            }
            2 => {
                if row > x {
                    out[row] = out[x];
                }
                row += 1;
            }
            _ => {
                let mut get = |bit: usize| if x & bit != 0 { r.u8().ok() } else { Some(0) };
                let (Some(n), Some(ins), Some(vol), Some(fx), Some(p1), Some(p2)) = (get(1), get(2), get(4), get(8), get(16), get(32)) else { break };
                let cell = &mut out[row];
                if x & 1 != 0 {
                    cell.note = if n > 120 { NOTE_OFF } else { n };
                }
                cell.instr = ins;
                mdl_cell(cell, vol, fx & 0x0F, fx >> 4, p1, p2);
                row += 1;
            }
        }
    }
    out
}

/// An MDL envelope (33 bytes: its number, 15 points of a tick step and
/// a value, flags, loop) as UMF's; `pan` and pitch ones are around 32.
fn mdl_envelope(e: &[u8], centred: bool) -> Envelope {
    let mut points = Vec::with_capacity(15);
    let mut tick = -i32::from(e[1]);
    for n in 0..15 {
        let (x, y) = (e[1 + n * 2], e[2 + n * 2]);
        if x == 0 {
            break;
        }
        tick += i32::from(x);
        let v = i32::from(y.min(64));
        points.push(EnvPoint { tick: tick.max(0) as u16, value: if centred { (v - 32) as i8 } else { v as i8 } });
    }
    let (flags, lp) = (e[31], e[32]);
    let n = points.len() as u8;
    let (ls, le, sus) = (lp & 0x0F, lp >> 4, flags & 0x0F);
    Envelope { sustain: (flags & 0x10 != 0 && sus < n).then_some((sus, sus)), looped: (flags & 0x20 != 0 && ls <= le && le < n).then_some((ls, le)), points, ..Default::default() }
}

/// A packed sample: each point's delta from the last, its sign, then
/// three bits or a run of zero bits (16 each) and four; 16-bit ones have
/// their low byte first, as it is.
fn mdl_unpack(b: &[u8], frames: usize, b16: bool) -> Vec<i16> {
    let mut bits = super::micro::Bits::new(b);
    let mut out = Vec::with_capacity(frames);
    let mut dlt = 0u8;
    'points: for _ in 0..frames {
        let low = if b16 {
            match bits.read(8) {
                Some(v) => v as u8,
                None => break,
            }
        } else {
            0
        };
        let Some(sign) = bits.read(1) else { break };
        let Some(short) = bits.read(1) else { break };
        let mut hi = if short == 1 {
            match bits.read(3) {
                Some(v) => v as u8,
                None => break,
            }
        } else {
            let mut h = 8u8;
            loop {
                match bits.read(1) {
                    Some(0) => h = h.wrapping_add(0x10),
                    Some(_) => break,
                    None => break 'points,
                }
            }
            match bits.read(4) {
                Some(v) => h.wrapping_add(v as u8),
                None => break,
            }
        };
        if sign == 1 {
            hi = !hi;
        }
        dlt = dlt.wrapping_add(hi);
        out.push(if b16 { (u16::from(low) | u16::from(dlt) << 8) as i16 } else { i16::from(dlt as i8) << 8 });
    }
    out
}

pub fn load_mdl(d: &[u8]) -> Result<Module> {
    let version = d[4];
    let mut chunks: Vec<([u8; 2], &[u8])> = Vec::new();
    let mut r = Reader::at(d, 5);
    while r.left() >= 6 {
        let id = r.bytes(2)?;
        let len = r.u32le()? as usize;
        chunks.push(([id[0], id[1]], r.bytes_upto(len)));
    }
    let chunk = |id: &[u8; 2]| chunks.iter().find(|c| &c.0 == id).map(|c| c.1);
    let mut info = Reader::new(chunk(b"IN").ok_or_else(|| crate::io::Error::Bad("it has no song header".into()))?);
    let title = info.text(32)?;
    let artist = info.text(20)?;
    let nord = usize::from(info.u16le()?);
    let restart = info.u16le()?;
    let (gv, speed, tempo) = (info.u8()?, info.u8()?, info.u8()?);
    let setup = info.bytes(32)?.to_vec();
    let orders: Vec<u16> = info.bytes(nord)?.iter().map(|&o| u16::from(o)).collect();

    // The tracks, and the patterns' (each a track a channel, 0 for none).
    let mut tracks: Vec<&[u8]> = vec![&[]];
    if let Some(t) = chunk(b"TR") {
        let mut tr = Reader::new(t);
        let n = usize::from(tr.u16le()?);
        for _ in 0..n {
            let len = usize::from(tr.u16le()?);
            tracks.push(tr.bytes_upto(len));
        }
    }
    let mut pats: Vec<(String, usize, Vec<usize>)> = Vec::new();
    if let Some(p) = chunk(b"PA") {
        let mut pr = Reader::new(p);
        let n = usize::from(pr.u8()?);
        for _ in 0..n {
            let (nch, rows, name) = if version >= 0x10 { (usize::from(pr.u8()?), usize::from(pr.u8()?) + 1, pr.text(16)?) } else { (32, 64, String::new()) };
            let which: Vec<usize> = (0..nch).map(|_| pr.u16le().map(usize::from)).collect::<Result<_>>()?;
            pats.push((name, rows, which));
        }
    }
    let used = pats.iter().flat_map(|(_, _, w)| w.iter().enumerate().filter(|(c, &t)| t > 0 && *c < 32).map(|(c, _)| c + 1)).max().unwrap_or(0);
    let listed = (0..32).filter(|&c| setup[c] & 0x80 == 0).map(|c| c + 1).max().unwrap_or(0);
    let nch = used.max(listed).max(1);
    let mut channels = Vec::with_capacity(nch);
    for &c in &setup[..nch] {
        let name = info.text(8).unwrap_or_default();
        channels.push(Channel { pan: (c & 0x7F) * 2, muted: c & 0x80 != 0, name, ..Default::default() });
    }
    let mut patterns = Vec::with_capacity(pats.len());
    for (name, rows, which) in pats {
        let mut p = Pattern::new(rows, nch);
        p.name = name;
        for (c, &t) in which.iter().enumerate().take(nch) {
            if let Some(track) = tracks.get(t).filter(|_| t > 0) {
                for (row, cell) in mdl_track(track, rows).into_iter().enumerate() {
                    p.cells[row * nch + c] = cell;
                }
            }
        }
        patterns.push(Arc::new(p));
    }

    let mut samples: Vec<Sample> = Vec::new();
    if let Some(is) = chunk(b"IS") {
        let mut ir = Reader::new(is);
        let mut data = Reader::new(chunk(b"SA").unwrap_or(&[]));
        let n = ir.u8()?;
        for _ in 0..n {
            let index = usize::from(ir.u8()?);
            if index == 0 || ir.left() < 54 {
                break;
            }
            let name = ir.text(32)?;
            let filename = ir.text(8)?;
            let c4 = if version < 0x10 { u32::from(ir.u16le()?) } else { ir.u32le()? };
            let (mut len, mut ls, mut ll) = (ir.u32le()?, ir.u32le()?, ir.u32le()?);
            let vol = ir.u8()?;
            let flags = ir.u8()?;
            let b16 = flags & 1 != 0;
            if b16 {
                (len, ls, ll) = (len / 2, ls / 2, ll / 2);
            }
            let frames = len as usize;
            let pcm = if flags & 0x0C != 0 {
                let size = data.u32le().unwrap_or(0) as usize;
                mdl_unpack(data.bytes_upto(size), frames, b16)
            } else {
                decode(data.bytes_upto(frames * if b16 { 2 } else { 1 }), if b16 { Pcm::S16le } else { Pcm::S8 }, frames)
            };
            if samples.len() < index {
                samples.resize(index, Sample::empty());
            }
            samples[index - 1] = Sample {
                name,
                filename,
                wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
                c5: c4.saturating_mul(2).clamp(1, 2_000_000),
                volume: if version < 0x10 { ((u16::from(vol) + 2) / 4).min(64) as u8 } else { 64 },
                looped: if ll != 0 { Loop { kind: if flags & 2 != 0 { LoopKind::PingPong } else { LoopKind::Forward }, start: ls, end: ls + ll } } else { Loop::default() },
                ..Sample::empty()
            };
        }
    }

    let mut instruments: Vec<Instrument> = Vec::new();
    if let Some(ii) = chunk(b"II") {
        let envs = |id: &[u8; 2]| -> Vec<Option<&[u8]>> {
            let mut v = vec![None; 64];
            if let Some(c) = chunk(id) {
                let n = c.first().copied().unwrap_or(0);
                for e in c[1..].chunks_exact(33).take(usize::from(n)) {
                    if let Some(slot) = v.get_mut(usize::from(e[0])) {
                        *slot = Some(e);
                    }
                }
            }
            v
        };
        let (vol_envs, pan_envs, pitch_envs) = (envs(b"VE"), envs(b"PE"), envs(b"FE"));
        let env = |set: &[Option<&[u8]>], flags: u8, centred: bool| -> Envelope {
            let mut e = set.get(usize::from(flags & 0x3F)).copied().flatten().map(|b| mdl_envelope(b, centred)).unwrap_or_default();
            e.enabled = flags & 0x80 != 0 && !e.points.is_empty();
            e
        };
        let mut ir = Reader::new(ii);
        let n = ir.u8()?;
        for _ in 0..n {
            let (ins, nsmp) = (usize::from(ir.u8()?), usize::from(ir.u8()?));
            let name = ir.text(32)?;
            let heads = ir.bytes(nsmp * 14)?;
            if ins == 0 {
                continue;
            }
            let mut inst = Instrument { name, keymap: (0..120).map(|k| (k as u8, 0)).collect(), global_volume: 128, ..Default::default() };
            let mut first = 0usize;
            for h in heads.chunks_exact(14) {
                let smp = usize::from(h[0]);
                if smp == 0 || smp > samples.len() {
                    continue;
                }
                let last = usize::from(h[1]).min(119);
                for k in first..=last {
                    inst.keymap[k].1 = smp as u16;
                }
                first = last + 1;
                inst.volume_env = env(&vol_envs, h[3], false);
                inst.pan_env = env(&pan_envs, h[5], true);
                inst.pitch_env = env(&pitch_envs, h[13], true);
                inst.fadeout = u32::from(u16::from_le_bytes([h[6], h[7]]));
                let s = &mut samples[smp - 1];
                if h[3] & 0x40 != 0 {
                    s.volume = ((u16::from(h[2]) + 2) / 4).min(64) as u8;
                }
                if h[5] & 0x40 != 0 {
                    s.pan = Some((u16::from(h[4]) * 2).min(254) as u8);
                }
                s.vibrato = AutoVibrato {
                    kind: match h[11] & 3 {
                        1 => 1,
                        2 => 2,
                        _ => 0,
                    },
                    speed: h[8],
                    depth: ((u16::from(h[9]) + 3) / 4) as u8,
                    sweep: h[10],
                };
            }
            if instruments.len() < ins {
                instruments.resize(ins, Instrument { keymap: (0..120).map(|k| (k as u8, 0)).collect(), global_volume: 128, ..Default::default() });
            }
            instruments[ins - 1] = inst;
        }
    }
    let mut m = Module {
        title,
        artist,
        message: chunk(b"ME").map(crate::io::message).unwrap_or_default(),
        format: "mdl".into(),
        format_name: "Digitrakker".into(),
        tracker: match version {
            0x11 => "Digitrakker 3",
            0x10 => "Digitrakker 2.3",
            _ => "Digitrakker 2",
        }
        .into(),
        kind: Kind::It,
        flags: Flags { linear: true, instruments: true, compat_gxx: true, ..Default::default() },
        speed: speed.max(1),
        tempo: u16::from(tempo).max(32),
        global_volume: ((u16::from(gv) + 1) / 2).min(128) as u8,
        mix_volume: 48,
        separation: 128,
        restart,
        channels,
        orders,
        patterns,
        samples,
        instruments,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- X-Tracker ----
//
// D-Lusion's Digital Music Format: "DDMF", a version (1 to 10), the
// title and composer, then chunks: SEQU the orders (and a loop), PATT
// the patterns, SMPI the samples' headers, SMPD their data (plain, or
// packed with a Huffman tree of deltas), CMSG a message. A pattern has a
// global track (tempo: ticks a quarter-second or real beats a minute,
// and rows a beat) and a track a channel with three effect columns: the
// instrument's, the note's and the volume's, their times in 256ths of
// a row. Read here as OpenMPT reads it: a channel of its own for the
// tempo, each order its own pattern (the tempo carries on from one to
// the next), the times turned into ticks.

pub fn probe_dmf_xt(d: &[u8]) -> bool {
    d.len() >= 66 && &d[..4] == b"DDMF" && (1..=10).contains(&d[4])
}

/// The tempo as it is between patterns.
struct DmfTempo {
    bpm_mode: bool,
    beat: u8,
    ticks: u8,
    bpm: u8,
    /// Ticks a row, as read.
    speed: u8,
}

/// The channel's memory between patterns.
#[derive(Clone, Copy)]
struct DmfChannel {
    buffered: u8,
    last: u8,
    vib_type: u8,
    trem_type: u8,
    high_offset: u8,
    backwards: bool,
}

fn dmf_porta(v: u8, ticks: u8, fine: bool) -> u8 {
    if v == 0 {
        0
    } else if (v <= 0x0F && fine) || ticks < 2 {
        v | 0xF0
    } else {
        (v / (ticks - 1)).max(1)
    }
}

fn dmf_slide(v: u8, ticks: u8, up: bool) -> u8 {
    let mut v = (v / 4).max(1);
    let fine = v < 0x0F || ticks < 2;
    if !fine {
        v = ((u16::from(v) + u16::from(ticks) - 2) / u16::from(ticks - 1)).max(1) as u8;
    }
    match (up, fine) {
        (true, true) => 0x0F | v << 4,
        (true, false) => v << 4,
        (false, true) => 0xF0 | (v & 0x0F),
        (false, false) => v & 0x0F,
    }
}

fn dmf_tremor(v: u8, ticks: u8) -> u8 {
    let on = (u32::from(v >> 4) * u32::from(ticks) / 15).clamp(1, 15) as u8;
    let off = (u32::from(v & 0x0F) * u32::from(ticks) / 15).clamp(1, 15) as u8;
    on << 4 | off
}

fn dmf_delay(v: u8, ticks: u8) -> u8 {
    (u32::from(v) * u32::from(ticks) / 255).min(15) as u8
}

/// A vibrato's speed is its period in rows here.
fn dmf_vibrato(v: u8, ticks: u8) -> u8 {
    let period = u32::from((v >> 4).max(1)) * u32::from(ticks);
    ((128 / period).clamp(1, 15) as u8) << 4 | (v & 0x0F).max(1)
}

/// Writes an effect in the first free place on `row`, or the row after.
fn put_effect(p: &mut Pattern, row: usize, fx: Fx, param: u8, from: usize) {
    for r in row..(row + 2).min(p.rows) {
        for c in from..p.channels {
            if let Some(cell) = p.cell_mut(r, c) {
                if cell.fx == Fx::None {
                    (cell.fx, cell.param) = (fx, param);
                    return;
                }
            }
        }
    }
}

fn dmf_pattern(data: &[u8], version: u8, nch: usize, tempo: &mut DmfTempo, chans: &mut [DmfChannel]) -> Pattern {
    let mut r = Reader::new(data);
    let (tracks, beat, rows) = if version < 3 {
        let t = r.u8().unwrap_or(0);
        r.skip(2).ok();
        (t, 0, r.u16le().unwrap_or(64))
    } else {
        let t = r.u8().unwrap_or(0);
        let b = r.u8().unwrap_or(0);
        (t, if version < 6 { 0 } else { b }, r.u16le().unwrap_or(64))
    };
    r.skip(4).ok();
    let rows = usize::from(rows).clamp(1, 1024);
    let mut p = Pattern::new(rows, nch);
    let tracks = usize::from(tracks).min(nch - 1);
    // Voices on channels this pattern doesn't have stop.
    for c in tracks + 1..nch {
        p.cells[c].note = NOTE_CUT;
    }
    tempo.beat = beat >> 4;
    let mut change = tempo.bpm_mode;
    let mut counters = vec![0u8; tracks + 1];
    for row in 0..rows {
        if r.eof() {
            break;
        }
        let mut delay = 0u8;
        if counters[0] == 0 {
            let info = r.u8().unwrap_or(0);
            if info & 0x80 != 0 {
                counters[0] = r.u8().unwrap_or(0);
            }
            let info = info & 0x3F;
            let v = if info != 0 { r.u8().unwrap_or(0) } else { 0 };
            match info {
                1 => {
                    (tempo.bpm_mode, tempo.ticks, tempo.bpm, change) = (false, v.max(1), 0, true);
                }
                2 if v > 0 => {
                    (tempo.bpm_mode, tempo.bpm, change) = (true, v, true);
                    if tempo.beat != 0 {
                        tempo.ticks = (u32::from(v) * u32::from(tempo.beat) * 15) as u8;
                    }
                }
                3 => {
                    tempo.beat = v >> 4;
                    if tempo.beat != 0 {
                        change = tempo.bpm_mode;
                    } else {
                        tempo.bpm_mode = false;
                    }
                }
                4 => delay = v,
                6 | 7 if v > 0 => {
                    let t = if tempo.bpm_mode { &mut tempo.bpm } else { &mut tempo.ticks };
                    *t = if info == 6 { t.saturating_add(v) } else { t.saturating_sub(v).max(1) };
                    change = true;
                }
                _ => {}
            }
        } else {
            counters[0] -= 1;
        }
        let mut new_tempo = None;
        if change {
            if !tempo.bpm_mode || tempo.beat != 0 {
                let tickspeed = if tempo.bpm_mode { (i32::from(tempo.bpm) * i32::from(tempo.beat) * 2).max(1) } else { (i32::from(tempo.ticks) + 1) * 30 };
                let (mut speed, mut bpm) = (255, 0);
                while speed >= 1 {
                    bpm = tickspeed * speed / 48;
                    if (32..=255).contains(&bpm) {
                        break;
                    }
                    speed -= 1;
                }
                tempo.speed = speed.max(1) as u8;
                new_tempo = Some((bpm.clamp(32, 255) as u8, tempo.speed));
            } else {
                change = false;
            }
        }
        let ticks = tempo.speed;
        for ch in 1..=tracks {
            if counters[ch] != 0 {
                counters[ch] -= 1;
                continue;
            }
            let info = r.u8().unwrap_or(0);
            if info & 0x80 != 0 {
                counters[ch] = r.u8().unwrap_or(0);
            }
            let st = &mut chans[ch];
            let mut cell = Cell::default();
            let mut slide_note = true;
            if info & 0x40 != 0 {
                cell.instr = r.u8().unwrap_or(0);
                slide_note = cell.instr == 0;
            }
            if info & 0x20 != 0 {
                let n = r.u8().unwrap_or(0);
                match n {
                    1..=108 => {
                        cell.note = (n + 24).min(NOTE_MAX);
                        st.last = cell.note;
                    }
                    129..=236 => st.buffered = ((n & 0x7F) + 24).min(NOTE_MAX),
                    255 => cell.note = NOTE_CUT,
                    _ => {}
                }
            }
            // An instrument on its own plays the last note again.
            if cell.note == NOTE_NONE && cell.instr > 0 {
                (cell.note, cell.instr) = (st.last, 0);
            }
            let is_note = (NOTE_MIN..=NOTE_MAX).contains(&cell.note);
            if is_note {
                st.backwards = false;
            }
            let (mut e1, mut e2, mut e3) = ((Fx::None, 0u8), (Fx::None, 0u8), (Fx::None, 0u8));
            let mut vol = None;
            if info & 0x10 != 0 {
                vol = Some(((u16::from(r.u8().unwrap_or(0)) + 2) / 4).min(64) as u8);
            }
            if info & 0x08 != 0 {
                let (cmd, v) = (r.u8().unwrap_or(0), r.u8().unwrap_or(0));
                match cmd {
                    1 => cell.note = NOTE_CUT,
                    2 => cell.note = NOTE_OFF,
                    3 => {
                        cell.note = st.last;
                        st.backwards = false;
                    }
                    4 => {
                        let x = dmf_delay(v, ticks);
                        if x > 0 {
                            e1 = (Fx::S3mEx, 0xD0 | x);
                        }
                        if cell.note == NOTE_NONE {
                            cell.note = st.last;
                            st.backwards = false;
                        }
                    }
                    5 => {
                        e1 = (Fx::Retrig, dmf_delay(v, ticks).max(1));
                        st.backwards = false;
                    }
                    6..=9 => {
                        // The offset's high part (64K steps) goes on the row before.
                        if row > 0 && cmd != st.high_offset {
                            if let Some(prev) = p.cell_mut(row - 1, ch).filter(|c| c.fx == Fx::None) {
                                (prev.fx, prev.param) = (Fx::S3mEx, 0xA0 | (cmd - 6));
                                st.high_offset = cmd;
                            }
                        }
                        e1 = (Fx::Offset, v);
                        if cell.note == NOTE_NONE {
                            cell.note = st.last;
                        }
                        st.backwards = false;
                    }
                    10 => {
                        e1 = (Fx::S3mEx, if st.backwards { 0x9E } else { 0x9F });
                        st.backwards = !st.backwards;
                    }
                    _ => {}
                }
            }
            if info & 0x04 != 0 {
                let (cmd, v) = (r.u8().unwrap_or(0), r.u8().unwrap_or(0));
                e2 = match cmd {
                    // Finetune in 16ths of a semitone: whole semitones on the note.
                    1 => {
                        let fine = i32::from(v as i8) * 8;
                        if (NOTE_MIN..=NOTE_MAX).contains(&cell.note) {
                            cell.note = (i32::from(cell.note) + fine / 128).clamp(i32::from(NOTE_MIN), i32::from(NOTE_MAX)) as u8;
                        }
                        (Fx::None, 0)
                    }
                    2 => match dmf_delay(v, ticks) {
                        0 => (Fx::None, 0),
                        x => (Fx::S3mEx, 0xD0 | x),
                    },
                    3 => (Fx::Arpeggio, v),
                    4 => (Fx::PortaUp, dmf_porta(v, ticks, true)),
                    5 => (Fx::PortaDown, dmf_porta(v, ticks, true)),
                    6 => {
                        if cell.note == NOTE_NONE {
                            cell.note = st.buffered;
                        }
                        (Fx::TonePorta, dmf_porta(v, ticks, false))
                    }
                    // Scratch to the note: as quick a slide as there is.
                    7 => {
                        cell.note = (u16::from(v) + 25).clamp(u16::from(NOTE_MIN), u16::from(NOTE_MAX)) as u8;
                        (Fx::TonePorta, 0xFF)
                    }
                    8..=10 => {
                        if row > 0 && cmd != st.vib_type {
                            if let Some(prev) = p.cell_mut(row - 1, ch).filter(|c| c.fx == Fx::None) {
                                (prev.fx, prev.param) = (Fx::S3mEx, 0x30 | (cmd - 8));
                                st.vib_type = cmd;
                            }
                        }
                        (Fx::Vibrato, dmf_vibrato(v, ticks))
                    }
                    11 => (Fx::Tremor, dmf_tremor(v, ticks)),
                    12 => match dmf_delay(v, ticks) {
                        0 => {
                            cell.note = NOTE_CUT;
                            (Fx::None, 0)
                        }
                        x => (Fx::S3mEx, 0xC0 | x),
                    },
                    _ => (Fx::None, 0),
                };
            }
            if info & 0x02 != 0 {
                let (cmd, v) = (r.u8().unwrap_or(0), r.u8().unwrap_or(0));
                e3 = match cmd {
                    1 | 2 => (Fx::VolSlide, dmf_slide(v, ticks, cmd == 1)),
                    3 => (Fx::Tremor, dmf_tremor(v, ticks)),
                    4..=6 => {
                        if row > 0 && cmd != st.trem_type {
                            if let Some(prev) = p.cell_mut(row - 1, ch).filter(|c| c.fx == Fx::None) {
                                (prev.fx, prev.param) = (Fx::S3mEx, 0x40 | (cmd - 4));
                                st.trem_type = cmd;
                            }
                        }
                        (Fx::Tremolo, dmf_vibrato(v, ticks))
                    }
                    7 => (Fx::Panning, v),
                    8 | 9 => (Fx::PanningSlide, dmf_slide(v, ticks, cmd == 8)),
                    10 => (Fx::Panbrello, dmf_vibrato(v, ticks)),
                    _ => (Fx::None, 0),
                };
            }
            // A note with no instrument by it slides there, not played again.
            if slide_note && (NOTE_MIN..=NOTE_MAX).contains(&cell.note) {
                if e2.0 == Fx::None {
                    e2 = (Fx::TonePorta, 0xFF);
                } else if e3.0 == Fx::None && e2.0 != Fx::TonePorta {
                    e3 = (Fx::TonePorta, 0xFF);
                }
            }
            if let Some(v) = vol {
                (cell.vol, cell.volp) = (Vol::Volume, v);
            }
            // The note's and volume's effects, then the instrument's before them.
            two_effects(&mut cell, e2, e3);
            if e1.0 != Fx::None {
                let other = (cell.fx, cell.param);
                (cell.fx, cell.param) = (Fx::None, 0);
                two_effects(&mut cell, other, e1);
                if cell.fx != e1.0 {
                    // The instrument's effect is the one kept in the effect column.
                    if let Some((v, x)) = vol_form(cell.fx, cell.param).filter(|_| cell.vol == Vol::None) {
                        (cell.vol, cell.volp) = (v, x);
                    }
                    (cell.fx, cell.param) = e1;
                }
            }
            if let Some(c) = p.cell_mut(row, ch) {
                *c = cell;
            }
        }
        if let Some((bpm, speed)) = new_tempo {
            change = false;
            if let Some(c) = p.cell_mut(row, 0) {
                (c.fx, c.param) = (Fx::Tempo, bpm);
            }
            put_effect(&mut p, row, Fx::Speed, speed, 0);
        }
        if delay >> 4 != 0 {
            put_effect(&mut p, row, Fx::S3mEx, 0xE0 | delay >> 4, 0);
        }
        if delay & 0x0F != 0 {
            let x = (u32::from(delay & 0x0F) * u32::from(ticks) / 15).clamp(1, 15) as u8;
            put_effect(&mut p, row, Fx::S3mEx, 0x60 | x, 0);
        }
    }
    p
}

/// A node of X-Tracker's Huffman tree: its value and its children.
#[derive(Clone, Copy, Default)]
struct DmfNode {
    left: i16,
    right: i16,
    value: u8,
}

/// Reads the tree, depth first as it's written: a node's 7-bit value,
/// whether a left and a right child follow, then they do. None when the
/// bits run out.
fn dmf_tree(bits: &mut super::micro::Bits, nodes: &mut [DmfNode; 256], count: &mut usize) -> Option<()> {
    let at = *count;
    if at > 255 {
        return Some(());
    }
    nodes[at].value = bits.read(7)? as u8;
    let (left, right) = (bits.read(1)?, bits.read(1)?);
    *count += 1;
    nodes[at].left = -1;
    nodes[at].right = -1;
    if left == 1 {
        nodes[at].left = *count as i16;
        dmf_tree(bits, nodes, count)?;
    }
    if right == 1 {
        nodes[at].right = *count as i16;
        dmf_tree(bits, nodes, count)?;
    }
    Some(())
}

/// A sample packed by a Huffman tree of 7-bit values: a sign bit, then
/// the tree walked a bit a step, each value added to the last.
fn dmf_unpack(b: &[u8], len: usize) -> Vec<u8> {
    let mut bits = super::micro::Bits::new(b);
    let mut nodes = [DmfNode::default(); 256];
    let mut out = Vec::with_capacity(len);
    if dmf_tree(&mut bits, &mut nodes, &mut 0).is_none() || nodes[0].left < 0 || nodes[0].right < 0 {
        return out;
    }
    let (mut value, mut delta) = (0u8, 0u8);
    'points: for _ in 0..len {
        let Some(sign) = bits.read(1) else { break };
        let mut node = 0usize;
        loop {
            let Some(bit) = bits.read(1) else { break 'points };
            let next = if bit == 1 { nodes[node].right } else { nodes[node].left };
            if !(0..=255).contains(&next) {
                break;
            }
            node = next as usize;
            delta = nodes[node].value;
            if nodes[node].left < 0 || nodes[node].right < 0 {
                break;
            }
        }
        if sign == 1 {
            delta ^= 0xFF;
        }
        value = value.wrapping_add(delta);
        out.push(value);
    }
    out
}

pub fn load_dmf_xt(d: &[u8]) -> Result<Module> {
    let version = d[4];
    let mut r = Reader::at(d, 5);
    let tracker = r.text(8)?;
    let title = r.text(30)?;
    let artist = r.text(20)?;
    r.skip(3)?;
    let mut chunks: Vec<([u8; 4], &[u8])> = Vec::new();
    while r.left() >= 8 {
        let id = r.tag()?;
        let mut len = r.u32le()? as usize;
        // Early versions wrote the orders' chunk short of its loop, and
        // a packed sample chunk's length wrong.
        let skip = match (version, &id) {
            (3, b"SEQU") => 2,
            (4, b"SEQU") => 4,
            _ => 0,
        };
        if version < 8 && &id == b"SMPD" {
            len = r.left();
        }
        chunks.push((id, r.bytes_upto(len)));
        r.skip(skip.min(r.left()))?;
    }
    let chunk = |id: &[u8; 4]| chunks.iter().find(|c| &c.0 == id).map(|c| c.1);
    let patt = chunk(b"PATT").ok_or_else(|| crate::io::Error::Bad("it has no patterns".into()))?;
    let mut pr = Reader::new(patt);
    let npat = usize::from(pr.u16le()?);
    let nch = usize::from(pr.u8()?).clamp(1, 32) + 1;
    let head = if version < 3 { 9 } else { 8 };
    let mut pat_data = Vec::with_capacity(npat);
    for _ in 0..npat {
        let start = pr.pos;
        pr.skip(head - 4)?;
        let len = pr.u32le()? as usize;
        pr.seek(start)?;
        pat_data.push(pr.bytes(head + len)?);
    }
    let mut sr = Reader::new(chunk(b"SEQU").unwrap_or(&[]));
    let loop_start = if version >= 3 { usize::from(sr.u16le().unwrap_or(0)) } else { 0 };
    let mut loop_end = if version >= 4 { usize::from(sr.u16le().unwrap_or(0xFFFF)) } else { usize::MAX };
    if version == 4 && loop_end == 0 {
        loop_end = usize::MAX;
    }
    let seq: Vec<usize> = (0..sr.left() / 2).map(|_| sr.u16le().map(usize::from)).collect::<Result<_>>()?;
    // Each order its own pattern: the tempo goes on from one to the next.
    let mut tempo = DmfTempo { bpm_mode: false, beat: 0, ticks: 32, bpm: 120, speed: 6 };
    let mut chans = vec![DmfChannel { buffered: 0, last: 0, vib_type: 8, trem_type: 4, high_offset: 6, backwards: false }; nch];
    let mut patterns = Vec::with_capacity(seq.len());
    let mut orders = Vec::with_capacity(seq.len());
    for &s in &seq {
        if let Some(data) = pat_data.get(s) {
            orders.push(patterns.len() as u16);
            patterns.push(dmf_pattern(data, version, nch, &mut tempo, &mut chans));
        }
    }
    if !orders.is_empty() {
        let last = orders.len() - 1;
        let (ls, le) = (loop_start.min(last), loop_end.min(last));
        if ls > 0 || le < last {
            let p = &mut patterns[le];
            let row = p.rows - 1;
            put_effect(p, row.saturating_sub(1), Fx::PositionJump, ls as u8, 0);
        }
    }
    let message =
        chunk(b"CMSG").map(|m| m.get(1..).unwrap_or(&[]).chunks(40).map(|l| crate::io::message(l).trim_end().to_string()).collect::<Vec<_>>().join("\n").trim_end().to_string()).unwrap_or_default();
    let mut samples = Vec::new();
    if let Some(si) = chunk(b"SMPI") {
        let mut ir = Reader::new(si);
        let mut data = Reader::new(chunk(b"SMPD").unwrap_or(&[]));
        let n = ir.u8()?;
        for _ in 0..n {
            let nlen = if version < 2 { 30 } else { usize::from(ir.u8()?) };
            let name = ir.text(nlen)?;
            let h = ir.bytes(16)?;
            let (mut len, mut ls, mut le) = (le32(h, 0), le32(h, 4), le32(h, 8));
            let (rate, vol, flags) = (u16::from_le_bytes([h[12], h[13]]), h[14], h[15]);
            let filename = if version >= 8 { ir.text(8)? } else { String::new() };
            ir.skip(if version > 1 { 6 } else { 2 })?;
            let b16 = flags & 2 != 0;
            if b16 {
                (len, ls, le) = (len / 2, ls / 2, le / 2);
            }
            let size = data.u32le().unwrap_or(0) as usize;
            let raw = data.bytes_upto(size);
            let frames = len as usize;
            let pcm = if flags & 0x0C == 0x04 {
                let bytes = dmf_unpack(raw, frames * if b16 { 2 } else { 1 });
                decode(&bytes, if b16 { Pcm::S16le } else { Pcm::S8 }, frames)
            } else {
                decode(raw, if b16 { Pcm::S16le } else { Pcm::S8 }, frames)
            };
            samples.push(Sample {
                name,
                filename,
                wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
                c5: u32::from(rate).max(1),
                volume: if vol == 0 { 64 } else { ((u16::from(vol) + 1) / 4).min(64) as u8 },
                // Its loop plays while the note's held.
                sustain: if flags & 1 != 0 && le > ls { Loop::forward(ls, le) } else { Loop::default() },
                ..Sample::empty()
            });
        }
    }
    let mut m = Module {
        title,
        artist,
        message,
        format: "dmf".into(),
        format_name: format!("X-Tracker (version {version})"),
        tracker: if version == 10 {
            "X-Tracker 32".into()
        } else if tracker.is_empty() || tracker == "XTRACKER" {
            "X-Tracker".into()
        } else {
            format!("X-Tracker (from {tracker})")
        },
        kind: Kind::It,
        flags: Flags { linear: true, compat_gxx: true, ..Default::default() },
        speed: 6,
        tempo: 120,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels: (0..nch).map(|c| Channel { name: if c == 0 { "Tempo".into() } else { String::new() }, ..Default::default() }).collect(),
        orders,
        patterns: patterns.into_iter().map(Arc::new).collect(),
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}
