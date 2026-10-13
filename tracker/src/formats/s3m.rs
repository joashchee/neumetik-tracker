//! Scream Tracker 3 (.s3m), and the trackers that wrote it after:
//! Impulse Tracker, Schism Tracker, OpenMPT, BeRoTracker, Imago
//! Orpheus. A header, the order list, parapointers (offsets in 16-byte
//! paragraphs) to the samples and the patterns, and patterns packed a
//! row at a time.

use std::sync::Arc;

use super::common::s3m_effect;
use crate::io::{bad, decode, interleave, Pcm, Reader, Result};
use crate::model::{Cell, Channel, Flags, FmPatch, Kind, Loop, Module, Pattern, Sample, Vol, Wave, NOTE_CUT, NOTE_MAX, ORDER_END, ORDER_SKIP};

pub fn probe(data: &[u8]) -> bool {
    data.len() >= 96 && &data[44..48] == b"SCRM" && data[29] == 16
}

/// The tracker a header's version word names.
pub fn tracker_name(cwt: u16) -> String {
    let (major, minor) = ((cwt >> 8) & 0x0F, cwt & 0xFF);
    match cwt >> 12 {
        1 => format!("Scream Tracker {major}.{minor:02x}"),
        2 => format!("Imago Orpheus {major}.{minor:02x}"),
        3 if cwt == 0x3216 => "Impulse Tracker 2.14p3".into(),
        3 => format!("Impulse Tracker {major}.{minor:02x}"),
        4 => format!("Schism Tracker ({cwt:04x})"),
        5 => "OpenMPT".into(),
        6 => "BeRoTracker".into(),
        7 => "CreamTracker".into(),
        _ => "Scream Tracker 3".into(),
    }
}

/// ModPlug's 4-bit ADPCM: 16 deltas, then two nibbles a byte, each an
/// index into them.
pub fn adpcm4(b: &[u8], frames: usize) -> Vec<i16> {
    let Some(table) = b.get(..16) else { return Vec::new() };
    let mut acc = 0i8;
    let mut out = Vec::with_capacity(frames);
    for &byte in &b[16..] {
        for nib in [byte & 0x0F, byte >> 4] {
            if out.len() >= frames {
                return out;
            }
            acc = acc.wrapping_add(table[usize::from(nib)] as i8);
            out.push(i16::from(acc) << 8);
        }
    }
    out
}

pub fn load(data: &[u8]) -> Result<Module> {
    let mut r = Reader::new(data);
    let title = r.text(28)?;
    r.seek(32)?;
    let ordnum = usize::from(r.u16le()?);
    let insnum = usize::from(r.u16le()?);
    let patnum = usize::from(r.u16le()?);
    let flags = r.u16le()?;
    let cwt = r.u16le()?;
    let ffi = r.u16le()?;
    r.seek(48)?;
    let gv = r.u8()?;
    let speed = r.u8()?;
    let tempo = r.u8()?;
    let mv = r.u8()?;
    let _ultraclick = r.u8()?;
    let default_pan = r.u8()?;
    r.seek(64)?;
    let settings = r.bytes(32)?.to_vec();
    let orders_raw = r.bytes(ordnum)?.to_vec();
    let mut ins_ptr = Vec::with_capacity(insnum);
    for _ in 0..insnum {
        ins_ptr.push(usize::from(r.u16le()?) * 16);
    }
    let mut pat_ptr = Vec::with_capacity(patnum);
    for _ in 0..patnum {
        pat_ptr.push(usize::from(r.u16le()?) * 16);
    }
    let pans = if default_pan == 252 { r.bytes(32).ok().map(|b| b.to_vec()) } else { None };

    // Channels: those in use, in order, mapped from S3M's 32 slots. Left
    // ones (0 to 7) and right (8 to 15); the AdLib's nine melody channels
    // (16 to 24) in the middle, as the mono chip was. Its drum channels
    // (25 up) are kept silent so the patterns still line up.
    let stereo = mv & 0x80 != 0;
    let mut channels: Vec<Channel> = Vec::new();
    let mut slot_of = [usize::MAX; 32];
    let last_used = (0..32).rev().find(|&i| settings[i] < 0x80).map_or(0, |i| i + 1);
    for i in 0..last_used {
        let s = settings[i];
        slot_of[i] = channels.len();
        let mut pan = if !stereo {
            128
        } else if s & 0x7F < 8 {
            0x33
        } else if s & 0x7F < 16 {
            0xCC
        } else if s & 0x7F < 25 {
            128
        } else {
            128
        };
        if let Some(p) = pans.as_ref().map(|p| p[i]).filter(|p| p & 0x20 != 0) {
            pan = (p & 0x0F) * 17;
        }
        channels.push(Channel { pan, muted: s >= 0x80 || (s & 0x7F) >= 25, ..Default::default() });
    }
    if channels.is_empty() {
        return bad("it has no channels");
    }

    let unsigned = ffi != 1;
    let mut samples = Vec::with_capacity(insnum);
    for &p in &ins_ptr {
        let mut s = Sample::empty();
        let mut h = Reader::at(data, p);
        let Ok(kind) = h.u8() else {
            samples.push(s);
            continue;
        };
        s.filename = h.text(12).unwrap_or_default();
        let hi = h.u8().unwrap_or(0);
        let lo = h.u16le().unwrap_or(0);
        let at = (usize::from(hi) << 16 | usize::from(lo)) * 16;
        let len = h.u32le().unwrap_or(0) as usize;
        let ls = h.u32le().unwrap_or(0);
        let le = h.u32le().unwrap_or(0);
        s.volume = h.u8().unwrap_or(64).min(64);
        let _ = h.u8();
        let pack = h.u8().unwrap_or(0);
        let sflags = h.u8().unwrap_or(0);
        s.c5 = h.u32le().unwrap_or(8363);
        let _ = h.skip(12);
        s.name = h.text(28).unwrap_or_default();
        if kind == 1 && len > 0 {
            let len = len.min(4 << 20);
            let st = sflags & 2 != 0;
            let b16 = sflags & 4 != 0;
            let channels = if st { 2 } else { 1 };
            let pcm = if pack == 4 && !b16 && !st {
                adpcm4(data.get(at..).unwrap_or(&[]), len)
            } else {
                let fmt = match (b16, unsigned) {
                    (false, true) => Pcm::U8,
                    (false, false) => Pcm::S8,
                    (true, true) => Pcm::U16le,
                    (true, false) => Pcm::S16le,
                };
                let bytes = data.get(at..).unwrap_or(&[]);
                let raw = decode(bytes, fmt, len * channels);
                if st && raw.len() == len * 2 {
                    interleave(&raw)
                } else if st {
                    // Cut short: keep the left.
                    raw[..raw.len().min(len)].to_vec()
                } else {
                    raw
                }
            };
            let ch = if st && pcm.len() == len * 2 { 2 } else { 1 };
            s.wave = Arc::new(Wave::new(if b16 { 16 } else { 8 }, ch, pcm));
            if sflags & 1 != 0 {
                s.looped = Loop::forward(ls, le);
            }
        } else if kind >= 2 {
            // An AdLib instrument: its eleven OPL bytes (D00 to D0A) where
            // a sample's place and length would be, and no waveform.
            if let Some(b) = data.get(p + 0x10..p + 0x1B) {
                let b: [u8; 11] = b.try_into().expect("eleven bytes");
                s.fm = Some(FmPatch::from_s3m(&b));
            }
            s.name = if s.name.is_empty() { "(AdLib instrument)".into() } else { s.name };
        }
        samples.push(s);
    }

    let mut patterns = Vec::with_capacity(patnum);
    for &p in &pat_ptr {
        let mut pat = Pattern::new(64, channels.len());
        if p != 0 && p + 2 <= data.len() {
            let mut pr = Reader::at(data, p + 2);
            let mut row = 0;
            while row < 64 {
                let Ok(what) = pr.u8() else { break };
                if what == 0 {
                    row += 1;
                    continue;
                }
                let slot = usize::from(what & 31);
                let mut cell = Cell::default();
                if what & 32 != 0 {
                    let (Ok(n), Ok(i)) = (pr.u8(), pr.u8()) else { break };
                    cell.note = match n {
                        255 => 0,
                        254 => NOTE_CUT,
                        n if (n & 0x0F) < 12 => ((n >> 4) * 12 + (n & 0x0F) + 12 + 1).min(NOTE_MAX),
                        _ => 0,
                    };
                    cell.instr = i;
                }
                if what & 64 != 0 {
                    let Ok(v) = pr.u8() else { break };
                    match v {
                        0..=64 => (cell.vol, cell.volp) = (Vol::Volume, v),
                        128..=192 => (cell.vol, cell.volp) = (Vol::Panning, v - 128),
                        _ => {}
                    }
                }
                if what & 128 != 0 {
                    let (Ok(c), Ok(i)) = (pr.u8(), pr.u8()) else { break };
                    (cell.fx, cell.param) = s3m_effect(c, i, false);
                }
                let ch = slot_of[slot];
                if let Some(c) = pat.cell_mut(row, ch) {
                    *c = cell;
                }
            }
        }
        patterns.push(Arc::new(pat));
    }

    let orders = orders_raw
        .iter()
        .take_while(|&&o| o != 255)
        .map(|&o| match o {
            254 => ORDER_SKIP,
            255 => ORDER_END,
            o => u16::from(o),
        })
        .filter(|&o| o >= ORDER_SKIP || usize::from(o) < patterns.len().max(1))
        .collect();

    let mut m = Module {
        title,
        format: "s3m".into(),
        format_name: "Scream Tracker 3".into(),
        tracker: tracker_name(cwt),
        kind: Kind::S3m,
        flags: Flags { amiga_limits: flags & 16 != 0, fast_slides: flags & 64 != 0 || cwt == 0x1300, ..Default::default() },
        speed: if speed == 0 || speed == 255 { 6 } else { speed },
        tempo: if tempo < 33 { 125 } else { u16::from(tempo) },
        global_volume: gv.min(64) * 2,
        mix_volume: (mv & 0x7F).max(16),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A song of one AdLib channel playing its instrument's middle C.
    fn adlib_song() -> Vec<u8> {
        let mut d = vec![0u8; 0x100];
        d[..4].copy_from_slice(b"Test");
        d[0x1C] = 0x1A;
        d[0x1D] = 16;
        d[0x20..0x26].copy_from_slice(&[2, 0, 1, 0, 1, 0]); // orders, instruments, patterns
        d[0x28] = 0x20; // made by Scream Tracker 3.20
        d[0x29] = 0x13;
        d[0x2A] = 2;
        d[0x2C..0x30].copy_from_slice(b"SCRM");
        d[0x30] = 64;
        (d[0x31], d[0x32], d[0x33]) = (6, 125, 0xB0);
        d[0x40..0x60].fill(255);
        d[0x40] = 16; // AdLib melody channel 1
        d[0x60..0x62].copy_from_slice(&[0, 255]);
        d[0x62..0x64].copy_from_slice(&7u16.to_le_bytes()); // the instrument at 0x70
        d[0x64..0x66].copy_from_slice(&12u16.to_le_bytes()); // the pattern at 0xC0
        let ins = 0x70;
        d[ins] = 2;
        d[ins + 0x10..ins + 0x1B].copy_from_slice(&[0x21, 0x21, 0x3F, 0x00, 0xF0, 0xF0, 0x00, 0x05, 0, 0, 0x00]);
        d[ins + 0x1C] = 64;
        d[ins + 0x20..ins + 0x24].copy_from_slice(&8363u32.to_le_bytes());
        d[ins + 0x30..ins + 0x35].copy_from_slice(b"organ");
        d[ins + 0x4C..ins + 0x50].copy_from_slice(b"SCRI");
        let mut pat = vec![0x20, 0x40, 1, 0];
        pat.extend(std::iter::repeat_n(0u8, 63));
        d.truncate(0xC0);
        d.extend_from_slice(&((pat.len() + 2) as u16).to_le_bytes());
        d.extend_from_slice(&pat);
        d
    }

    #[test]
    fn an_adlib_instrument_is_read_and_played() {
        let m = load(&adlib_song()).unwrap();
        assert_eq!(m.samples[0].fm.map(|p| p.to_s3m()[0]), Some(0x21));
        assert!(!m.channels[0].muted);
        let audio = crate::player::render(std::sync::Arc::new(m), 44_100, 2.0, 0.0);
        let left: Vec<f32> = audio.iter().step_by(2).copied().collect();
        let crossings = left[4410..48_510].windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        // Middle C for a second: 261.6 Hz.
        assert!((259..=264).contains(&crossings), "{crossings}");
    }
}
