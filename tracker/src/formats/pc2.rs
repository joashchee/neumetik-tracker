//! A third lot of the PC's trackers: Dynamic Studio (.dsm), Composer 667,
//! Graoumf Tracker (.gtk), DSMI's Advanced Module Format (.amf, and the
//! compact .dmf of Webfoot's games), Epic MegaGames' MASI (.psm, new
//! and old) and Liquid Tracker (.liq, and its older NO modules).

use std::sync::Arc;

use super::common::{finetune_c5, mod_effect, s3m_effect};
use crate::io::{bad, decode, Pcm, Reader, Result};
use crate::model::*;

/// A row's number written as two decimal digits in hex (a break's).
fn bcd(p: u8) -> u8 {
    (p >> 4) * 10 + (p & 0x0F)
}

// ---- Dynamic Studio ----
//
// "DSm\x1A", version 0x20, title and artist, counts, the channels'
// panning, orders, eight-character track names for each pattern, 32-byte
// sample headers, patterns of 64 rows of four-byte cells, the samples.

pub fn probe_dsm_ds(d: &[u8]) -> bool {
    d.len() >= 64 && &d[..4] == b"DSm\x1A" && d[4] == 0x20 && (1..=16).contains(&d[45]) && d[46] > 0 && d[47] > 0 && d[49] <= 100
}

pub fn load_dsm_ds(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 5);
    let title = r.text(20)?;
    let artist = r.text(20)?;
    let nch = usize::from(r.u8()?);
    let nsmp = usize::from(r.u8()?);
    let nord = usize::from(r.u8()?);
    let _pack = r.u8()?;
    let gv = r.u8()?;
    r.seek(64)?;
    let pans = r.bytes(nch)?.to_vec();
    let orders: Vec<u16> = r.bytes(nord)?.iter().map(|&o| u16::from(o)).collect();
    let npat = usize::from(orders.iter().copied().max().unwrap_or(0)) + 1;
    let names: Vec<String> = (0..nch).map(|_| r.text(8)).collect::<Result<_>>()?;
    r.skip((npat - 1) * nch * 8)?;
    let mut heads = Vec::with_capacity(nsmp);
    for _ in 0..nsmp {
        let name = r.text(22)?;
        let kind = r.u8()?;
        let len = usize::from(r.u16le()?);
        let ft = r.u8()?;
        let vol = r.u8()?;
        let ls = u32::from(r.u16le()?);
        let ll = u32::from(r.u16le()?);
        r.skip(1)?;
        heads.push((name, kind == 16, len, ft, vol, ls, ll));
    }
    let mut patterns = Vec::with_capacity(npat);
    for _ in 0..npat {
        let data = r.bytes(64 * nch * 4)?;
        let mut p = Pattern::new(64, nch);
        for (cell, b) in p.cells.iter_mut().zip(data.chunks_exact(4)) {
            cell.instr = b[0];
            if b[1] > 0 && b[1] <= 168 {
                cell.note = (b[1] >> 1) + 36;
            }
            let (cmd, param) = (b[2], b[3]);
            (cell.fx, cell.param) = match cmd {
                0x08 => match param >> 4 {
                    // Panning in 16 steps; slides up as far as the sample's
                    // own volume (here as far as they go); fine slides
                    // over all five octaves.
                    0x0 => (Fx::ModEx, 0x80 | param),
                    0x1 => (Fx::VolSlide, param << 4),
                    0x2 => (Fx::ModEx, 0xA0 | (param & 15)),
                    0x3 | 0x4 => (Fx::ModEx, param - 0x20),
                    _ => (Fx::None, 0),
                },
                // 3D simulation: front, right, back, left, round.
                0x13 => {
                    let v = u32::from(param & 0x7F) * 2;
                    let pan = match param {
                        0..=0x40 => v + 0x80,
                        0x41..=0x7F => 0x180 - v,
                        0x80..=0xBF => 0x80u32.saturating_sub(v),
                        _ => v.saturating_sub(0x80),
                    };
                    (Fx::Panning, pan.min(255) as u8)
                }
                0x20..=0x2F => {
                    (cell.vol, cell.volp) = (Vol::Volume, (cmd & 15) * 4 + 4);
                    (Fx::Offset, param)
                }
                0x00..=0x0F | 0x11 | 0x12 => mod_effect(cmd & 0x0F, param, false),
                _ => (Fx::None, 0),
            };
        }
        patterns.push(Arc::new(p));
    }
    let mut samples = Vec::with_capacity(nsmp);
    for (name, b16, len, ft, vol, ls, ll) in heads {
        let pcm = if b16 { decode(r.bytes_upto(len * 2), Pcm::S16le, len) } else { decode(r.bytes_upto(len), Pcm::S8, len) };
        samples.push(Sample {
            name,
            wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
            c5: finetune_c5(ft),
            volume: vol.min(64),
            looped: if ll > 2 { Loop::forward(ls, ls + ll) } else { Loop::default() },
            ..Sample::empty()
        });
    }
    let mut m = Module {
        title,
        artist,
        format: "dstudio".into(),
        format_name: "Dynamic Studio".into(),
        tracker: "Dynamic Studio".into(),
        kind: Kind::Mod,
        speed: 6,
        tempo: 125,
        global_volume: (u16::from(gv) * 128 / 100) as u8,
        mix_volume: 32,
        separation: 128,
        channels: pans.iter().zip(names).map(|(&p, name)| Channel { pan: (p & 15) * 0x11, name, ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Composer 667 ----
//
// "gf", 64 instrument names, the speed, the orders' count, where each of
// 128 patterns starts, the orders, 64 OPL2 instruments of 11 bytes,
// patterns of 32 rows as a stream of commands for 18 FM channels. No
// PCM: neumetik-synth plays it all.

const C667_HEADER: usize = 2 + 64 * 8 + 2 + 256;

pub fn probe_667(d: &[u8]) -> bool {
    if d.len() < C667_HEADER + 64 * 11 || &d[..2] != b"gf" || !(1..=15).contains(&d[514]) || d[515] > 128 {
        return false;
    }
    if d[2..514].iter().any(|&c| c <= 31) {
        return false;
    }
    let offs: Vec<i32> = d[516..772].chunks_exact(2).map(|b| i32::from(u16::from_le_bytes([b[0], b[1]]))).collect();
    offs.windows(2).all(|w| w[1] > w[0])
}

pub fn load_667(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 2);
    let names: Vec<String> = (0..64).map(|_| r.text(8)).collect::<Result<_>>()?;
    let speed = r.u8()?;
    let nord = usize::from(r.u8()?);
    let offsets: Vec<usize> = (0..128).map(|_| r.u16le().map(usize::from)).collect::<Result<_>>()?;
    let orders: Vec<u16> = r.bytes(nord)?.iter().map(|&o| u16::from(o)).collect();
    if orders.iter().any(|&o| o >= 128) {
        return bad("an order points past the patterns");
    }
    let mut samples = Vec::with_capacity(64);
    for name in names {
        let b: [u8; 11] = r.bytes(11)?.try_into().expect("eleven bytes");
        // Composer 670's order: the connection, then each operator's.
        let fm = FmPatch::from_c67(&b);
        samples.push(Sample { name, fm: (!fm.is_silent()).then_some(fm), ..Sample::empty() });
    }
    let base = r.pos;
    const NCH: usize = 18;
    let (mut left, mut right) = (false, false);
    let mut patterns = Vec::with_capacity(128);
    for &off in &offsets {
        let mut pr = Reader::new(d.get(base + off..).unwrap_or(&[]));
        let mut p = Pattern::new(32, NCH);
        let mut row = 0usize;
        while let Ok(b) = pr.u8() {
            match b {
                0xFF => {
                    let skip = usize::from(pr.u8().unwrap_or(0));
                    row += skip;
                    if row >= 32 || skip == 0 {
                        break;
                    }
                }
                0xFE | 0xFD => {
                    let (Ok(ch), Ok(v)) = (pr.u8(), pr.u8()) else {
                        break;
                    };
                    if usize::from(ch) >= NCH || v > 63 {
                        return bad("a pattern names a channel or instrument that isn't there");
                    }
                    let cell = &mut p.cells[row * NCH + usize::from(ch)];
                    if b == 0xFE {
                        cell.instr = v + 1;
                    } else {
                        (cell.vol, cell.volp) = (Vol::Volume, 63 - v);
                    }
                }
                0xFC => {
                    let Ok(to) = pr.u8() else { break };
                    (p.cells[row * NCH].fx, p.cells[row * NCH].param) = (Fx::PositionJump, to);
                }
                0xFB => (p.cells[row * NCH].fx, p.cells[row * NCH].param) = (Fx::PatternBreak, 0),
                ch if usize::from(ch) < NCH => {
                    let Ok(note) = pr.u8() else { break };
                    if note >= 0x7C {
                        return bad("a pattern has a note 667 can't play");
                    }
                    p.cells[row * NCH + usize::from(ch)].note = 13 + (note & 0x0F) + (note >> 4) * 12;
                    if ch % 2 == 1 {
                        right = true;
                    } else {
                        left = true;
                    }
                }
                _ => return bad("a pattern has a command 667 doesn't write"),
            }
        }
        patterns.push(Arc::new(p));
    }
    let mut m = Module {
        format: "667".into(),
        format_name: "Composer 667".into(),
        tracker: "Composer 667".into(),
        kind: Kind::S3m,
        speed,
        tempo: 150,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels: (0..NCH)
            .map(|c| Channel {
                pan: if left && right {
                    if c % 2 == 1 {
                        255
                    } else {
                        0
                    }
                } else {
                    128
                },
                name: format!("FM {}", c + 1),
                ..Default::default()
            })
            .collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.message = m.samples.iter().map(|s| s.name.clone()).filter(|n| !n.is_empty()).collect::<Vec<_>>().join("\n");
    m.tidy();
    Ok(m)
}

// ---- Graoumf Tracker ----
//
// "GTK" and a version (1 to 4), title, a short comment, counts, sample
// headers (more in each version), 256 big-endian orders, patterns of
// four-byte cells (five from version 4, a volume after), big-endian
// samples. Its effects are 16-bit, the top byte the command.

pub fn probe_gtk(d: &[u8]) -> bool {
    if d.len() < 206 || &d[..3] != b"GTK" || !(1..=4).contains(&d[3]) {
        return false;
    }
    let be = |i: usize| u16::from_be_bytes([d[i], d[i + 1]]);
    let (nsmp, rows, nch, nord, restart) = (be(196), be(198), be(200), be(202), be(204));
    nsmp <= 255 && (1..=256).contains(&rows) && (1..=32).contains(&nch) && nord <= 256 && restart < nord
}

/// Graoumf Tracker 2's logarithmic volume (256 steps an octave, 0x800 the
/// top... 0x100 is 0 dB) as linear, 0x1000 0 dB.
pub(super) fn graoumf_log(v: u16) -> u32 {
    let v = v.min(0xFFF);
    ((2f64.powf(f64::from(v & 0xFF) / 256.0) * 32768.0).round() as u32) >> (15 - u32::from(v >> 8)).min(31)
}

/// A Graoumf Tracker 1 effect (command and parameter) in a cell.
pub(super) fn gtk_effect(cell: &mut Cell, fx: u8, p: u8) {
    if fx == 0 {
        return;
    }
    let p12 = u16::from(fx & 0x0F) << 8 | u16::from(p);
    let p4 = p.min(15);
    let slide = p.min(14);
    let div32 = ((u16::from(p) + 31) / 32).min(14) as u8;
    let (f, v) = match fx >> 4 {
        0x2 | 0x3 => {
            let v = if fx >> 4 == 2 {
                p12 / 4
            } else if p12 == 0 {
                0
            } else {
                (graoumf_log(p12) / 4).min(64) as u16
            };
            (cell.vol, cell.volp) = (Vol::Volume, v.min(64) as u8);
            return;
        }
        0x4 => (Fx::Panning, (p12 / 16) as u8),
        0x5 => (Fx::GlobalVolume, ((p12 + 1) / 32).min(128) as u8),
        0x6 => (Fx::GlobalVolume, if p12 == 0 { 0 } else { (graoumf_log(p12) / 32).min(128) as u8 }),
        // A roll: x its speed (the most times it goes, yy, isn't kept).
        0x7 => (Fx::Retrig, fx & 0x0F),
        // A roll, and a volume.
        0x8 => {
            (cell.vol, cell.volp) = (Vol::Volume, ((u16::from(p) + 3) / 4).min(64) as u8);
            (Fx::Retrig, fx & 0x0F)
        }
        // 9000 doesn't take up the last offset.
        0x9 => (Fx::Offset, p12.clamp(1, 255) as u8),
        _ => gtk_short_effect(cell, fx, p, p4, slide, div32),
    };
    if f != Fx::None {
        (cell.fx, cell.param) = (f, v);
    }
}

/// The effects with an 8-bit parameter.
fn gtk_short_effect(cell: &mut Cell, fx: u8, p: u8, p4: u8, slide: u8, div32: u8) -> (Fx, u8) {
    match fx {
        0x01..=0x04 | 0x07 | 0x0B | 0x0F => mod_effect(fx, p, false),
        0x05 => {
            if cell.vol == Vol::None {
                (cell.vol, cell.volp) = (Vol::VibratoDepth, 0);
            }
            (Fx::TonePorta, p)
        }
        0x06 => {
            if cell.vol == Vol::None {
                (cell.vol, cell.volp) = (Vol::TonePorta, 0);
            }
            (Fx::Vibrato, p)
        }
        0x09 => (Fx::S3mEx, 0xD0 | p4),
        0x0A => (Fx::KeyOff, p),
        0x0C if p <= 2 => (Fx::S3mEx, 0x30 | p),
        0x0D => (Fx::PatternBreak, p),
        0x0E if p <= 2 => (Fx::S3mEx, 0x40 | p),
        0x10 => (Fx::Arpeggio, p),
        0x11 => (Fx::PortaUp, 0xF0 | slide),
        0x12 => (Fx::PortaDown, 0xF0 | slide),
        0x13 => (Fx::Retrig, p),
        0x14 => (Fx::VolSlide, slide << 4),
        0x15 => (Fx::VolSlide, slide),
        0x18 => (Fx::TonePortaVol, slide << 4),
        0x19 => (Fx::TonePortaVol, slide),
        0x1C => (Fx::VibratoVol, slide << 4),
        0x1D => (Fx::VibratoVol, slide),
        0xA0 if p > 0 => (Fx::GlobalVolSlide, div32 << 4),
        0xA1 if p > 0 => (Fx::GlobalVolSlide, div32),
        0xA4 => (Fx::VolSlide, slide << 4 | 0x0F),
        0xA5 => (Fx::VolSlide, slide | 0xF0),
        0xA6 if p > 0 => (Fx::GlobalVolSlide, div32 << 4 | 0x0F),
        0xA7 if p > 0 => (Fx::GlobalVolSlide, div32 | 0xF0),
        0xA8 if p > 0 => (Fx::Speed, p),
        0xAA => (Fx::S3mEx, 0xE0 | p4),
        0xAC => (Fx::PortaUp, 0xE0 | p4),
        0xAD => (Fx::PortaDown, 0xE0 | p4),
        0xAE => (Fx::PanningSlide, (slide / 2) << 4),
        0xAF => (Fx::PanningSlide, slide / 2),
        _ => (Fx::None, 0),
    }
}

pub fn load_gtk(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    r.skip(3)?;
    let version = r.u8()?;
    let title = r.text(32)?;
    let comment = crate::io::message(r.bytes(160)?);
    let nsmp = usize::from(r.u16be()?);
    let rows = usize::from(r.u16be()?);
    let nch = usize::from(r.u16be()?);
    let nord = usize::from(r.u16be()?);
    let restart = r.u16be()?;
    let mut heads = Vec::with_capacity(nsmp);
    for _ in 0..nsmp {
        let name = r.text(if version == 1 { 32 } else { 28 })?;
        let mut pan = None;
        if version >= 3 {
            r.skip(14)?;
            let p = r.i16be()?;
            if p >= 0 {
                pan = Some((u32::from(p as u16) * 255 / 4095).min(255) as u8);
            }
        }
        let (mut bytes, mut c5) = (1, 8363);
        if version >= 2 {
            bytes = r.u16be()?;
            c5 = u32::from(r.u16be()?);
        }
        let len = r.u32be()?;
        let ls = r.u32be()?;
        let ll = r.u32be()?;
        let vol = r.u16be()?;
        let ft = r.i16be()?;
        heads.push((name, pan, bytes == 2, c5, len, ls, ll, vol, ft));
    }
    let orders: Vec<u16> = r.bytes(512)?.chunks_exact(2).take(nord).map(|b| u16::from_be_bytes([b[0], b[1]])).collect();
    let npat = usize::from(orders.iter().copied().max().unwrap_or(0)) + 1;
    let size = if version < 4 { 4 } else { 5 };
    let mut patterns = Vec::with_capacity(npat);
    for _ in 0..npat {
        let data = r.bytes(rows * nch * size)?;
        let mut p = Pattern::new(rows, nch);
        for (cell, b) in p.cells.iter_mut().zip(data.chunks_exact(size)) {
            if (24..84).contains(&b[0]) {
                cell.note = b[0] + 13;
            }
            cell.instr = b[1];
            gtk_effect(cell, b[2], b[3]);
            if size == 5 && b[4] != 0 {
                (cell.vol, cell.volp) = (Vol::Volume, ((u16::from(b[4]) + 1) / 4).min(64) as u8);
            }
        }
        patterns.push(Arc::new(p));
    }
    let mut samples = Vec::with_capacity(nsmp);
    for (name, pan, b16, c5, len, ls, ll, vol, ft) in heads {
        let div = if b16 { 2 } else { 1 };
        let frames = len as usize / div;
        let pcm = if b16 { decode(r.bytes_upto(frames * 2), Pcm::S16be, frames) } else { decode(r.bytes_upto(frames), Pcm::S8, frames) };
        samples.push(Sample {
            name,
            wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
            // Its finetune in eighths of a semitone, on its own rate.
            c5: (f64::from(if c5 == 0 { 8363 } else { c5 }) * 2f64.powf(f64::from(ft) / 96.0)).round().clamp(1.0, 2_000_000.0) as u32,
            volume: (vol / 4).min(64) as u8,
            pan,
            looped: if ls > 0 || ll > 2 { Loop::forward(ls / div as u32, ls.saturating_add(ll) / div as u32) } else { Loop::default() },
            ..Sample::empty()
        });
    }
    let mut m = Module {
        title,
        message: comment,
        format: "gtk".into(),
        format_name: "Graoumf Tracker".into(),
        tracker: format!("Graoumf Tracker (version {version})"),
        kind: Kind::It,
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        restart,
        channels: (0..nch).map(|c| Channel { pan: if matches!(c % 4, 0 | 3) { 64 } else { 192 }, ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- DSMI Advanced Module Format ----
//
// "AMF" and a version (1, 8 to 14), or "DMF" (Webfoot's compact form,
// with no names and delta samples). Each order is a pattern of its own,
// a track for each channel; tracks, shared between them, are lists of
// three-byte events (row, command, value).

pub fn probe_amf(d: &[u8]) -> bool {
    if d.len() < 4 {
        return false;
    }
    let v = d[3];
    let at = match &d[..3] {
        b"AMF" if v == 1 || (8..=14).contains(&v) => 36,
        b"DMF" if (10..=14).contains(&v) => 4,
        _ => return false,
    };
    let Some(h) = d.get(at..at + 5) else {
        return false;
    };
    if h[0] == 0 || h[1] == 0 || u16::from_le_bytes([h[2], h[3]]) == 0 {
        return false;
    }
    match v {
        ..=8 => true,
        9..=11 => (1..=16).contains(&h[4]),
        _ => (1..=32).contains(&h[4]),
    }
}

/// An event's effect (command 0x81 and up) in a cell.
fn amf_effect(cell: &mut Cell, cmd: u8, value: u8) {
    let neg = |v: u8| (v as i8).unsigned_abs();
    let (fx, p) = match cmd & 0x7F {
        0x01 => (Fx::Speed, value),
        0x02 | 0x0A | 0x0B => {
            let p = if value & 0x80 != 0 { neg(value) & 0x0F } else { (value & 0x0F) << 4 };
            ([Fx::VolSlide, Fx::TonePortaVol, Fx::VibratoVol][usize::from(cmd & 0x7F == 0x0A) + 2 * usize::from(cmd & 0x7F == 0x0B)], p)
        }
        0x03 => {
            if matches!(cell.vol, Vol::None | Vol::Volume) {
                (cell.vol, cell.volp) = (Vol::Volume, value.min(64));
                return;
            }
            (Fx::Volume, value.min(64))
        }
        0x04 if value & 0x80 != 0 => (Fx::PortaUp, neg(value) & 0x7F),
        0x04 => (Fx::PortaDown, value),
        0x06 => (Fx::TonePorta, value),
        0x07 => (Fx::Tremor, value),
        0x08 => (Fx::Arpeggio, value),
        0x09 => (Fx::Vibrato, value),
        0x0C => (Fx::PatternBreak, value),
        0x0D => (Fx::PositionJump, value),
        0x0F => (Fx::Retrig, value),
        0x10 => (Fx::Offset, value),
        0x11 if value == 0 => return,
        0x11 if value & 0x80 != 0 => (Fx::VolSlide, 0xF0 | (neg(value) & 0x0F)),
        0x11 => (Fx::VolSlide, 0x0F | (value & 0x0F) << 4),
        // Fine and extra-fine slides, up when negative.
        0x12 | 0x16 if value == 0 => return,
        0x12 | 0x16 => {
            let fine = if cmd & 0x7F == 0x16 { 0xE0 } else { 0xF0 };
            if value & 0x80 != 0 {
                (Fx::PortaUp, fine | (neg(value) & 0x0F))
            } else {
                (Fx::PortaDown, fine | (value & 0x0F))
            }
        }
        0x13 => (Fx::S3mEx, 0xD0 | (value & 0x0F)),
        0x14 => (Fx::S3mEx, 0xC0 | (value & 0x0F)),
        0x15 => (Fx::Tempo, value),
        // Panning, -64 to 64 (100 surround); into the volume column when
        // the effect's taken.
        0x17 if value == 100 => (Fx::S3mEx, 0x91),
        0x17 => {
            let pan = ((value as i8).clamp(-64, 64) + 64) as u8;
            if cell.fx != Fx::None {
                if matches!(cell.vol, Vol::None | Vol::Panning) {
                    (cell.vol, cell.volp) = (Vol::Panning, pan / 2);
                }
                return;
            }
            s3m_effect(24, pan, false)
        }
        _ => return,
    };
    (cell.fx, cell.param) = (fx, p);
}

pub fn load_amf(d: &[u8]) -> Result<Module> {
    let dmf = &d[..3] == b"DMF";
    let version = d[3];
    let mut r = Reader::at(d, 4);
    let title = if dmf { String::new() } else { r.text(32)? };
    let nsmp = usize::from(r.u8()?);
    let nord = usize::from(r.u8()?);
    let ntrk = usize::from(r.u16le()?);
    let nch = if version >= 9 { usize::from(r.u8()?) } else { 4 };
    let mut channels: Vec<Channel> = (0..nch).map(|c| Channel { pan: if c & 1 != 0 { 0xC0 } else { 0x40 }, ..Default::default() }).collect();
    if version >= 11 {
        let stored = if version >= 12 { 32 } else { 16 };
        let pans = r.bytes(stored.max(nch))?;
        for (c, &p) in channels.iter_mut().zip(pans) {
            if p == 100 {
                c.surround = true;
            } else {
                c.pan = ((i32::from(p as i8) + 64) * 2).clamp(0, 255) as u8;
            }
        }
    } else if version >= 9 {
        r.skip(16)?;
    }
    let (mut tempo, mut speed) = (125u16, 6u8);
    if version >= 13 {
        let t = r.u8()?;
        tempo = if t < 32 { 125 } else { u16::from(t) };
        speed = r.u8()?;
    }
    let mut lengths = vec![64usize; nord];
    let mut track_refs = Vec::with_capacity(nord);
    for len in lengths.iter_mut() {
        if version >= 14 {
            *len = usize::from(r.u16le()?);
        }
        track_refs.push((0..nch).map(|_| r.u16le().map(usize::from)).collect::<Result<Vec<_>>>()?);
    }
    // DMP 2.32's converter wrote new headers at the old headers' length.
    let mut truncated = false;
    if version == 10 {
        let mut t = r.clone();
        for _ in 0..nsmp {
            let Ok(h) = t.bytes(65) else { break };
            let le32 = |i: usize| u32::from_le_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
            let (len, ls, le) = (le32(50), le32(57), le32(61));
            if !(h[0] <= 1 && le32(46) as usize <= nsmp && len <= 0x10_0000 && h[56] <= 64 && ls <= len && le <= len) {
                truncated = true;
                break;
            }
        }
    }
    let mut heads = Vec::with_capacity(nsmp);
    for _ in 0..nsmp {
        let kind = r.u8()?;
        let (name, filename) = if dmf { (String::new(), crate::io::text(r.bytes(1)?)) } else { (r.text(32)?, r.text(13)?) };
        let index = r.u32le()? as usize;
        let (len, rate, vol, ls, le) = if version < 10 {
            let (len, rate, vol, ls, le) = (u32::from(r.u16le()?), r.u16le()?, r.u8()?, u32::from(r.u16le()?), u32::from(r.u16le()?));
            (len, rate, vol, ls, if le == 0xFFFF { 0 } else { le })
        } else {
            let (len, rate, vol, ls) = (r.u32le()?, r.u16le()?, r.u8()?, r.u32le()?);
            // The compact header's loop end is three bytes; a truncated
            // header has none (a loop goes to the end).
            let le = if dmf {
                r.u24le()?
            } else if truncated {
                if ls > 0 {
                    len
                } else {
                    0
                }
            } else {
                r.u32le()?
            };
            (len, rate, vol, ls, le)
        };
        let looped = if kind != 0 && le > ls.saturating_add(2) && le <= len { Loop::forward(ls, le) } else { Loop::default() };
        heads.push((Sample { name, filename, c5: u32::from(rate), volume: vol.min(64), looped, ..Sample::empty() }, index, len as usize));
    }
    let map: Vec<usize> = (0..ntrk).map(|_| r.u16le().map(usize::from)).collect::<Result<_>>()?;
    let count = map.iter().copied().max().unwrap_or(0);
    let mut tracks = Vec::with_capacity(count);
    for _ in 0..count {
        let n = usize::from(r.u16le()?);
        r.skip(1)?;
        tracks.push(r.bytes_upto(n * 3 + if version == 1 { 3 } else { 0 }));
    }
    // The samples' data in the order their indexes say, shared by every
    // sample with the same index.
    let mut waves: Vec<Option<Arc<Wave>>> = vec![None; nsmp];
    for i in 1..=nsmp {
        if r.eof() {
            break;
        }
        let start = r.pos;
        for (t, (_, index, len)) in heads.iter().enumerate() {
            if *index != i {
                continue;
            }
            r.pos = start;
            let b = r.bytes_upto(*len);
            let pcm = if dmf {
                // Unsigned deltas.
                let mut acc = 0u8;
                b.iter()
                    .map(|&v| {
                        acc = acc.wrapping_add(v);
                        i16::from((acc ^ 0x80) as i8) << 8
                    })
                    .collect()
            } else {
                decode(b, Pcm::U8, *len)
            };
            waves[t] = Some(Arc::new(Wave::new(8, 1, pcm)));
        }
    }
    let samples: Vec<Sample> = heads.into_iter().zip(waves).map(|((s, _, _), w)| Sample { wave: w.unwrap_or_default(), ..s }).collect();
    let mut patterns = Vec::with_capacity(nord);
    for (o, refs) in track_refs.iter().enumerate() {
        let rows = lengths[o].clamp(1, 256);
        let mut p = Pattern::new(rows, nch);
        for (ch, &t) in refs.iter().enumerate() {
            if t == 0 || t > ntrk {
                continue;
            }
            let real = map[t - 1];
            if real == 0 || real > count {
                continue;
            }
            for e in tracks[real - 1].chunks_exact(3) {
                let (row, cmd, value) = (usize::from(e[0]), e[1], e[2]);
                if row >= rows {
                    break;
                }
                let cell = &mut p.cells[row * nch + ch];
                match cmd {
                    0 if value == 0 => cell.note = NOTE_CUT,
                    0x00..=0x7E => {
                        cell.note = (cmd + 1).min(NOTE_MAX);
                        if value != 0xFF {
                            (cell.vol, cell.volp) = (Vol::Volume, value.min(64));
                        }
                    }
                    0x7F => {}
                    0x80 => cell.instr = value.saturating_add(1),
                    _ => amf_effect(cell, cmd, value),
                }
            }
        }
        patterns.push(Arc::new(p));
    }
    let mut m = Module {
        title,
        format: "dsmi".into(),
        format_name: if dmf { "DSMI Advanced Module Format (compact)".into() } else { "DSMI Advanced Module Format".into() },
        tracker: format!("DSMI (AMF version {version})"),
        kind: Kind::S3m,
        speed,
        tempo,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels,
        orders: (0..nord as u16).collect(),
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Epic MegaGames MASI ----
//
// The new PSM: "PSM ", a size, "FILE", then RIFF-like chunks: TITL,
// SDFT ("MAINSONG"), a PBOD for each pattern, a SONG for each subsong
// (its order list and channel settings as a little program, OPLH) and
// a DSMP for each sample (8-bit deltas). Sinaria's files differ a
// little (longer pattern names, plainer effects). The old PSM16:
// "PSM\xFE", a header pointing at its parts, each tagged.

pub fn probe_psm(d: &[u8]) -> bool {
    d.len() >= 12 && &d[..4] == b"PSM " && &d[8..12] == b"FILE"
}

fn psm_chunks(b: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut r = Reader::new(b);
    let mut out = Vec::new();
    while r.left() >= 8 {
        let (Ok(id), Ok(len)) = (r.tag(), r.u32le()) else {
            break;
        };
        out.push((id, r.bytes_upto(len as usize)));
    }
    out
}

/// A pattern's name: "P0  ", "P13 " (or Sinaria's "PATT0   "), as its number.
fn psm_pattern(r: &mut Reader, sinaria: &mut bool) -> Option<usize> {
    let mut id = r.bytes(4).ok()?;
    if id == b"PATT" {
        id = r.bytes(4).ok()?;
        *sinaria = true;
    } else {
        id = &id[1..];
    }
    let digits: String = id.iter().take_while(|c| c.is_ascii_digit()).map(|&c| c as char).collect();
    digits.parse().ok()
}

pub fn load_psm(d: &[u8]) -> Result<Module> {
    let chunks = psm_chunks(&d[12..]);
    if !chunks.iter().any(|(id, b)| id == b"SDFT" && b.starts_with(b"MAINSONG")) {
        return bad("it has no MAINSONG");
    }
    let mut nch = 0usize;
    for (_, b) in chunks.iter().filter(|(id, _)| id == b"SONG") {
        if b.len() < 11 || b[9] != 1 {
            return bad("its song is packed");
        }
        nch = nch.max(usize::from(b[10]));
    }
    if nch == 0 {
        return bad("it has no channels");
    }
    let nch = nch.min(64);
    let mut sinaria = false;
    let mut sequences = Vec::new();
    // Each channel's panning, surround and volume, the first song's kept.
    let mut setup: Option<Vec<(u8, bool, u8)>> = None;
    let (mut speed, mut tempo) = (6u8, 125u16);
    for (_, b) in chunks.iter().filter(|(id, _)| id == b"SONG") {
        let name = crate::io::text(&b[..9]);
        let mut chans = vec![(128u8, false, 64u8); nch];
        let set_pan = |chans: &mut Vec<(u8, bool, u8)>, ch: u8, kind: u8, pan: Option<u8>| {
            let Some(c) = chans.get_mut(usize::from(ch)) else {
                return;
            };
            match kind {
                0 => {
                    if let Some(p) = pan {
                        c.0 = p ^ 128;
                    }
                    c.1 = false;
                }
                2 => (c.0, c.1) = (128, true),
                4 => (c.0, c.1) = (128, false),
                _ => {}
            }
        };
        let (mut orders, mut restart) = (Vec::new(), 0u16);
        for (id, sb) in psm_chunks(&b[11..]) {
            match &id {
                b"OPLH" if sb.len() >= 9 => {
                    let mut r = Reader::at(sb, 2);
                    let (mut count, mut first) = (0u16, None);
                    while let Ok(op) = r.u8() {
                        match op {
                            0 => break,
                            0x01 => {
                                let pat = psm_pattern(&mut r, &mut sinaria).unwrap_or(0xFF);
                                orders.push(match pat {
                                    0xFF => ORDER_END,
                                    0xFE => ORDER_SKIP,
                                    p => p.min(usize::from(ORDER_SKIP) - 1) as u16,
                                });
                                first.get_or_insert(count);
                            }
                            0x02 => r.skip(4)?,
                            0x03 | 0x04 => {
                                let to = r.u16le()?;
                                if let Some(f) = first.filter(|&f| to >= f) {
                                    restart = to - f;
                                }
                                if op == 0x03 {
                                    r.skip(1)?;
                                }
                            }
                            0x05 => {
                                let (ch, kind) = (r.u8()?, r.u8()?);
                                set_pan(&mut chans, ch, kind, None);
                            }
                            0x06 => r.skip(1)?,
                            0x07 => speed = r.u8()?,
                            0x08 => tempo = u16::from(r.u8()?),
                            0x0C => r.skip(6)?,
                            0x0D => {
                                let (ch, pan, kind) = (r.u8()?, r.u8()?, r.u8()?);
                                set_pan(&mut chans, ch, kind, Some(pan));
                            }
                            0x0E => {
                                let (ch, vol) = (r.u8()?, r.u8()?);
                                if let Some(c) = chans.get_mut(usize::from(ch)) {
                                    c.2 = (vol / 4 + 1).min(64);
                                }
                            }
                            _ => return bad("its song has a command MASI doesn't know"),
                        }
                        count += 1;
                    }
                }
                b"PPAN" => {
                    for (ch, p) in sb.chunks_exact(2).take(nch).enumerate() {
                        set_pan(&mut chans, ch as u8, p[0], Some(p[1]));
                    }
                }
                _ => {}
            }
        }
        if !orders.is_empty() {
            setup.get_or_insert(chans);
            sequences.push(Sequence { name, orders, restart });
        }
    }
    let Some(setup) = setup else {
        return bad("it has no order list");
    };
    let mut samples: Vec<Sample> = Vec::new();
    for (_, b) in chunks.iter().filter(|(id, _)| id == b"DSMP") {
        if b.len() < 96 {
            continue;
        }
        let le32 = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        // Sinaria's sample IDs are four bytes longer, its rate shorter.
        let x = if sinaria { 4 } else { 0 };
        let number = usize::from(u16::from_le_bytes([b[52 + x], b[53 + x]]));
        let (len, ls, le) = (le32(54 + x) as usize, le32(58 + x), le32(62 + x));
        let vol = b[if sinaria { 73 } else { 68 }];
        let c5 = if sinaria { u32::from(u16::from_le_bytes([b[78], b[79]])) } else { le32(73) };
        let pcm = decode(&b[96..], Pcm::D8, len);
        let frames = pcm.len() as u32;
        // The loop's end is its last frame (but not in Sinaria's).
        let le = if sinaria {
            le
        } else if le != 0 {
            le.saturating_add(1)
        } else {
            0
        }
        .min(frames);
        let s = Sample {
            name: crate::io::text(&b[13 + x..46 + x]),
            filename: crate::io::text(&b[1..9]),
            wave: Arc::new(Wave::new(8, 1, pcm)),
            c5: c5.clamp(1, 2_000_000),
            volume: ((u16::from(vol) + 1) / 2).min(64) as u8,
            looped: if b[0] & 0x80 != 0 { Loop::forward(ls.min(le), le) } else { Loop::default() },
            ..Sample::empty()
        };
        if number < 255 {
            if samples.len() <= number {
                samples.resize(number + 1, Sample::empty());
            }
            samples[number] = s;
        }
    }
    let mut patterns: Vec<Arc<Pattern>> = Vec::new();
    for (_, b) in chunks.iter().filter(|(id, _)| id == b"PBOD") {
        let mut r = Reader::new(b);
        if b.len() < 8 || r.u32le()? as usize != b.len() {
            continue;
        }
        let Some(num) = psm_pattern(&mut r, &mut sinaria).filter(|&n| n < 1024) else {
            continue;
        };
        let rows = usize::from(r.u16le()?).clamp(1, 256);
        let mut p = Pattern::new(rows, nch);
        for row in 0..rows {
            let Ok(size) = r.u16le() else { break };
            if size <= 2 {
                continue;
            }
            let mut rr = Reader::new(r.bytes_upto(usize::from(size) - 2));
            while rr.left() >= 3 {
                let (flags, ch) = (rr.u8()?, rr.u8()?);
                let cell = &mut p.cells[row * nch + usize::from(ch).min(nch - 1)];
                if flags & 0x80 != 0 {
                    let n = rr.u8()?;
                    cell.note = if sinaria {
                        if n < 85 {
                            n + 36
                        } else {
                            0
                        }
                    } else if n == 0xFF {
                        NOTE_CUT
                    } else if n < 129 {
                        ((n & 0x0F) + 12 * (n >> 4) + 13).min(NOTE_MAX)
                    } else {
                        0
                    };
                }
                if flags & 0x40 != 0 {
                    cell.instr = rr.u8()?.saturating_add(1);
                }
                if flags & 0x20 != 0 {
                    (cell.vol, cell.volp) = (Vol::Volume, (rr.u8()?.min(127) + 1) / 2);
                }
                if flags & 0x10 != 0 {
                    let (cmd, param) = (rr.u8()?, rr.u8()?);
                    (cell.fx, cell.param) = psm_effect(&mut rr, cmd, param, sinaria);
                }
            }
        }
        if patterns.len() <= num {
            patterns.resize(num + 1, Arc::new(Pattern::new(64, nch)));
        }
        patterns[num] = Arc::new(p);
    }
    let title = chunks.iter().find(|(id, _)| id == b"TITL").map(|(_, b)| crate::io::text(b)).unwrap_or_default();
    let first = sequences[0].clone();
    let mut m = Module {
        title,
        format: "psm".into(),
        format_name: "Epic MegaGames MASI".into(),
        tracker: if sinaria { "MASI (new, Sinaria's)".into() } else { "MASI (new)".into() },
        kind: Kind::S3m,
        flags: Flags { old_effects: true, compat_gxx: true, ..Default::default() },
        speed,
        tempo,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        restart: first.restart,
        channels: setup.iter().map(|&(pan, surround, volume)| Channel { pan, surround, volume, ..Default::default() }).collect(),
        orders: first.orders,
        patterns,
        samples,
        ..Default::default()
    };
    if sequences.len() > 1 {
        m.sequences = sequences;
    }
    m.tidy();
    Ok(m)
}

/// A new PSM effect, in S3M's terms.
fn psm_effect(r: &mut Reader, cmd: u8, p: u8, sinaria: bool) -> (Fx, u8) {
    let porta = |p: u8| {
        if sinaria {
            p
        } else if p < 4 {
            p | 0xF0
        } else {
            p >> 2
        }
    };
    match cmd {
        0x01 => (Fx::VolSlide, if sinaria { p << 4 | 0x0F } else { (p & 0x1E) << 3 | 0x0F }),
        0x02 => (Fx::VolSlide, if sinaria { p << 4 } else { (p << 3) & 0xF0 }),
        0x03 => (Fx::VolSlide, if sinaria { p | 0xF0 } else { 0xF0 | (p >> 1) }),
        0x04 => (
            Fx::VolSlide,
            if sinaria {
                p & 0x0F
            } else if p < 2 {
                p | 0xF0
            } else {
                (p >> 1) & 0x0F
            },
        ),
        0x0B => (Fx::PortaUp, 0xF0 | porta(p)),
        0x0C => (Fx::PortaUp, porta(p)),
        0x0D => (Fx::PortaDown, 0xF0 | porta(p)),
        0x0E => (Fx::PortaDown, porta(p)),
        0x0F => (Fx::TonePorta, if sinaria { p } else { p >> 2 }),
        0x10 => (Fx::TonePortaVol, p & 0xF0),
        0x11 => (Fx::S3mEx, 0x10 | (p & 1)),
        0x12 => (Fx::TonePortaVol, (p >> 4) & 0x0F),
        0x15 => (Fx::Vibrato, p),
        0x16 => (Fx::S3mEx, 0x30 | (p & 0x0F)),
        0x17 => (Fx::VibratoVol, 0xF0 | p),
        0x18 => (Fx::VibratoVol, p),
        0x1F => (Fx::Tremolo, p),
        0x20 => (Fx::S3mEx, 0x40 | (p & 0x0F)),
        // A three-byte offset: its middle byte.
        0x29 => {
            let mid = r.u8().unwrap_or(0);
            let _ = r.u8();
            (Fx::Offset, mid)
        }
        0x2A => (Fx::Retrig, p),
        0x2B => (Fx::S3mEx, 0xC0 | (p & 0x0F)),
        0x2C => (Fx::S3mEx, 0xD0 | (p & 0x0F)),
        0x33 => {
            let _ = r.u8();
            (Fx::PositionJump, p / 2)
        }
        // MASI breaks to the first row whatever the parameter says.
        0x34 => (Fx::PatternBreak, 0),
        0x35 => (Fx::S3mEx, 0xB0 | (p & 0x0F)),
        0x36 => (Fx::S3mEx, 0xE0 | (p & 0x0F)),
        0x3D => (Fx::Speed, p),
        0x3E => (Fx::Tempo, p),
        0x47 => (Fx::Arpeggio, p),
        0x48 => (Fx::S3mEx, 0x20 | (p & 0x0F)),
        0x49 => (Fx::S3mEx, 0x80 | (p & 0x0F)),
        _ => (Fx::None, 0),
    }
}

pub fn probe_psm16(d: &[u8]) -> bool {
    d.len() >= 146 && &d[..4] == b"PSM\xFE" && d[63] == 0x1A && matches!(d[65], 0x10 | 0x01) && d[66] == 0 && d[64] & 3 == 0 && {
        let (play, real) = (u16::from_le_bytes([d[78], d[79]]), u16::from_le_bytes([d[80], d[81]]));
        play <= 32 && real <= 32 && play.max(real) > 0
    }
}

pub fn load_psm16(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 4);
    let title = r.text(59)?;
    r.skip(4)?;
    let speed = r.u8()?;
    let tempo = r.u8()?;
    let master = r.u8()?;
    let _len = r.u16le()?;
    let nord = usize::from(r.u16le()?);
    let npat = usize::from(r.u16le()?);
    let nsmp = usize::from(r.u16le()?);
    let nch = usize::from(r.u16le()?.max(r.u16le()?)).clamp(1, 32);
    let mut offsets = [0usize; 5];
    for o in &mut offsets {
        *o = r.u32le()? as usize;
    }
    let [ord_at, pan_at, pat_at, smp_at, msg_at] = offsets;
    let tagged = |at: usize, tag: &[u8]| (at > 4 && d.get(at - 4..at) == Some(tag)).then_some(at);
    let orders: Vec<u16> = tagged(ord_at, b"PORD").and_then(|at| d.get(at..at + nord)).map(|b| b.iter().map(|&o| u16::from(o)).collect()).unwrap_or_default();
    let mut channels: Vec<Channel> = (0..nch).map(|_| Channel::default()).collect();
    if let Some(b) = tagged(pan_at, b"PPAN").and_then(|at| d.get(at..at + nch)) {
        // 15 is left, 0 right.
        for (c, &p) in channels.iter_mut().zip(b) {
            c.pan = ((u32::from(15 - (p & 15)) * 256 + 8) / 15).min(255) as u8;
        }
    }
    let mut samples: Vec<Sample> = Vec::new();
    if let Some(at) = tagged(smp_at, b"PSAH") {
        let mut h = Reader::at(d, at);
        for _ in 0..nsmp {
            let Ok(b) = h.bytes(64) else { break };
            let le32 = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
            let number = usize::from(u16::from_le_bytes([b[45], b[46]]));
            let flags = b[47];
            let (off, mut len, ls, le) = (le32(37) as usize, le32(48) as usize, le32(52), le32(56));
            let (ft, vol, c2) = (b[60], b[61], u16::from_le_bytes([b[62], b[63]]));
            if number == 0 || number > 255 || samples.get(number - 1).is_some_and(Sample::sounds) {
                continue;
            }
            let b16 = flags & 0x04 != 0;
            if b16 {
                len /= 2;
            }
            let pcm_kind = if flags & 0x08 != 0 {
                if b16 {
                    Pcm::U16le
                } else {
                    Pcm::U8
                }
            } else if flags & 0x10 != 0 || flags & 0x7F == 0 {
                if b16 {
                    Pcm::D16le
                } else {
                    Pcm::D8
                }
            } else if b16 {
                Pcm::S16le
            } else {
                Pcm::S8
            };
            let pcm = decode(d.get(off..).unwrap_or(&[]), pcm_kind, len);
            // Finetune (low nibble) and transpose (high, 7 the middle) on
            // top of its rate.
            let shift = f64::from(i32::from(ft ^ 0x08) - 0x78) / (12.0 * 16.0);
            let kind = if flags & 0x20 != 0 { LoopKind::PingPong } else { LoopKind::Forward };
            let s = Sample {
                name: crate::io::text(&b[13..37]),
                filename: crate::io::text(&b[..13]),
                wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
                c5: (f64::from(c2) * 2f64.powf(shift)).round().clamp(1.0, 2_000_000.0) as u32,
                volume: vol.min(64),
                looped: if flags & 0x80 != 0 { Loop { kind, start: ls, end: le } } else { Loop::default() },
                ..Sample::empty()
            };
            if samples.len() < number {
                samples.resize(number, Sample::empty());
            }
            samples[number - 1] = s;
        }
    }
    let mut patterns = Vec::with_capacity(npat);
    if let Some(at) = tagged(pat_at, b"PPAT") {
        let mut pr = Reader::at(d, at);
        for _ in 0..npat {
            let Ok(size) = pr.u16le() else { break };
            let (Ok(rows), Ok(_)) = (pr.u8(), pr.u8()) else {
                break;
            };
            if size < 4 {
                continue;
            }
            // Padded to sixteen bytes.
            let body = pr.bytes_upto(((usize::from(size) + 15) & !15) - 4);
            let rows = usize::from(rows).clamp(1, 64);
            let mut p = Pattern::new(rows, nch);
            let mut r = Reader::new(body);
            let mut row = 0;
            while row < rows {
                let Ok(f) = r.u8() else { break };
                if f == 0 {
                    row += 1;
                    continue;
                }
                let cell = &mut p.cells[row * nch + usize::from(f & 0x1F).min(nch - 1)];
                if f & 0x80 != 0 {
                    let (Ok(n), Ok(i)) = (r.u8(), r.u8()) else {
                        break;
                    };
                    cell.note = if n > 0 { n.saturating_add(36).min(NOTE_MAX) } else { 0 };
                    cell.instr = i;
                }
                if f & 0x40 != 0 {
                    let Ok(v) = r.u8() else { break };
                    (cell.vol, cell.volp) = (Vol::Volume, v.min(64));
                }
                if f & 0x20 != 0 {
                    let (Ok(cmd), Ok(p)) = (r.u8(), r.u8()) else {
                        break;
                    };
                    (cell.fx, cell.param) = match cmd {
                        0x01 => (Fx::VolSlide, p << 4 | 0x0F),
                        0x02 => (Fx::VolSlide, p << 4),
                        0x03 => (Fx::VolSlide, 0xF0 | p),
                        0x04 => (Fx::VolSlide, p & 0x0F),
                        0x0A => (Fx::PortaUp, 0xF0 | p),
                        0x0B => (Fx::PortaUp, p),
                        0x0C => (Fx::PortaDown, 0xF0 | p),
                        0x0D => (Fx::PortaDown, p),
                        0x0E => (Fx::TonePorta, p),
                        0x0F => (Fx::S3mEx, 0x10 | (p & 0x0F)),
                        0x10 => (Fx::TonePortaVol, p << 4),
                        0x11 => (Fx::TonePortaVol, p & 0x0F),
                        0x14 => (Fx::Vibrato, p),
                        0x15 => (Fx::S3mEx, 0x30 | (p & 0x0F)),
                        0x16 => (Fx::VibratoVol, p << 4),
                        0x17 => (Fx::VibratoVol, p & 0x0F),
                        0x1E => (Fx::Tremolo, p),
                        0x1F => (Fx::S3mEx, 0x40 | (p & 0x0F)),
                        0x28 => {
                            let mid = r.u8().unwrap_or(0);
                            let _ = r.u8();
                            (Fx::Offset, mid)
                        }
                        0x29 => (Fx::Retrig, p & 0x0F),
                        0x2A => (Fx::S3mEx, 0xC0 | p),
                        0x2B => (Fx::S3mEx, 0xD0 | (p & 0x0F)),
                        0x32 => (Fx::PositionJump, p),
                        0x33 => (Fx::PatternBreak, p),
                        0x34 => (Fx::S3mEx, 0xB0 | (p & 0x0F)),
                        0x35 => (Fx::S3mEx, 0xE0 | (p & 0x0F)),
                        0x3C => (Fx::Speed, p),
                        0x3D => (Fx::Tempo, p),
                        0x46 => (Fx::Arpeggio, p),
                        0x47 => (Fx::S3mEx, 0x20 | (p & 0x0F)),
                        0x48 => (Fx::S3mEx, 0x80 | (p & 0x0F)),
                        _ => (Fx::None, 0),
                    };
                }
            }
            patterns.push(Arc::new(p));
        }
    }
    let message = if msg_at != 0 {
        let mut r = Reader::at(d, msg_at.min(d.len()));
        r.u16le().map(|n| crate::io::message(r.bytes_upto(usize::from(n)))).unwrap_or_default()
    } else {
        String::new()
    };
    let mut m = Module {
        title,
        message,
        format: "psm".into(),
        format_name: "Epic MegaGames MASI".into(),
        tracker: "MASI (old, PSM16)".into(),
        kind: Kind::S3m,
        speed,
        tempo: u16::from(tempo),
        global_volume: 128,
        mix_volume: if master == 255 { 48 } else { master.clamp(16, 128) },
        separation: 128,
        channels,
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Liquid Tracker ----
//
// "Liquid Module:", the title, author and tracker, a version, settings,
// the channels' panning and volume, the orders, then patterns ("LP\0\0",
// packed a channel at a time) and instruments ("LDSS", a sample each).
// Before 1.00, the NO format: "NO\0\0", 63 sample headers and patterns
// of 32-bit cells, unpacked.

pub fn probe_liq(d: &[u8]) -> bool {
    d.len() >= 109 && d.starts_with(b"Liquid Module:")
}

pub fn probe_no(d: &[u8]) -> bool {
    if d.len() < 43 + 256 + 63 * 46 || &d[..4] != b"NO\0\0" {
        return false;
    }
    let n = usize::from(d[4]);
    n <= 29 && d[5..5 + n].iter().all(|&c| c != 0) && d[34] > 0 && (1..=16).contains(&d[36])
}

/// A Liquid Tracker effect, its letter counted from A, in S3M's terms.
/// Pan and global volume are written in decimal.
fn liq_effect(fx: u8, p: u8) -> (Fx, u8) {
    let dec = bcd(p);
    match fx {
        0 => (Fx::Arpeggio, p),
        1 => (Fx::Tempo, p),
        2 => (Fx::PatternBreak, dec),
        3 => (Fx::PortaDown, p),
        5 => (Fx::FineVibrato, p),
        6 => (Fx::GlobalVolume, (dec.min(64)) * 2),
        9 => (Fx::PositionJump, p),
        11 => (Fx::VolSlide, p),
        12 => {
            let y = p & 0x0F;
            match p >> 4 {
                0x3 => (Fx::S3mEx, 0x10 | y),
                0x4 => (Fx::S3mEx, 0x30 | if y & 3 == 3 { y - 1 } else { y }),
                0x5 => (Fx::S3mEx, 0x20 | y),
                0x6 => (Fx::S3mEx, 0xB0 | y),
                0x7 => (Fx::S3mEx, 0x40 | if y & 3 == 3 { y - 1 } else { y }),
                0xC => (Fx::S3mEx, 0xC0 | y),
                0xD => (Fx::S3mEx, 0xD0 | y),
                0xE => (Fx::S3mEx, 0xE0 | y),
                _ => (Fx::None, 0),
            }
        }
        13 => (Fx::TonePorta, p),
        14 => (Fx::Offset, p),
        15 => liq_pan(dec),
        17 => (Fx::Retrig, p),
        18 => (Fx::Speed, p),
        19 => (Fx::Tremolo, p),
        20 => (Fx::PortaUp, p),
        21 => (Fx::Vibrato, p),
        23 => (Fx::TonePortaVol, p),
        24 => (Fx::VibratoVol, p),
        _ => (Fx::None, 0),
    }
}

/// Pan control, decimal: 0 to 64, 66 surround (70 resets them all, not kept).
fn liq_pan(v: u8) -> (Fx, u8) {
    match v {
        66 => (Fx::S3mEx, 0x91),
        0..=64 => (Fx::Panning, (u16::from(v) * 255 / 64) as u8),
        _ => (Fx::None, 0),
    }
}

/// A note: 0 is C-0 three octaves down; 0xFE lets it go.
fn liq_note(n: u8) -> Result<u8> {
    match n {
        0xFE => Ok(NOTE_OFF),
        0..=107 => Ok((n + 37).min(NOTE_MAX)),
        _ => bad("a pattern has a note Liquid Tracker can't play"),
    }
}

/// A packed event: `mask` says which of note, instrument, volume,
/// effect and parameter follow.
fn liq_event(r: &mut Reader, mask: u8) -> Result<Cell> {
    let mut cell = Cell::default();
    if mask & 0x01 != 0 {
        cell.note = liq_note(r.u8()?)?;
    }
    if mask & 0x02 != 0 {
        cell.instr = r.u8()?.saturating_add(1);
    }
    if mask & 0x04 != 0 {
        (cell.vol, cell.volp) = (Vol::Volume, r.u8()?);
    }
    let fx = if mask & 0x08 != 0 { Some(r.u8()?.wrapping_sub(b'A')) } else { None };
    let p = if mask & 0x10 != 0 { r.u8()? } else { 0xFF };
    if cell.instr > 100 || cell.volp > 64 {
        return bad("a pattern has an event Liquid Tracker doesn't write");
    }
    if let Some(fx) = fx {
        (cell.fx, cell.param) = liq_effect(fx, p);
    }
    Ok(cell)
}

pub fn load_liq(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 14);
    let title = r.text(30)?;
    let artist = r.text(20)?;
    r.skip(1)?;
    let tracker = r.text(20)?;
    let version = r.u16le()?;
    let speed = r.u16le()?;
    let tempo = r.u16le()?;
    r.skip(4)?;
    let nch = usize::from(r.u16le()?);
    let _flags = r.u32le()?;
    let npat = usize::from(r.u16le()?);
    let nins = usize::from(r.u16le()?);
    let (mut len, mut header) = (usize::from(r.u16le()?), usize::from(r.u16le()?));
    if nch == 0 || nch > 64 || npat > 256 || nins > 256 {
        return bad("its header doesn't add up");
    }
    let (stored_ch, stored_ord) = if version >> 8 == 0 {
        // 0.00 kept no length, and room for 64 channels and 256 orders.
        header = len;
        len = 0;
        r.skip(3)?;
        (64, 256)
    } else {
        (nch, len)
    };
    if len > 256 {
        return bad("it has more than 256 orders");
    }
    let mut channels = Vec::with_capacity(nch);
    for b in r.bytes(stored_ch)?.iter().take(nch) {
        let (pan, surround) = match *b {
            0..=63 => (b << 2, false),
            64 => (255, false),
            66 => (128, true),
            _ => return bad("a channel's panning is out of range"),
        };
        channels.push(Channel { pan, surround, ..Default::default() });
    }
    // 32 is full volume, 64 twice it.
    for (c, &v) in channels.iter_mut().zip(r.bytes(stored_ch)?) {
        c.volume = (v.min(32) * 2).min(64);
    }
    let raw = r.bytes(stored_ord)?;
    let orders: Vec<u16> = if version < 0x100 { raw.iter().take_while(|&&o| o != 0xFF).map(|&o| u16::from(o)).collect() } else { raw[..len].iter().map(|&o| u16::from(o)).collect() };
    r.seek(header)?;
    let mut patterns = Vec::with_capacity(npat);
    for _ in 0..npat {
        if r.tag()? != *b"LP\0\0" {
            return bad("a pattern isn't marked as one");
        }
        let name = r.text(30)?;
        let rows = usize::from(r.u16le()?);
        let size = r.u32le()? as usize;
        r.skip(4)?;
        if rows == 0 || rows > 256 {
            return bad("a pattern has more than 256 rows");
        }
        let mut p = Pattern::new(rows, nch);
        p.name = name;
        let mut pr = Reader::new(r.bytes_upto(size));
        liq_pattern(&mut pr, &mut p)?;
        patterns.push(Arc::new(p));
    }
    let mut samples = Vec::with_capacity(nins);
    for _ in 0..nins {
        let Ok(tag) = r.tag() else { break };
        if &tag == b"????" {
            samples.push(Sample::empty());
            continue;
        }
        if &tag != b"LDSS" {
            return bad("an instrument isn't marked as one");
        }
        let start = r.pos - 4;
        let ldss = r.u16le()?;
        let name = r.text(30)?;
        r.skip(40)?;
        r.skip(1)?;
        let mut len = r.u32le()? as usize;
        let (mut ls, mut le) = (r.u32le()?, r.u32le()?);
        let c5 = r.u32le()?;
        let vol = r.u8()?;
        let flags = r.u8()?;
        let pan = r.u8()?;
        r.skip(1)?;
        let gv = r.u8()?;
        r.skip(1)?;
        let hdr = usize::from(r.u16le()?);
        r.skip(6)?;
        r.skip(1)?;
        let loop_kind = r.u8()?;
        r.skip(10)?;
        let filename = r.text(25)?;
        r.seek(start + hdr.max(0x90))?;
        let (b16, stereo) = (flags & 1 != 0, flags & 2 != 0);
        let raw_le = le;
        let div = (if b16 { 2 } else { 1 }) * (if stereo { 2 } else { 1 });
        len /= div;
        ls /= div as u32;
        le /= div as u32;
        let ch = if stereo { 2 } else { 1 };
        // Stereo samples keep the left channel, then the right.
        let pcm = decode(r.bytes_upto(len * div), if b16 { Pcm::S16le } else { Pcm::S8 }, len * ch);
        let pcm = if stereo { crate::io::interleave(&pcm) } else { pcm };
        samples.push(Sample {
            name,
            filename,
            wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, ch as u8, pcm)),
            c5: if c5 == 0 { 8363 } else { c5.min(2_000_000) },
            volume: vol.min(64),
            // 32 is full, as the channels'.
            global_volume: if ldss < 0x101 { 64 } else { (gv.min(32) * 2).min(64) },
            pan: (pan <= 64).then(|| (u16::from(pan) * 4).min(255) as u8),
            looped: if raw_le > 0 { Loop { kind: if loop_kind == 1 { LoopKind::PingPong } else { LoopKind::Forward }, start: ls, end: le } } else { Loop::default() },
            ..Sample::empty()
        });
    }
    let mut m = Module {
        title,
        artist,
        format: "liq".into(),
        format_name: "Liquid Tracker".into(),
        tracker: format!("{} (LIQ {}.{:02})", if tracker.is_empty() { "Liquid Tracker" } else { &tracker }, version >> 8, version & 0xFF),
        kind: Kind::S3m,
        speed: speed.clamp(1, 255) as u8,
        tempo: tempo.min(255),
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels,
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

/// A packed Liquid pattern: channel after channel, each row by row.
/// C0 ends it, A0 and E1 xx go to the next channel (skipping xx), 80 and
/// E0 xx to the next row (skipping xx); C1 to DF are a packed event, A1
/// to BF one repeated (new bytes each row), 81 to 9F one copied; below,
/// an event unpacked (a note, then instrument, volume, effect and its
/// parameter, or the next row's first byte early).
fn liq_pattern(r: &mut Reader, p: &mut Pattern) -> Result<()> {
    let (rows, nch) = (p.rows as i64, p.channels as i64);
    let (mut row, mut ch) = (0i64, 0i64);
    let (mut mask, mut repeat) = (0u8, 0u8);
    let mut pending: Option<u8> = None;
    loop {
        if ch >= nch || row >= rows || row < 0 {
            return bad("a pattern runs past its rows or channels");
        }
        let at = (row * nch + ch) as usize;
        if pending.is_none() && repeat > 0 {
            p.cells[at] = liq_event(r, mask)?;
            repeat -= 1;
        } else {
            let Some(x) = pending.take().or_else(|| r.u8().ok()) else {
                return Ok(());
            };
            match x {
                0xC0 => return Ok(()),
                0xA0 | 0xE1 => {
                    if x == 0xE1 {
                        ch += i64::from(r.u8()?);
                    }
                    ch = (ch + 1).min(nch - 1);
                    row = -1;
                }
                0x80 | 0xE0 => {
                    if x == 0xE0 {
                        row += i64::from(r.u8()?);
                    }
                }
                0xC1..=0xDF => p.cells[at] = liq_event(r, x)?,
                0xA1..=0xBF => {
                    repeat = r.u8()?;
                    mask = x;
                    p.cells[at] = liq_event(r, x)?;
                }
                0x81..=0x9F => {
                    let n = r.u8()?;
                    let cell = liq_event(r, x)?;
                    p.cells[at] = cell;
                    for _ in 0..n {
                        row += 1;
                        if row >= rows {
                            return bad("a pattern runs past its rows");
                        }
                        p.cells[(row * nch + ch) as usize] = cell;
                    }
                }
                _ => {
                    let mut cell = Cell { note: liq_note(x)?, ..Default::default() };
                    let i = r.u8()?;
                    if i > 100 {
                        // Only a note: this is the next row's first byte.
                        p.cells[at] = cell;
                        row += 1;
                        pending = Some(i);
                        continue;
                    }
                    let (v, fx, param) = (r.u8()?, r.u8()?, r.u8()?);
                    if i != 0xFF {
                        cell.instr = i + 1;
                    }
                    if v != 0xFF {
                        if v > 64 {
                            return bad("a pattern has a volume past 64");
                        }
                        (cell.vol, cell.volp) = (Vol::Volume, v);
                    }
                    if fx != 0xFF {
                        (cell.fx, cell.param) = liq_effect(fx.wrapping_sub(b'A'), param);
                    }
                    p.cells[at] = cell;
                }
            }
        }
        row += 1;
        if row >= rows {
            row = 0;
            repeat = 0;
            ch += 1;
        }
        if ch >= nch {
            ch = 0;
        }
    }
}

/// An effect of the NO format, its letter counted from A.
fn no_effect(fx: u8, p: u8) -> (Fx, u8) {
    let y = p & 0x0F;
    match fx {
        0x0 => mod_effect(0x0F, p, false),
        0x1 => (Fx::Vibrato, p),
        0x2 => (Fx::PatternBreak, bcd(p)),
        0x3 => (Fx::PortaDown, p),
        0x4 => (Fx::PortaUp, p),
        0x6 => (Fx::Arpeggio, p),
        0x7 => match bcd(p) {
            v @ 0..=64 => liq_pan(v),
            _ => (Fx::None, 0),
        },
        0x8 => match p >> 4 {
            0x0 => (Fx::VibratoVol, y << 4),
            0x1 => (Fx::VibratoVol, y),
            0x2 => (Fx::TonePortaVol, y << 4),
            0x3 => (Fx::TonePortaVol, y),
            // Tremolo and a volume slide: the tremolo kept.
            0x4 | 0x5 => (Fx::Tremolo, 0),
            _ => (Fx::None, 0),
        },
        0x9 => (Fx::PositionJump, p),
        0xA => (Fx::Tremolo, p),
        0xB => (Fx::VolSlide, p),
        0xC => match p >> 4 {
            0x0 => (Fx::PortaUp, 0xF0 | y),
            0x1 => (Fx::PortaDown, 0xF0 | y),
            0x2 => (Fx::VolSlide, y << 4 | 0x0F),
            0x3 => (Fx::VolSlide, 0xF0 | y),
            0x4 => (Fx::S3mEx, 0x30 | if y & 3 == 3 { y - 1 } else { y }),
            0x5 => (Fx::S3mEx, 0x40 | if y & 3 == 3 { y - 1 } else { y }),
            0x6 => (Fx::Retrig, y),
            0x7 => (Fx::S3mEx, 0xC0 | y),
            0x8 => (Fx::S3mEx, 0xD0 | y),
            0xB => (Fx::S3mEx, 0xB0 | y),
            0xC => (Fx::S3mEx, 0xE0 | y),
            _ => (Fx::None, 0),
        },
        0xD => (Fx::TonePorta, p),
        0xE => (Fx::Offset, p),
        _ => (Fx::None, 0),
    }
}

pub fn load_no(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 4);
    let n = usize::from(r.u8()?);
    let title = crate::io::text(&r.bytes(29)?[..n.min(29)]);
    let npat = usize::from(r.u8()?);
    r.skip(1)?;
    let nch = usize::from(r.u8()?);
    r.skip(6)?;
    let orders: Vec<u16> = r.bytes(256)?.iter().take_while(|&&o| o != 0xFF).map(|&o| u16::from(o)).collect();
    let mut heads = Vec::with_capacity(63);
    for _ in 0..63 {
        let b = r.bytes(46)?;
        let le32 = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let name = crate::io::text(&b[1..1 + usize::from(b[0]).min(30)]);
        heads.push((name, b[31], u16::from_le_bytes([b[32], b[33]]), le32(34) as usize, le32(38), le32(42)));
    }
    let mut patterns = Vec::with_capacity(npat);
    for _ in 0..npat {
        let mut p = Pattern::new(64, nch);
        for cell in p.cells.iter_mut() {
            let x = r.u32le()?;
            let (note, ins, vol, fx, param) = (x & 0x3F, (x >> 6) & 0x7F, (x >> 13) & 0x7F, ((x >> 20) & 0x0F) as u8, (x >> 24) as u8);
            if note != 0x3F {
                cell.note = (37 + note as u8).min(NOTE_MAX);
            }
            if ins != 0x7F {
                cell.instr = ins as u8 + 1;
            }
            if vol != 0x7F {
                (cell.vol, cell.volp) = (Vol::Volume, (vol as u8).min(64));
            }
            if fx != 0x0F {
                (cell.fx, cell.param) = no_effect(fx, param);
            }
        }
        patterns.push(Arc::new(p));
    }
    let samples = heads
        .into_iter()
        .map(|(name, vol, c5, len, ls, le)| {
            let pcm = decode(r.bytes_upto(len), Pcm::U8, len);
            Sample {
                name,
                wave: Arc::new(Wave::new(8, 1, pcm)),
                c5: if c5 == 0 { 8363 } else { u32::from(c5) },
                volume: vol.min(64),
                looped: if le > 0 { Loop::forward(ls, le) } else { Loop::default() },
                ..Sample::empty()
            }
        })
        .collect();
    let mut m = Module {
        title,
        format: "liq".into(),
        format_name: "Liquid Tracker".into(),
        tracker: "Liquid Tracker (NO)".into(),
        kind: Kind::S3m,
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels: (0..nch).map(|c| Channel { pan: if c & 1 != 0 { 192 } else { 64 }, ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liquid_patterns_unpack() {
        // Channel 0: a packed note and instrument on row 0, row 1 skipped,
        // then an unpacked note with no more on row 2; next channel: a
        // copied event over rows 0 to 2. Then the end.
        let data = [0xC3, 24, 0, 0x80, 30, 0xA0, 0x83, 2, 12, 4, 0xC0];
        let mut p = Pattern::new(4, 2);
        liq_pattern(&mut Reader::new(&data), &mut p).unwrap();
        assert_eq!((p.cell(0, 0).note, p.cell(0, 0).instr), (61, 1));
        assert_eq!(p.cell(2, 0).note, 67);
        assert_eq!(p.cell(3, 0).note, 0);
        for row in 0..3 {
            assert_eq!((p.cell(row, 1).note, p.cell(row, 1).instr), (49, 5));
        }
        assert_eq!(p.cell(3, 1).note, 0);
    }

    #[test]
    fn graoumf_log_volume_doubles_an_octave_up() {
        assert_eq!(graoumf_log(0x100), 2);
        assert_eq!(graoumf_log(0xF00), 32768);
        assert_eq!(graoumf_log(0xE00), 16384);
    }
}
