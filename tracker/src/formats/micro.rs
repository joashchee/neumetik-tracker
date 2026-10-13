//! The home micros' trackers: ProTracker 3.6's IFF modules (the Amiga),
//! Archimedes Tracker and Digital Symphony (Acorn's Archimedes), and
//! Megatracker (the Atari Falcon).

use std::sync::Arc;

use super::common::{finetune_c5, mod_effect};
use crate::io::{bad, decode, Pcm, Reader, Result};
use crate::model::*;

/// A little-endian bit reader, low bits first (Digital Symphony's LZW
/// and sigma-delta packing; Digitrakker's and X-Tracker's samples).
pub(super) struct Bits<'a> {
    b: &'a [u8],
    pos: usize,
    buf: u32,
    have: u32,
}

impl<'a> Bits<'a> {
    pub(super) fn new(b: &'a [u8]) -> Bits<'a> {
        Bits { b, pos: 0, buf: 0, have: 0 }
    }

    pub(super) fn read(&mut self, n: u32) -> Option<u32> {
        while self.have < n {
            let byte = *self.b.get(self.pos)?;
            self.pos += 1;
            self.buf |= u32::from(byte) << self.have;
            self.have += 8;
        }
        let v = self.buf & ((1 << n) - 1);
        self.buf >>= n;
        self.have -= n;
        Some(v)
    }

    /// Bytes taken, rounded up to a multiple of four.
    fn used_aligned(&self) -> usize {
        (self.pos + 3) & !3
    }
}

/// μ-law (G.711) as 16-bit.
fn ulaw(v: u8) -> i16 {
    let u = !v;
    let exp = (u >> 4) & 7;
    let mag = ((i32::from(u & 0x0F) << 3) + 0x84) << exp;
    let s = mag - 0x84;
    (if u & 0x80 != 0 { -s } else { s }) as i16
}

// ---- ProTracker 3.6 ----
//
// An IFF FORM MODL around a MOD: VERS (the version), INFO (title,
// counts, tempo), CMNT (the author and comments), PTDT (the MOD itself,
// tagged PATT). Each chunk's size counts its own header, and the first's
// counts the MODL too.

pub fn probe_pt36(d: &[u8]) -> bool {
    d.len() >= 20 && &d[..4] == b"FORM" && &d[8..12] == b"MODL"
}

pub fn load_pt36(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 12);
    let (mut info, mut comment, mut version, mut module) = (None, None, String::new(), None);
    let mut first = true;
    while r.left() >= 8 {
        let id = r.tag()?;
        let size = (r.u32be()? as usize).saturating_sub(if first { 12 } else { 8 });
        first = false;
        if &id == b"PTDT" {
            // Files with samples over 64K say too little here: the
            // module runs to the end.
            module = Some(&d[r.pos..]);
            break;
        }
        let body = r.bytes_upto(size);
        match &id {
            b"VERS" if body.len() > 6 && &body[4..6] == b"PT" => version = crate::io::text(&body[6..]),
            b"INFO" if body.len() >= 64 => info = Some(body),
            b"CMNT" => comment = Some(body),
            _ => {}
        }
    }
    let Some(module) = module else {
        return bad("it has no module in it");
    };
    let mut m = super::protracker::load(module, false)?;
    m.format = "pt36".into();
    m.format_name = "ProTracker 3.6".into();
    m.tracker = format!("ProTracker {}", if version.is_empty() { "3.6" } else { &version });
    if let Some(info) = info {
        let name = crate::io::text(&info[..32]);
        if !name.is_empty() {
            m.title = name;
        }
        let tempo = u16::from_be_bytes([info[40], info[41]]);
        let cia = u16::from_be_bytes([info[42], info[43]]) & 0x100 != 0;
        m.flags.vblank = !cia;
        if cia && tempo >= 32 {
            m.tempo = tempo.min(255);
        }
    }
    if let Some(c) = comment {
        let author = crate::io::text(&c[..c.len().min(32)]);
        if author != "UNNAMED AUTHOR" {
            m.artist = author;
        }
        if c.len() > 32 {
            // Lines of 40 characters.
            m.message = c[32..].chunks(40).map(crate::io::text).collect::<Vec<_>>().join("\n").trim_end().to_string();
        }
    }
    Ok(m)
}

// ---- Archimedes Tracker ----
//
// MUSX, then little-endian IFF-like chunks: TINF (the date), MVOX (the
// channels), STER (their stereo places), MNAM and ANAM (title, author),
// MLEN, PNUM and PLEN (the song's length, patterns, each one's rows),
// SEQU (the orders), a PATT for each pattern and a SAMP for each sample.
// Volumes are logarithmic, 0 to 255, and samples in the VIDC's own
// 8-bit logarithmic coding.

/// A logarithmic volume (0 to 255, 32 steps an octave) as 0 to 64.
fn arch_volume(v: u8) -> u8 {
    if v == 0 {
        0
    } else {
        (64.0 * 2f64.powf(-(255.0 - f64::from(v)) / 32.0)).round() as u8
    }
}

/// A VIDC sample byte: the sign in bit 0, then a μ-law-like magnitude
/// (eight chords of sixteen steps), as 16-bit.
fn vidc(v: u8) -> i16 {
    let m = v >> 1;
    let (chord, step) = (m >> 4, i32::from(m & 15));
    let mag = (((2 * step + 33) << chord) - 33) * 4;
    (if v & 1 != 0 { -mag } else { mag }) as i16
}

/// A VIDC sample byte as 16-bit, for the other Archimedes formats.
pub(crate) fn vidc_point(v: u8) -> i16 {
    vidc(v)
}

pub fn probe_musx(d: &[u8]) -> bool {
    d.len() >= 16 && &d[..4] == b"MUSX" && d[8..12].iter().all(u8::is_ascii_uppercase)
}

/// A stereo place, 1 (left) to 7 (right), as panning.
fn arch_pan(s: u8) -> u8 {
    (42 * u16::from(s) - 40) as u8
}

fn arch_effect(fx: u8, p: u8) -> (Fx, u8) {
    match fx {
        0x00..=0x03 => mod_effect(fx, p, false),
        0x0B => mod_effect(0x0D, p, false),
        0x0C if p <= 64 => (Fx::Volume, p),
        // 1 left, 4 the middle, 7 right.
        0x0E | 0x19 if (1..8).contains(&p) => (Fx::Panning, arch_pan(p)),
        // Slides of the logarithmic volume: about as far in the linear.
        0x10 if p > 0 => (Fx::VolSlide, p.min(15) << 4),
        0x11 if p > 0 => (Fx::VolSlide, p.min(15)),
        0x13 => (Fx::PositionJump, p),
        0x1C => mod_effect(0x0F, p, false),
        0x1F => (Fx::Volume, arch_volume(p)),
        _ => (Fx::None, 0),
    }
}

pub fn load_musx(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 8);
    let (mut nch, mut ster, mut title, mut artist) = (0usize, [0u8; 8], String::new(), String::new());
    let (mut len, mut rows, mut seq) = (0usize, [64u8; 64], Vec::new());
    let (mut patterns, mut samples) = (Vec::new(), Vec::new());
    while r.left() >= 8 {
        let id = r.tag()?;
        let size = r.u32le()? as usize;
        let body = r.bytes_upto(size);
        let le32 = |b: &[u8]| b.get(..4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
        match &id {
            b"MVOX" => nch = le32(body).min(8),
            b"STER" => ster[..body.len().min(8)].copy_from_slice(&body[..body.len().min(8)]),
            b"MNAM" => title = crate::io::text(body),
            b"ANAM" => artist = crate::io::text(body),
            b"MLEN" => len = le32(body).min(128),
            b"PLEN" => rows[..body.len().min(64)].copy_from_slice(&body[..body.len().min(64)]),
            b"SEQU" => seq = body.to_vec(),
            b"PATT" if nch > 0 && patterns.len() < 64 => {
                let n = usize::from(rows[patterns.len()]).max(1);
                let mut p = Pattern::new(n, nch);
                for (i, c) in body.chunks_exact(4).take(n * nch).enumerate() {
                    let (fx, param) = arch_effect(c[1], c[0]);
                    p.cells[i] = Cell { note: if c[3] > 0 { c[3].saturating_add(48).min(NOTE_MAX) } else { 0 }, instr: c[2], fx, param, ..Default::default() };
                }
                patterns.push(Arc::new(p));
            }
            b"SAMP" if samples.len() < 36 => samples.push(arch_sample(body)),
            _ => {}
        }
    }
    if nch == 0 {
        return bad("it doesn't say how many channels it has");
    }
    let mut m = Module {
        title,
        artist,
        format: "musx".into(),
        format_name: "Archimedes Tracker".into(),
        tracker: "Archimedes Tracker".into(),
        kind: Kind::Mod,
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: (0..nch)
            .map(|c| Channel {
                pan: if (1..8).contains(&ster[c]) {
                    arch_pan(ster[c])
                } else if matches!(c % 4, 0 | 3) {
                    192
                } else {
                    64
                },
                ..Default::default()
            })
            .collect(),
        orders: seq.iter().take(len.max(1)).map(|&o| u16::from(o)).collect(),
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

/// A SAMP chunk: SNAM, SVOL, SLEN, ROFS (the loop's start), RLEN (its
/// length), SDAT.
fn arch_sample(b: &[u8]) -> Sample {
    let mut r = Reader::new(b);
    let mut s = Sample::empty();
    let (mut len, mut ls, mut ll, mut data): (usize, usize, usize, &[u8]) = (0, 0, 0, &[]);
    while r.left() >= 8 {
        let (Ok(id), Ok(size)) = (r.tag(), r.u32le()) else {
            break;
        };
        let body = r.bytes_upto(size as usize);
        let le32 = || body.get(..4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
        match &id {
            b"SNAM" => s.name = crate::io::text(body),
            b"SVOL" => s.volume = arch_volume(le32() as u8),
            b"SLEN" => len = le32(),
            b"ROFS" => ls = le32(),
            b"RLEN" => ll = le32(),
            b"SDAT" => data = body,
            _ => {}
        }
    }
    let pcm: Vec<i16> = data.iter().take(len).map(|&v| vidc(v)).collect();
    let frames = pcm.len();
    s.wave = Arc::new(Wave::new(16, 1, pcm));
    if ls < frames {
        if ll > 2 {
            s.looped = Loop::forward(ls as u32, (ls + ll.min(frames - ls)) as u32);
        } else if ll == 2 && ls > 0 {
            // A repeat of 2 from past the start: to the end.
            s.looped = Loop::forward(ls as u32, frames as u32);
        }
    }
    s
}

// ---- Digital Symphony ----
//
// A magic, the channels, orders and tracks, 63 samples' lengths, the
// title, which commands are allowed, the orders (a track for each
// channel), tracks of 64 four-byte rows, then each sample's name,
// loop, volume and finetune, and its data, raw or packed (13-bit LZW,
// μ-law, sigma-delta). Blocks may be LZW-packed too.

const DSYM_MAGIC: &[u8] = b"\x02\x01\x13\x13\x14\x12\x01\x0B";

pub fn probe_dsym(d: &[u8]) -> bool {
    d.len() >= 17 + 72 && &d[..8] == DSYM_MAGIC && d[8] <= 1 && (1..=8).contains(&d[9]) && u16::from_le_bytes([d[10], d[11]]) <= 4096 && u16::from_le_bytes([d[12], d[13]]) <= 4096
}

/// 13-bit LZW (codes 256 reset, 257 end), `size` bytes out (fewer when
/// the file's cut short); the reader left at the next multiple of four
/// bytes.
fn dsym_lzw(r: &mut Reader, size: usize) -> Vec<u8> {
    const MAX: usize = 1 << 13;
    let mut bits = Bits::new(&r.data[r.pos..]);
    let mut out = Vec::with_capacity(size.min(r.left() * 50));
    let mut prev_of = vec![MAX as u16; MAX];
    let mut value = vec![0u8; MAX];
    for (i, v) in value.iter_mut().enumerate().take(256) {
        *v = i as u8;
    }
    let mut stack = vec![0u8; MAX];
    let (mut width, mut prev, mut next) = (9u32, 0u16, 257u16);
    let mut cut = false;
    loop {
        let Some(code) = bits.read(width) else {
            cut = true;
            break;
        };
        let code = code as u16;
        if code == 257 || code > next || out.len() >= size {
            break;
        }
        if code == 256 {
            (width, prev, next) = (9, 0, 257);
            continue;
        }
        let mut c = if code < next { code } else { prev };
        let mut at = MAX;
        loop {
            at -= 1;
            stack[at] = value[usize::from(c)];
            c = prev_of[usize::from(c)];
            if usize::from(c) >= MAX || at == 0 {
                break;
            }
        }
        out.extend_from_slice(&stack[at..]);
        if code == next {
            out.push(stack[at]);
        }
        if usize::from(next) < MAX {
            if out.len() >= size {
                continue;
            }
            value[usize::from(next)] = stack[at];
            prev_of[usize::from(next)] = prev;
            next += 1;
            if usize::from(next) != MAX && u32::from(next) == 1 << width {
                width += 1;
            }
        }
        prev = code;
    }
    r.pos = if cut { r.data.len() } else { (r.pos + bits.used_aligned()).min(r.data.len()) };
    out.truncate(size);
    out
}

/// Sigma-delta: a run length, then differences in a width that grows
/// and shrinks; `size` bytes out.
fn dsym_sigma_delta(r: &mut Reader, size: usize) -> Option<Vec<u8>> {
    let max_run = r.u8().ok()?.max(1);
    let mut bits = Bits::new(&r.data[r.pos..]);
    let size = size.min(r.left().saturating_mul(8));
    let mut out = Vec::with_capacity(size);
    let (mut run, mut width) = (max_run, 8u32);
    let mut acc = bits.read(8)? as u8;
    out.push(acc);
    while out.len() < size {
        let Some(v) = bits.read(width) else { break };
        if v == 0 {
            if width >= 9 {
                break;
            }
            width += 1;
            run = max_run;
            continue;
        }
        acc = if v & 1 != 0 { acc.wrapping_sub((v >> 1) as u8) } else { acc.wrapping_add((v >> 1) as u8) };
        out.push(acc);
        if v >> (width - 1) != 0 {
            run = max_run;
            continue;
        }
        run -= 1;
        if run == 0 {
            width = (width - 1).max(1);
            run = max_run;
        }
    }
    r.pos += bits.used_aligned();
    r.pos = r.pos.min(r.data.len());
    Some(out)
}

/// A block: a packing byte (0 raw, 1 LZW), then `size` bytes' worth.
fn dsym_block(r: &mut Reader, size: usize) -> Result<Vec<u8>> {
    let packed = r.u8()?;
    let out = match packed {
        0 => r.bytes(size)?.to_vec(),
        1 => dsym_lzw(r, size),
        _ => return bad("a block is packed in a way Digital Symphony doesn't"),
    };
    if out.len() < size {
        return bad("a packed block ends early");
    }
    Ok(out)
}

pub fn load_dsym(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 8);
    let version = r.u8()?;
    let nch = usize::from(r.u8()?);
    let norders = usize::from(r.u16le()?);
    let ntracks = usize::from(r.u16le()?);
    let info_len = r.u24le()? as usize;
    let mut heads = Vec::with_capacity(63);
    for _ in 0..63 {
        let name_len = r.u8()?;
        let len = if name_len & 0x80 == 0 { (r.u24le()? as usize) << 1 } else { 0 };
        heads.push((name_len, len));
    }
    let title = r.pstring()?;
    let allowed = r.bytes(8)?.to_vec();
    let seq = if norders > 0 { dsym_block(&mut r, norders * nch * 2)? } else { Vec::new() };
    let mut tracks = Vec::with_capacity(ntracks * 256);
    for start in (0..ntracks).step_by(2000) {
        tracks.extend(dsym_block(&mut r, (ntracks - start).min(2000) * 256)?);
    }
    let mut patterns = Vec::with_capacity(norders);
    for o in 0..norders {
        let mut p = Pattern::new(64, nch);
        for ch in 0..nch {
            let t = usize::from(u16::from_le_bytes([seq[(o * nch + ch) * 2], seq[(o * nch + ch) * 2 + 1]]));
            if t >= ntracks {
                continue;
            }
            for row in 0..64 {
                let b = &tracks[t * 256 + row * 4..t * 256 + row * 4 + 4];
                let cell = &mut p.cells[row * nch + ch];
                let note = b[0] & 0x3F;
                cell.note = if note > 0 { note + 48 } else { 0 };
                cell.instr = (b[0] >> 6) | ((b[1] & 0x0F) << 2);
                let cmd = (b[1] >> 6) | ((b[2] & 0x0F) << 2);
                let param = u16::from(b[2] >> 4) | u16::from(b[3]) << 4;
                if allowed[usize::from(cmd >> 3)] & (1 << (cmd & 7)) == 0 || (cmd == 0 && param == 0) {
                    continue;
                }
                dsym_effect(cell, cmd, param);
                if cmd == 0x2B {
                    // A line jump: break to the row, in this pattern again.
                    let at = row * nch;
                    if let Some(free) = (0..nch).find(|&c| c != ch && p.cells[at + c].fx == Fx::None) {
                        p.cells[at + free].fx = Fx::PositionJump;
                        p.cells[at + free].param = o.min(255) as u8;
                    }
                }
            }
        }
        patterns.push(Arc::new(p));
    }
    let mut samples = Vec::with_capacity(63);
    for &(name_len, len) in &heads {
        match dsym_sample(&mut r, name_len, len) {
            Ok(s) => samples.push(s),
            // A file cut short: the samples it has.
            Err(_) if r.eof() => break,
            Err(e) => return Err(e),
        }
    }
    samples.resize(63, Sample::empty());
    let message = if info_len > 0 { dsym_block(&mut r, info_len).map(|b| crate::io::message(&b)).unwrap_or_default() } else { String::new() };
    let mut m = Module {
        title,
        message,
        format: "dsym".into(),
        format_name: "Digital Symphony".into(),
        tracker: format!("Digital Symphony (version {version})"),
        kind: Kind::Mod,
        flags: Flags { amiga_limits: true, ..Default::default() },
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: (0..nch).map(|c| Channel { pan: if matches!(c & 3, 1 | 2) { 64 } else { 192 }, ..Default::default() }).collect(),
        orders: (0..norders.min(usize::from(ORDER_SKIP))).map(|o| o as u16).collect(),
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

/// A sample's name, loop, volume and finetune, and its data.
fn dsym_sample(r: &mut Reader, name_len: u8, len: usize) -> Result<Sample> {
    let mut s = Sample { name: r.text(usize::from(name_len & 0x3F))?, ..Sample::empty() };
    if name_len & 0x80 != 0 {
        return Ok(s);
    }
    let ls = (r.u24le()? as usize) << 1;
    let ll = (r.u24le()? as usize) << 1;
    s.volume = r.u8()?.min(64);
    s.c5 = finetune_c5(r.u8()?);
    if len > 0 {
        let (bits, pcm) = match r.u8()? {
            0 => (16, r.bytes_upto(len).iter().map(|&v| ulaw((v << 7) | (!v >> 1))).collect()),
            1 => (8, decode(&dsym_lzw(r, len), Pcm::D8, len)),
            2 => (8, decode(r.bytes_upto(len), Pcm::S8, len)),
            3 => (16, decode(r.bytes_upto(len * 2), Pcm::S16le, len)),
            4 => (8, decode(&dsym_sigma_delta(r, len).unwrap_or_default(), Pcm::U8, len)),
            5 => (16, dsym_sigma_delta(r, len).unwrap_or_default().iter().map(|&v| ulaw(v ^ if v & 0x80 != 0 { 0x7F } else { 0 })).collect()),
            _ => return bad("a sample is packed in a way Digital Symphony doesn't"),
        };
        s.wave = Arc::new(Wave::new(bits, 1, pcm));
    }
    // Its loop lasts while the note's held: 32 (unset sample repeat)
    // lets it go.
    if ll > 2 {
        s.sustain = Loop::forward(ls as u32, (ls + ll) as u32);
    }
    Ok(s)
}

/// A Digital Symphony command (six bits) and its 12-bit parameter.
fn dsym_effect(cell: &mut Cell, cmd: u8, param: u16) {
    let p = param as u8;
    let hi = (param >> 8) as u8;
    let ex = |x: u8, y: u8| (Fx::ModEx, ((x & 15) << 4) | (y & 15));
    (cell.fx, cell.param) = match cmd {
        // Arpeggio and the slides, with a volume slide in the top bits.
        0x00 | 0x01 | 0x02 | 0x20 | 0x21 | 0x22 => {
            if hi != 0 {
                (cell.vol, cell.volp) = (if cmd < 0x20 { Vol::VolSlideUp } else { Vol::VolSlideDown }, hi.min(15));
            }
            mod_effect(cmd & 0x0F, p, false)
        }
        0x03..=0x07 | 0x0C => mod_effect(cmd, p, false),
        0x09 => (Fx::Offset, (param >> 1).min(255) as u8),
        // A volume slide, or a fine slide with one in the volume column.
        0x0A | 0x2A if param < 0xFF => mod_effect(0x0A, p, false),
        0x0A | 0x2A => {
            if param & 0xF0 != 0 {
                (cell.vol, cell.volp) = (Vol::VolSlideUp, (p >> 4) & 15);
            } else {
                (cell.vol, cell.volp) = (Vol::VolSlideDown, p & 15);
            }
            ex(if cmd < 0x20 { 1 } else { 2 }, hi)
        }
        0x0B => (Fx::PositionJump, param.min(255) as u8),
        0x0F if param > 0 => (Fx::Speed, param.min(255) as u8),
        0x0D => (Fx::PatternBreak, if p > 63 { 0 } else { p }),
        0x10 | 0x13 | 0x14 | 0x15 | 0x17 | 0x1F => ex(cmd, p),
        0x16 | 0x19 | 0x1C | 0x1D | 0x1E => ex(cmd, param.min(15) as u8),
        // A fine slide and a fine volume slide; or the volume alone.
        0x11 | 0x12 | 0x1A | 0x1B if p != 0 => {
            if hi != 0 {
                (cell.vol, cell.volp) = (if cmd >= 0x1A { Vol::FineVolDown } else { Vol::FineVolUp }, hi.min(15));
            }
            ex(if cmd == 0x11 || cmd == 0x1A { 1 } else { 2 }, p)
        }
        0x11 | 0x12 | 0x1A | 0x1B => ex(if cmd >= 0x1A { 0xB } else { 0xA }, hi),
        0x2F if param > 0 => (Fx::Tempo, ((param + 4).max(8) / 8).clamp(32, 255) as u8),
        0x2B => (Fx::PatternBreak, p.min(63)),
        0x30 if param & 7 != 0 => (Fx::Panning, [0x00, 0x00, 0x2B, 0x56, 0x80, 0xAA, 0xD4, 0xFF][usize::from(param & 7)]),
        0x30 if param >> 4 != 0x80 => {
            let v = (param >> 4) as u8;
            (Fx::Panning, if v < 0x80 { v + 0x80 } else { 0xFF - v })
        }
        // Unset sample repeat: the note let go.
        0x32 if cell.note == 0 => {
            cell.note = NOTE_OFF;
            (Fx::None, 0)
        }
        0x32 => (Fx::KeyOff, 0),
        _ => (Fx::None, 0),
    };
}

// ---- Megatracker ----
//
// "MGT", a version, "\xBDMCS", counts, then offsets to the song, its
// order list, the instruments, the patterns (a track for each channel)
// and the tracks, packed a row at a time. Big-endian throughout.

pub fn probe_mgt(d: &[u8]) -> bool {
    d.len() >= 64 && &d[..3] == b"MGT" && &d[4..8] == b"\xBDMCS"
}

pub fn load_mgt(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 3);
    let version = r.u8()?;
    r.skip(4)?;
    let nch = usize::from(r.u16be()?);
    let _songs = r.u16be()?;
    let _len = r.u16be()?;
    let npat = usize::from(r.u16be()?);
    let ntrk = usize::from(r.u16be()?);
    let nins = usize::from(r.u16be()?);
    r.skip(6)?;
    let song_at = r.u32be()? as usize;
    let _seq_at = r.u32be()?;
    let ins_at = r.u32be()? as usize;
    let pat_at = r.u32be()? as usize;
    let trk_at = r.u32be()? as usize;
    if nch == 0 || nch > 64 || npat > 1024 || nins > 64 {
        return bad("its header doesn't add up");
    }
    r.seek(song_at)?;
    let title = r.text(32)?;
    let seq_at = r.u32be()? as usize;
    let len = usize::from(r.u16be()?);
    let restart = r.u16be()?;
    let tempo = r.u8()?;
    let speed = r.u8()?;
    r.seek(seq_at)?;
    let mut orders = Vec::with_capacity(len);
    for _ in 0..len {
        orders.push(r.u16be()?.min(ORDER_SKIP - 1));
    }
    r.seek(ins_at)?;
    let mut heads = Vec::with_capacity(nins);
    for _ in 0..nins {
        let name = r.text(32)?;
        let at = r.u32be()? as usize;
        let len = r.u32be()? as usize;
        let ls = r.u32be()?;
        let ll = r.u32be()?;
        r.skip(8)?;
        let c5 = r.u32be()?;
        let vol = r.u16be()? >> 4;
        r.skip(2)?;
        let flags = r.u8()?;
        r.skip(15)?;
        heads.push((name, at, len, ls, ll, c5, vol, flags));
    }
    let track = |t: usize| -> Option<Pattern> {
        let at = trk_at + t * 4;
        let off = u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?) as usize;
        let mut r = Reader::at(d, off);
        let rows = usize::from(r.u16be().ok()?).min(256);
        let mut p = Pattern::new(rows, 1);
        let mut row = 0;
        while row < rows {
            let Ok(b) = r.u8() else { break };
            row += usize::from(b & 3);
            if row >= rows {
                break;
            }
            let mut field = |bit: u8| if b & bit != 0 { r.u8().ok() } else { None };
            let note = field(0x04).unwrap_or(0);
            let instr = field(0x08).unwrap_or(0);
            let vol = field(0x10).unwrap_or(0);
            let fx = field(0x20).unwrap_or(0);
            let param = field(0x40).unwrap_or(0);
            let _param2 = field(0x80);
            let cell = &mut p.cells[row];
            cell.instr = instr;
            cell.note = match note {
                1 => NOTE_OFF,
                12..=118 => note + 1,
                _ => 0,
            };
            (cell.fx, cell.param) = match fx {
                0x00..=0x0F => mod_effect(fx, param, true),
                0x13 | 0x14 | 0x15 | 0x17 | 0x1C | 0x1D | 0x1E => (Fx::ModEx, ((fx & 15) << 4) | (param & 15)),
                _ => (Fx::None, 0),
            };
            // FastTracker 2's volume column.
            let (x, y) = (vol >> 4, vol & 15);
            (cell.vol, cell.volp) = match x {
                1..=4 => (Vol::Volume, vol - 0x10),
                5 if y == 0 => (Vol::Volume, 64),
                6 => (Vol::VolSlideDown, y),
                7 => (Vol::VolSlideUp, y),
                8 => (Vol::FineVolDown, y),
                9 => (Vol::FineVolUp, y),
                0xA => (Vol::VibratoSpeed, y),
                0xB => (Vol::VibratoDepth, y),
                0xC => (Vol::Panning, y * 4 + 2),
                0xD => (Vol::PanSlideLeft, y),
                0xE => (Vol::PanSlideRight, y),
                0xF => (Vol::TonePorta, y),
                _ => (Vol::None, 0),
            };
            row += 1;
        }
        Some(p)
    };
    r.seek(pat_at)?;
    let mut patterns = Vec::with_capacity(npat);
    for _ in 0..npat {
        let rows = usize::from(r.u16be()?).clamp(1, 256);
        let mut p = Pattern::new(rows, nch);
        for ch in 0..nch {
            // Track 0 is the empty one.
            let t = usize::from(r.u16be()?).saturating_sub(1);
            if t == 0 || t >= ntrk {
                continue;
            }
            let Some(tp) = track(t) else { continue };
            for row in 0..rows.min(tp.rows) {
                p.cells[row * nch + ch] = tp.cells[row];
            }
        }
        patterns.push(Arc::new(p));
    }
    let samples = heads
        .into_iter()
        .map(|(name, at, len, ls, ll, c5, vol, flags)| {
            let pcm = decode(d.get(at..).unwrap_or(&[]), Pcm::S8, len);
            let kind = if flags & 2 != 0 { LoopKind::PingPong } else { LoopKind::Forward };
            let looped = if flags & 3 != 0 { Loop { kind, start: ls, end: ls.saturating_add(ll) } } else { Loop::default() };
            Sample { name, wave: Arc::new(Wave::new(8, 1, pcm)), c5: if c5 == 0 { 8363 } else { c5.min(2_000_000) }, volume: vol.min(64) as u8, looped, ..Sample::empty() }
        })
        .collect();
    let mut m = Module {
        title,
        format: "mgt".into(),
        format_name: "Megatracker".into(),
        tracker: format!("Megatracker {}.{}", version >> 4, version & 15),
        kind: Kind::Xm,
        speed: if speed == 0 { 6 } else { speed },
        tempo: u16::from(tempo),
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        restart,
        channels: (0..nch).map(|c| Channel { pan: if matches!(c % 4, 0 | 3) { 64 } else { 192 }, ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    if usize::from(m.restart) >= m.orders.len() {
        m.restart = 0;
    }
    m.tidy();
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulaw_and_vidc_are_symmetric_and_loud_at_the_top() {
        assert_eq!(ulaw(0xFF), 0);
        assert_eq!(ulaw(0x80), 32124);
        assert_eq!(ulaw(0x00), -32124);
        assert_eq!(vidc(0), 0);
        assert_eq!(vidc(0xFE), 32124);
        assert_eq!(vidc(0xFF), -32124);
    }

    #[test]
    fn archimedes_volumes_are_logarithmic() {
        assert_eq!(arch_volume(255), 64);
        assert_eq!(arch_volume(223), 32);
        assert_eq!(arch_volume(0), 0);
    }

    #[test]
    fn lzw_reads_literals_and_repeats() {
        // Codes 'a' 'b' 258 ("ab") then end, nine bits each, low bits first.
        let codes = [97u32, 98, 258, 257];
        let mut bytes = vec![0u8; 8];
        for (i, c) in codes.iter().enumerate() {
            for b in 0..9 {
                if c >> b & 1 != 0 {
                    bytes[(i * 9 + b) / 8] |= 1 << ((i * 9 + b) % 8);
                }
            }
        }
        let mut r = Reader::new(&bytes);
        assert_eq!(dsym_lzw(&mut r, 4), b"abab");
        assert_eq!(r.pos, 8);
    }
}
