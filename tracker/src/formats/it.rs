//! Impulse Tracker (.it), and what writes it: Schism Tracker, OpenMPT
//! (and its .mptm, IT with more after it), ModPlug Tracker. A header, the
//! orders, offsets to instruments, samples and patterns; instruments
//! with three envelopes, new note actions and a filter; samples that may
//! be compressed (IT 2.14's and 2.15's packing, undone here).

use std::sync::Arc;

use super::common::s3m_effect;
use crate::io::{decode, interleave, message, Pcm, Reader, Result};
use crate::model::{
    AutoVibrato, Cell, Channel, EnvPoint, Envelope, Flags, Instrument, Kind, Loop, LoopKind, MidiOut, Module, Nna, Pattern, Sample, Vol, Wave, NOTE_CUT, NOTE_FADE, NOTE_OFF, ORDER_END, ORDER_SKIP,
};

pub fn probe(data: &[u8]) -> bool {
    data.len() >= 192 && &data[..4] == b"IMPM"
}

/// What the version word says made it.
fn tracker_name(cwt: u16, cmwt: u16) -> String {
    match cwt >> 12 {
        1 => "Schism Tracker".into(),
        5 => "OpenMPT".into(),
        0 if cwt == 0x0888 || cwt == 0x0300 => "OpenMPT / ModPlug Tracker".into(),
        0 if cwt >= 0x0200 => format!("Impulse Tracker {}.{:02x}", (cwt >> 8) & 0x0F, cwt & 0xFF),
        0 => format!("Impulse Tracker (format {}.{:02x})", (cmwt >> 8) & 0x0F, cmwt & 0xFF),
        _ => "Impulse Tracker".into(),
    }
}

/// Bits read lowest first.
struct Bits<'a> {
    b: &'a [u8],
    pos: usize,
    bit: u32,
}

impl Bits<'_> {
    fn read(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for i in 0..n {
            let byte = *self.b.get(self.pos)?;
            v |= u32::from((byte >> self.bit) & 1) << i;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
        }
        Some(v)
    }
}

/// IT 2.14's sample packing (2.15's when `it215`: the deltas twice
/// integrated), in blocks of 32 KB of output, each with its own length.
pub fn decompress(data: &[u8], frames: usize, sixteen: bool, it215: bool) -> Vec<i16> {
    let mut out: Vec<i16> = Vec::with_capacity(frames);
    let mut r = Reader::new(data);
    let block_frames = if sixteen { 0x4000 } else { 0x8000 };
    while out.len() < frames {
        let Ok(len) = r.u16le() else { break };
        let Ok(block) = r.bytes(usize::from(len)) else { break };
        let mut bits = Bits { b: block, pos: 0, bit: 0 };
        let want = block_frames.min(frames - out.len());
        let (top, step_bits): (u32, u32) = if sixteen { (17, 4) } else { (9, 3) };
        let mut width = top;
        let (mut d1, mut d2) = (0i32, 0i32);
        let mut made = 0;
        while made < want {
            let Some(mut value) = bits.read(width) else { break };
            if width < 7 {
                if value == 1 << (width - 1) {
                    let Some(v) = bits.read(step_bits) else { break };
                    let v = v + 1;
                    width = if v < width { v } else { v + 1 };
                    continue;
                }
            } else if width < top {
                let border = ((if sixteen { 0xFFFFu32 } else { 0xFF }) >> (top - width)) - if sixteen { 8 } else { 4 };
                if value > border && value <= border + if sixteen { 16 } else { 8 } {
                    value -= border;
                    width = if value < width { value } else { value + 1 };
                    continue;
                }
            } else if width == top {
                if value & (1 << (top - 1)) != 0 {
                    width = (value + 1) & 0xFF;
                    if width == 0 || width > top {
                        break;
                    }
                    continue;
                }
            } else {
                break;
            }
            let full = if sixteen { 16 } else { 8 };
            let v = if width < full {
                let shift = 32 - width;
                ((value << shift) as i32) >> shift
            } else {
                let shift = 32 - full;
                ((value << shift) as i32) >> shift
            };
            d1 = d1.wrapping_add(v);
            d2 = d2.wrapping_add(d1);
            let s = if it215 { d2 } else { d1 };
            out.push(if sixteen { s as i16 } else { (i16::from(s as i8)) << 8 });
            made += 1;
        }
        // A block that ends early is padded, so the next one starts in step.
        while made < want {
            out.push(0);
            made += 1;
        }
    }
    out.truncate(frames);
    out
}

/// The volume column byte as UMF's.
fn volume_column(v: u8) -> (Vol, u8) {
    const PORTA: [u8; 10] = [0, 1, 4, 8, 16, 32, 64, 96, 128, 255];
    match v {
        0..=64 => (Vol::Volume, v),
        65..=74 => (Vol::FineVolUp, v - 65),
        75..=84 => (Vol::FineVolDown, v - 75),
        85..=94 => (Vol::VolSlideUp, v - 85),
        95..=104 => (Vol::VolSlideDown, v - 95),
        105..=114 => (Vol::PortaDown, v - 105),
        115..=124 => (Vol::PortaUp, v - 115),
        128..=192 => (Vol::Panning, v - 128),
        193..=202 => (Vol::TonePorta, PORTA[usize::from(v - 193)]),
        203..=212 => (Vol::VibratoDepth, v - 203),
        _ => (Vol::None, 0),
    }
}

fn envelope(r: &mut Reader, pan: bool) -> Result<Envelope> {
    let flags = r.u8()?;
    let num = r.u8()?.min(25);
    let (lb, le, sb, se) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
    let mut points = Vec::with_capacity(25);
    for i in 0..25 {
        let v = r.i8()?;
        let t = r.u16le()?;
        if i < num {
            points.push(EnvPoint { tick: t, value: if pan { v.clamp(-32, 32) } else { v.clamp(0, 64) } });
        }
    }
    r.skip(1)?;
    let n = points.len() as u8;
    Ok(Envelope {
        enabled: flags & 1 != 0 && n > 0,
        looped: (flags & 2 != 0 && lb <= le && le < n).then_some((lb, le)),
        sustain: (flags & 4 != 0 && sb <= se && se < n).then_some((sb, se)),
        carry: flags & 8 != 0,
        filter: flags & 0x80 != 0,
        points,
        release: None,
    })
}

fn instrument(data: &[u8], at: usize, old: bool) -> Result<Instrument> {
    let mut r = Reader::at(data, at);
    if r.peek(4) != Some(b"IMPI") {
        return Ok(Instrument::default());
    }
    r.skip(4)?;
    let filename = r.text(12)?;
    r.skip(1)?;
    let mut ins = Instrument { filename, global_volume: 128, ..Default::default() };
    if old {
        // Impulse Tracker 1's instruments: one volume envelope.
        let flags = r.u8()?;
        let (vls, vle, sls, sle) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
        r.skip(2)?;
        ins.fadeout = u32::from(r.u16le()?) * 128;
        ins.nna = Nna::from_u8(r.u8()?);
        ins.dct = r.u8()?;
        r.skip(4)?;
        ins.name = r.text(26)?;
        r.skip(6)?;
        let km = r.bytes(240)?;
        ins.keymap = km.chunks_exact(2).map(|c| (c[0].min(119), u16::from(c[1]))).collect();
        r.skip(200)?;
        let mut points = Vec::new();
        for _ in 0..25 {
            let (t, v) = (r.u8()?, r.u8()?);
            if t == 0xFF {
                break;
            }
            points.push(EnvPoint { tick: u16::from(t), value: v.min(64) as i8 });
        }
        let n = points.len() as u8;
        ins.volume_env = Envelope {
            enabled: flags & 1 != 0 && n > 0,
            looped: (flags & 2 != 0 && vls <= vle && vle < n).then_some((vls, vle)),
            sustain: (flags & 4 != 0 && sls <= sle && sle < n).then_some((sls, sle)),
            points,
            ..Default::default()
        };
        return Ok(ins);
    }
    ins.nna = Nna::from_u8(r.u8()?);
    ins.dct = r.u8()?;
    ins.dca = r.u8()?;
    ins.fadeout = u32::from(r.u16le()?.min(1024)) * 64;
    ins.pitch_pan_separation = r.i8()?;
    ins.pitch_pan_center = r.u8()?;
    ins.global_volume = r.u8()?.min(128);
    let dfp = r.u8()?;
    ins.pan = (dfp & 0x80 == 0).then(|| (u16::from(dfp.min(64)) * 255 / 64) as u8);
    ins.random_volume = r.u8()?;
    ins.random_pan = r.u8()?;
    r.skip(4)?;
    ins.name = r.text(26)?;
    let ifc = r.u8()?;
    let ifr = r.u8()?;
    ins.cutoff = (ifc & 0x80 != 0).then_some(ifc & 0x7F);
    ins.resonance = (ifr & 0x80 != 0).then_some(ifr & 0x7F);
    // Its MIDI output: a channel (1 to 16), a program and bank.
    let (mch, mpr, bank) = (r.u8()?, r.u8()?, r.u16le()?);
    if (1..=16).contains(&mch) {
        ins.midi = Some(MidiOut { channel: mch - 1, program: (mpr < 128).then_some(mpr), bank: (bank < 16384).then_some(bank) });
    }
    let km = r.bytes(240)?;
    ins.keymap = km.chunks_exact(2).map(|c| (c[0].min(119), u16::from(c[1]))).collect();
    ins.volume_env = envelope(&mut r, false)?;
    ins.pan_env = envelope(&mut r, true)?;
    ins.pitch_env = envelope(&mut r, true)?;
    Ok(ins)
}

pub(crate) fn sample(data: &[u8], at: usize, cwt: u16) -> Result<Sample> {
    let mut r = Reader::at(data, at);
    let mut s = Sample::empty();
    if r.peek(4) != Some(b"IMPS") {
        return Ok(s);
    }
    r.skip(4)?;
    s.filename = r.text(12)?;
    r.skip(1)?;
    s.global_volume = r.u8()?.min(64);
    let flags = r.u8()?;
    s.volume = r.u8()?.min(64);
    s.name = r.text(26)?;
    let cvt = r.u8()?;
    let dfp = r.u8()?;
    s.pan = (dfp & 0x80 != 0).then(|| (u16::from((dfp & 0x7F).min(64)) * 255 / 64) as u8);
    let len = r.u32le()? as usize;
    let lb = r.u32le()?;
    let le = r.u32le()?;
    s.c5 = r.u32le()?.clamp(1, 9_999_999);
    let sb = r.u32le()?;
    let se = r.u32le()?;
    let ptr = r.u32le()? as usize;
    s.vibrato = AutoVibrato { speed: r.u8()?, depth: r.u8()?, sweep: r.u8()?, kind: r.u8()? };
    s.vibrato.kind = match s.vibrato.kind {
        0 => 0,
        1 => 1,
        2 => 2,
        _ => 3,
    };
    if flags & 0x10 != 0 {
        s.looped = Loop { kind: if flags & 0x40 != 0 { LoopKind::PingPong } else { LoopKind::Forward }, start: lb, end: le };
    }
    if flags & 0x20 != 0 {
        s.sustain = Loop { kind: if flags & 0x80 != 0 { LoopKind::PingPong } else { LoopKind::Forward }, start: sb, end: se };
    }
    if flags & 1 == 0 || len == 0 {
        return Ok(s);
    }
    let len = len.min(16 << 20);
    let b16 = flags & 2 != 0;
    let stereo = flags & 4 != 0;
    let ch = if stereo { 2 } else { 1 };
    let bytes = data.get(ptr..).unwrap_or(&[]);
    let pcm = if flags & 8 != 0 {
        let it215 = cvt & 4 != 0;
        if stereo {
            // Each channel packed on its own, one after the other.
            let left = decompress(bytes, len, b16, it215);
            let used = packed_len(bytes, len, b16);
            let right = decompress(bytes.get(used..).unwrap_or(&[]), len, b16, it215);
            left.iter().zip(right.iter()).flat_map(|(&l, &r)| [l, r]).collect()
        } else {
            decompress(bytes, len, b16, it215)
        }
    } else {
        let fmt = match (b16, cvt & 1 != 0, cvt & 4 != 0) {
            (false, _, true) => Pcm::D8,
            (true, _, true) => Pcm::D16le,
            (false, true, _) => Pcm::S8,
            (false, false, _) => Pcm::U8,
            (true, true, _) => Pcm::S16le,
            (true, false, _) => Pcm::U16le,
        };
        let raw = decode(bytes, fmt, len * ch);
        if stereo && raw.len() == len * 2 {
            interleave(&raw)
        } else {
            raw
        }
    };
    let _ = cwt;
    let ch = if stereo && pcm.len() == len * 2 { 2 } else { 1 };
    s.wave = Arc::new(Wave::new(if b16 { 16 } else { 8 }, ch, pcm));
    Ok(s)
}

/// The bytes packed blocks take for `frames` frames.
fn packed_len(data: &[u8], frames: usize, sixteen: bool) -> usize {
    let per = if sixteen { 0x4000 } else { 0x8000 };
    let mut at = 0;
    let mut left = frames;
    while left > 0 && at + 2 <= data.len() {
        at += 2 + usize::from(u16::from_le_bytes([data[at], data[at + 1]]));
        left = left.saturating_sub(per);
    }
    at.min(data.len())
}

pub fn load(data: &[u8]) -> Result<Module> {
    let mut r = Reader::new(data);
    r.seek(4)?;
    let title = r.text(26)?;
    r.skip(2)?;
    let ordnum = usize::from(r.u16le()?);
    let insnum = usize::from(r.u16le()?);
    let smpnum = usize::from(r.u16le()?);
    let patnum = usize::from(r.u16le()?);
    let cwt = r.u16le()?;
    let cmwt = r.u16le()?;
    let flags = r.u16le()?;
    let special = r.u16le()?;
    let gv = r.u8()?;
    let mv = r.u8()?;
    let speed = r.u8()?;
    let tempo = r.u8()?;
    let sep = r.u8()?;
    let _pwd = r.u8()?;
    let msg_len = usize::from(r.u16le()?);
    let msg_off = r.u32le()? as usize;
    r.skip(4)?;
    let pans = r.bytes(64)?.to_vec();
    let vols = r.bytes(64)?.to_vec();
    let orders_raw = r.bytes(ordnum)?.to_vec();
    let offsets = |n: usize, r: &mut Reader| -> Result<Vec<usize>> { (0..n).map(|_| r.u32le().map(|v| v as usize)).collect() };
    let ins_off = offsets(insnum, &mut r)?;
    let smp_off = offsets(smpnum, &mut r)?;
    let pat_off = offsets(patnum, &mut r)?;

    let old = cmwt < 0x200;
    let instruments: Vec<Instrument> = if flags & 4 != 0 { ins_off.iter().map(|&o| instrument(data, o, old).unwrap_or_default()).collect() } else { Vec::new() };
    let samples: Vec<Sample> = smp_off.iter().map(|&o| sample(data, o, cwt).unwrap_or_else(|_| Sample::empty())).collect();

    // Patterns, packed with each channel's last mask and values remembered.
    let mut raw = Vec::with_capacity(patnum);
    let mut used = 0usize;
    for &p in &pat_off {
        if p == 0 || p + 8 > data.len() {
            raw.push((64usize, Vec::new()));
            continue;
        }
        let mut pr = Reader::at(data, p);
        let len = usize::from(pr.u16le()?);
        let rows = usize::from(pr.u16le()?).clamp(1, 1024);
        pr.skip(4)?;
        let packed = pr.bytes_upto(len);
        let mut pr = Reader::new(packed);
        let mut cells: Vec<(usize, usize, Cell)> = Vec::new();
        let mut last_mask = [0u8; 64];
        let mut last = [Cell::default(); 64];
        let mut row = 0;
        while row < rows {
            let Ok(var) = pr.u8() else { break };
            if var == 0 {
                row += 1;
                continue;
            }
            let ch = usize::from((var - 1) & 63);
            let mask = if var & 128 != 0 {
                let Ok(m) = pr.u8() else { break };
                last_mask[ch] = m;
                m
            } else {
                last_mask[ch]
            };
            let mut c = Cell::default();
            if mask & 1 != 0 {
                let Ok(n) = pr.u8() else { break };
                last[ch].note = match n {
                    0..=119 => n + 1,
                    255 => NOTE_OFF,
                    254 => NOTE_CUT,
                    _ => NOTE_FADE,
                };
                c.note = last[ch].note;
            }
            if mask & 2 != 0 {
                let Ok(i) = pr.u8() else { break };
                last[ch].instr = i;
                c.instr = i;
            }
            if mask & 4 != 0 {
                let Ok(v) = pr.u8() else { break };
                (last[ch].vol, last[ch].volp) = volume_column(v);
                (c.vol, c.volp) = (last[ch].vol, last[ch].volp);
            }
            if mask & 8 != 0 {
                let (Ok(f), Ok(p)) = (pr.u8(), pr.u8()) else { break };
                (last[ch].fx, last[ch].param) = s3m_effect(f, p, true);
                (c.fx, c.param) = (last[ch].fx, last[ch].param);
            }
            if mask & 16 != 0 {
                c.note = last[ch].note;
            }
            if mask & 32 != 0 {
                c.instr = last[ch].instr;
            }
            if mask & 64 != 0 {
                (c.vol, c.volp) = (last[ch].vol, last[ch].volp);
            }
            if mask & 128 != 0 {
                (c.fx, c.param) = (last[ch].fx, last[ch].param);
            }
            if !c.is_empty() {
                used = used.max(ch + 1);
            }
            cells.push((row, ch, c));
        }
        raw.push((rows, cells));
    }
    // Channels: as many as the patterns use.
    let nch = used.max(1);
    let patterns = raw
        .into_iter()
        .map(|(rows, cells)| {
            let mut p = Pattern::new(rows, nch);
            for (row, ch, c) in cells {
                if let Some(slot) = p.cell_mut(row, ch) {
                    *slot = c;
                }
            }
            Arc::new(p)
        })
        .collect();
    let channels = (0..nch)
        .map(|i| {
            let p = pans[i];
            let surround = p & 0x7F == 100;
            Channel {
                pan: if surround { 128 } else { (u16::from((p & 0x7F).min(64)) * 255 / 64) as u8 },
                volume: vols[i].min(64),
                surround,
                muted: p & 0x80 != 0,
                name: String::new(),
                effect: None,
            }
        })
        .collect();
    let orders = orders_raw
        .iter()
        .map(|&o| match o {
            254 => ORDER_SKIP,
            255 => ORDER_END,
            o => u16::from(o),
        })
        .take_while(|&o| o != ORDER_END)
        .collect();
    let msg = if special & 1 != 0 && msg_len > 0 { data.get(msg_off..msg_off.saturating_add(msg_len).min(data.len())).map(message).unwrap_or_default() } else { String::new() };

    let mut m = Module {
        title,
        message: msg,
        format: if cwt == 0x0889 { "mptm" } else { "it" }.into(),
        format_name: "Impulse Tracker".into(),
        tracker: tracker_name(cwt, cmwt),
        kind: Kind::It,
        flags: Flags { linear: flags & 8 != 0, instruments: flags & 4 != 0, old_effects: flags & 16 != 0, compat_gxx: flags & 32 != 0, ..Default::default() },
        speed: speed.max(1),
        tempo: u16::from(tempo.max(32)),
        global_volume: gv.min(128),
        mix_volume: mv.min(128),
        separation: if flags & 1 != 0 { sep.min(128) } else { 0 },
        channels,
        orders,
        patterns,
        samples,
        instruments,
        ..Default::default()
    };
    if m.format == "mptm" {
        m.format_name = "OpenMPT".into();
    }
    m.tidy();
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_block_decompresses_to_silence_not_a_panic() {
        assert_eq!(decompress(&[0, 0], 10, false, false), vec![0; 10]);
        assert_eq!(decompress(&[], 10, true, true), Vec::<i16>::new());
    }
}
