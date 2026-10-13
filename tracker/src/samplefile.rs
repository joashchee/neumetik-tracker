//! Sample (and instrument) files, read and written: WAV (with its
//! `smpl` loops), AIFF and AIFC, the Amiga's IFF 8SVX (and 16SV), FLAC,
//! Impulse Tracker's ITS, FastTracker 2's XI instruments, and the Gravis
//! UltraSound's PAT patches. Each from its public description; FLAC's
//! decoder and encoder (fixed predictors, Rice-coded residuals) are
//! written here too.

use std::sync::Arc;

use crate::formats::common::relative_c5;
use crate::io::{bad, decode, Error, Pcm, Reader, Result};
use crate::model::*;

/// What a file held: its samples, and the instrument that maps them,
/// when it's an instrument file (XI, a PAT with several waves).
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    pub samples: Vec<Sample>,
    pub instrument: Option<Instrument>,
}

/// The kinds written, by extension.
pub const KINDS: &[(&str, &str)] = &[("wav", "WAV"), ("aiff", "AIFF"), ("8svx", "IFF 8SVX"), ("flac", "FLAC"), ("its", "Impulse Tracker sample"), ("xi", "FastTracker 2 instrument"), ("pat", "Gravis patch")];

/// Every extension read.
pub const EXTENSIONS: &[&str] = &["wav", "wave", "aif", "aiff", "aifc", "8svx", "16sv", "iff", "svx", "flac", "its", "xi", "pat"];

/// Reads a sample file, whatever it is (by its marks, then its name).
pub fn read(data: &[u8], name: &str) -> Result<Loaded> {
    let one = |s: Sample| Loaded { samples: vec![s], instrument: None };
    let stem = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = stem.rsplit_once('.').map_or(stem, |(a, _)| a);
    let named = |mut s: Sample| {
        if s.name.trim().is_empty() {
            s.name = stem.chars().take(32).collect();
        }
        s
    };
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WAVE" {
        return read_wav(data).map(named).map(one);
    }
    if data.len() >= 12 && &data[..4] == b"FORM" && (&data[8..12] == b"AIFF" || &data[8..12] == b"AIFC") {
        return read_aiff(data).map(named).map(one);
    }
    if data.len() >= 12 && &data[..4] == b"FORM" && (&data[8..12] == b"8SVX" || &data[8..12] == b"16SV") {
        return read_8svx(data).map(named).map(one);
    }
    if data.len() >= 4 && &data[..4] == b"fLaC" {
        return read_flac(data).map(named).map(one);
    }
    if data.len() >= 80 && &data[..4] == b"IMPS" {
        return crate::formats::it::sample(data, 0, 0x0214).map(named).map(one);
    }
    if data.len() >= 21 && &data[..21] == b"Extended Instrument: " {
        return read_xi(data);
    }
    if data.len() >= 22 && (data.starts_with(b"GF1PATCH110") || data.starts_with(b"GF1PATCH100")) {
        return read_pat(data);
    }
    bad("it isn't a sample file this reads (WAV, AIFF, 8SVX, FLAC, ITS, XI or PAT)")
}

/// Writes a sample as `kind` (an extension from `KINDS`). Returns the
/// file and what it couldn't keep.
pub fn write(s: &Sample, kind: &str) -> (Vec<u8>, Vec<String>) {
    let mut lost = Vec::new();
    let bytes = match kind {
        "aiff" | "aif" => write_aiff(s),
        "8svx" => {
            if s.wave.bits > 8 {
                lost.push("8SVX keeps 8 bits a point".into());
            }
            if s.wave.channels > 1 {
                lost.push("8SVX keeps one channel".into());
            }
            if s.looped.kind == LoopKind::PingPong {
                lost.push("8SVX's loops only go forward".into());
            }
            if s.sustain.on() {
                lost.push("8SVX has no sustain loop".into());
            }
            write_8svx(s)
        }
        "flac" => {
            if s.looped.on() || s.sustain.on() {
                lost.push("FLAC keeps no loop".into());
            }
            write_flac(&s.wave, s.c5)
        }
        "its" => write_its(s),
        "xi" => {
            if s.wave.channels > 1 {
                lost.push("XI keeps one channel".into());
            }
            if s.sustain.on() {
                lost.push("XI has no sustain loop".into());
            }
            write_xi(&Instrument::for_sample(&s.name, 1), std::slice::from_ref(s))
        }
        "pat" => {
            if s.wave.channels > 1 {
                lost.push("PAT keeps one channel".into());
            }
            write_pat(s)
        }
        _ => {
            if s.sustain.on() {
                lost.push("WAV keeps the loop, not the sustain loop".into());
            }
            write_wav(s)
        }
    };
    (bytes, lost)
}

// ---- WAV, with its loops ----

/// A WAV file with the loop its `smpl` chunk gives.
fn read_wav(data: &[u8]) -> Result<Sample> {
    let (wave, rate) = crate::wav::read(data)?;
    let mut s = Sample { wave: Arc::new(wave), c5: rate, ..Sample::empty() };
    let mut r = Reader::at(data, 12);
    while r.left() >= 8 {
        let id = r.tag()?;
        let len = r.u32le()? as usize;
        let start = r.pos;
        if &id == b"smpl" && len >= 36 {
            r.skip(12)?;
            let unity = r.u32le()?;
            let fraction = r.u32le()?;
            r.skip(8)?;
            let loops = r.u32le()?;
            r.skip(4)?;
            // The rate that plays middle C, from the note it's at as recorded.
            if unity < 128 && unity != 60 {
                let semis = 60.0 - f64::from(unity) - f64::from(fraction) / 4_294_967_296.0;
                s.c5 = (f64::from(rate) * 2f64.powf(semis / 12.0)).round() as u32;
            }
            if loops > 0 {
                r.skip(4)?;
                let kind = r.u32le()?;
                let a = r.u32le()?;
                let b = r.u32le()?;
                s.looped = Loop { kind: if kind == 1 { LoopKind::PingPong } else { LoopKind::Forward }, start: a, end: b.saturating_add(1) }.clamped(s.frames() as u32);
            }
        } else if &id == b"LIST" && len >= 4 && r.bytes(4)? == b"INFO" {
            {
                while r.pos + 8 <= start + len {
                    let sub = r.tag()?;
                    let n = r.u32le()? as usize;
                    let text = crate::io::text(r.bytes_upto(n));
                    if &sub == b"INAM" {
                        s.name = text.chars().take(32).collect();
                    } else if &sub == b"IART" || &sub == b"ICOP" {
                        add_credit(&mut s.credit, &text);
                    }
                    if n & 1 == 1 && r.pos < data.len() {
                        r.skip(1)?;
                    }
                }
            }
        }
        r.seek((start + len + (len & 1)).min(data.len()))?;
    }
    Ok(s)
}

fn write_wav(s: &Sample) -> Vec<u8> {
    let mut out = crate::wav::write_wave(&s.wave, s.c5);
    let mut extra = Vec::new();
    if s.looped.on() {
        let mut c = Vec::new();
        for v in [0u32, 0, 1_000_000_000 / s.c5.max(1), 60, 0, 0, 0, 1, 0, 0, u32::from(s.looped.kind == LoopKind::PingPong), s.looped.start, s.looped.end - 1, 0, 0] {
            c.extend_from_slice(&v.to_le_bytes());
        }
        extra.extend_from_slice(b"smpl");
        extra.extend_from_slice(&(c.len() as u32).to_le_bytes());
        extra.extend_from_slice(&c);
    }
    let mut info = Vec::new();
    for (tag, text) in [(b"INAM", &s.name), (b"IART", &s.credit)] {
        if !text.is_empty() {
            let mut b = text.as_bytes().to_vec();
            b.push(0);
            if b.len() & 1 == 1 {
                b.push(0);
            }
            info.extend_from_slice(tag);
            info.extend_from_slice(&(b.len() as u32).to_le_bytes());
            info.extend_from_slice(&b);
        }
    }
    if !info.is_empty() {
        extra.extend_from_slice(b"LIST");
        extra.extend_from_slice(&(info.len() as u32 + 4).to_le_bytes());
        extra.extend_from_slice(b"INFO");
        extra.extend_from_slice(&info);
    }
    out.extend_from_slice(&extra);
    let riff = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    out
}

// ---- AIFF and AIFC ----

/// An 80-bit IEEE extended number (AIFF's sample rate).
fn read_extended(b: &[u8]) -> f64 {
    let exp = i32::from(u16::from_be_bytes([b[0], b[1]]) & 0x7FFF);
    let sign = if b[0] & 0x80 != 0 { -1.0 } else { 1.0 };
    let mant = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
    if exp == 0 && mant == 0 {
        return 0.0;
    }
    sign * mant as f64 * 2f64.powi(exp - 16383 - 63)
}

fn write_extended(v: f64) -> [u8; 10] {
    let mut out = [0u8; 10];
    if v <= 0.0 {
        return out;
    }
    let exp = v.log2().floor() as i32;
    let mant = (v / 2f64.powi(exp - 63)) as u64;
    out[..2].copy_from_slice(&((exp + 16383) as u16).to_be_bytes());
    out[2..].copy_from_slice(&mant.to_be_bytes());
    out
}

fn read_aiff(data: &[u8]) -> Result<Sample> {
    let aifc = &data[8..12] == b"AIFC";
    let mut r = Reader::at(data, 12);
    let mut comm: Option<(u16, u32, u16, f64, [u8; 4])> = None;
    let mut sound: Option<&[u8]> = None;
    let mut markers: Vec<(u16, u32)> = Vec::new();
    let mut inst: Option<(i8, (u16, u16, u16))> = None;
    let mut name = String::new();
    let mut credit = String::new();
    while r.left() >= 8 {
        let id = r.tag()?;
        let len = r.u32be()? as usize;
        let start = r.pos;
        match &id {
            b"COMM" => {
                let ch = r.u16be()?;
                let frames = r.u32be()?;
                let bits = r.u16be()?;
                let rate = read_extended(r.bytes(10)?);
                let comp = if aifc && len >= 22 { r.tag()? } else { *b"NONE" };
                comm = Some((ch.max(1), frames, bits, rate, comp));
            }
            b"SSND" => {
                let offset = r.u32be()? as usize;
                r.skip(4)?;
                let body = &data[(start + 8).min(data.len())..(start + len).min(data.len())];
                sound = Some(body.get(offset..).unwrap_or(&[]));
            }
            b"MARK" => {
                let n = r.u16be()?;
                for _ in 0..n {
                    let id = r.u16be()?;
                    let pos = r.u32be()?;
                    // A Pascal string, padded to an even length.
                    let n = usize::from(r.u8()?);
                    r.skip(n + (n + 1) % 2)?;
                    markers.push((id, pos));
                }
            }
            b"INST" => {
                let base = r.i8()?;
                r.skip(7)?;
                let mode = r.u16be()?;
                let a = r.u16be()?;
                let b = r.u16be()?;
                inst = Some((base, (mode, a, b)));
            }
            b"NAME" => name = crate::io::text(r.bytes_upto(len)),
            b"AUTH" | b"(c) " => add_credit(&mut credit, &crate::io::text(r.bytes_upto(len))),
            _ => {}
        }
        r.seek((start + len + (len & 1)).min(data.len()))?;
    }
    let (ch, frames, bits, rate, comp) = comm.ok_or_else(|| Error::Bad("it has no COMM chunk".into()))?;
    let body = sound.unwrap_or(&[]);
    let points = frames as usize * usize::from(ch);
    let pcm: Vec<i16> = match (&comp, bits) {
        (b"NONE" | b"twos", 1..=8) => decode(body, Pcm::S8, points),
        (b"raw ", 1..=8) => decode(body, Pcm::U8, points),
        (b"NONE" | b"twos", 9..=16) => decode(body, Pcm::S16be, points),
        (b"sowt", 9..=16) => decode(body, Pcm::S16le, points),
        (b"NONE" | b"twos" | b"in24", 17..=24) => body.chunks_exact(3).take(points).map(|c| i16::from_be_bytes([c[0], c[1]])).collect(),
        (b"NONE" | b"twos" | b"in32", 25..=32) => body.chunks_exact(4).take(points).map(|c| i16::from_be_bytes([c[0], c[1]])).collect(),
        (b"fl32" | b"FL32", _) => body.chunks_exact(4).take(points).map(|c| (f32::from_be_bytes([c[0], c[1], c[2], c[3]]).clamp(-1.0, 1.0) * 32767.0) as i16).collect(),
        _ => return Err(Error::Unsupported(format!("AIFC's {} compression isn't read", String::from_utf8_lossy(&comp)))),
    };
    let keep = usize::from(ch).min(2);
    let pcm: Vec<i16> = if usize::from(ch) <= 2 { pcm } else { pcm.chunks_exact(usize::from(ch)).flat_map(|f| f[..2].to_vec()).collect() };
    let mut s = Sample { name: name.chars().take(32).collect(), credit, wave: Arc::new(Wave::new(if bits <= 8 { 8 } else { 16 }, keep as u8, pcm)), c5: rate.round().max(1.0) as u32, ..Sample::empty() };
    if let Some((base, (mode, a, b))) = inst {
        let at = |id: u16| markers.iter().find(|m| m.0 == id).map(|m| m.1);
        if let (Some(a), Some(b)) = (at(a), at(b)) {
            s.looped = Loop { kind: match mode {
                1 => LoopKind::Forward,
                2 => LoopKind::PingPong,
                _ => LoopKind::Off,
            }, start: a, end: b }
            .clamped(s.frames() as u32);
        }
        if base > 0 && base != 60 {
            s.c5 = (f64::from(s.c5) * 2f64.powf(f64::from(60 - i32::from(base)) / 12.0)).round() as u32;
        }
    }
    Ok(s)
}

fn chunk_be(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
    if body.len() & 1 == 1 {
        out.push(0);
    }
}

fn write_aiff(s: &Sample) -> Vec<u8> {
    let w = &s.wave;
    let ch = u16::from(w.channels.max(1));
    let bits = if w.bits == 8 { 8u16 } else { 16 };
    let mut body = Vec::new();
    let mut comm = Vec::new();
    comm.extend_from_slice(&ch.to_be_bytes());
    comm.extend_from_slice(&(w.frames() as u32).to_be_bytes());
    comm.extend_from_slice(&bits.to_be_bytes());
    comm.extend_from_slice(&write_extended(f64::from(s.c5)));
    chunk_be(&mut body, b"COMM", &comm);
    if !s.name.is_empty() {
        chunk_be(&mut body, b"NAME", s.name.as_bytes());
    }
    if !s.credit.is_empty() {
        chunk_be(&mut body, b"AUTH", s.credit.as_bytes());
    }
    if s.looped.on() {
        let mut mark = Vec::new();
        mark.extend_from_slice(&2u16.to_be_bytes());
        for (id, pos, name) in [(1u16, s.looped.start, "beg loop"), (2, s.looped.end, "end loop")] {
            mark.extend_from_slice(&id.to_be_bytes());
            mark.extend_from_slice(&pos.to_be_bytes());
            mark.push(name.len() as u8);
            mark.extend_from_slice(name.as_bytes());
            if name.len() % 2 == 0 {
                mark.push(0);
            }
        }
        chunk_be(&mut body, b"MARK", &mark);
        let mut inst = vec![60u8, 0, 0, 127, 1, 127, 0, 0];
        inst.extend_from_slice(&(if s.looped.kind == LoopKind::PingPong { 2u16 } else { 1 }).to_be_bytes());
        inst.extend_from_slice(&1u16.to_be_bytes());
        inst.extend_from_slice(&2u16.to_be_bytes());
        inst.extend_from_slice(&[0; 6]);
        chunk_be(&mut body, b"INST", &inst);
    }
    let mut ssnd = vec![0u8; 8];
    for &v in &w.pcm {
        if bits == 8 {
            ssnd.push((v >> 8) as u8);
        } else {
            ssnd.extend_from_slice(&v.to_be_bytes());
        }
    }
    chunk_be(&mut body, b"SSND", &ssnd);
    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&(body.len() as u32 + 4).to_be_bytes());
    out.extend_from_slice(b"AIFF");
    out.extend_from_slice(&body);
    out
}

// ---- IFF 8SVX ----

/// Fibonacci-delta compression's steps.
const FIB: [i8; 16] = [-34, -21, -13, -8, -5, -3, -2, -1, 0, 1, 2, 3, 5, 8, 13, 21];

fn read_8svx(data: &[u8]) -> Result<Sample> {
    let sixteen = &data[8..12] == b"16SV";
    let mut r = Reader::at(data, 12);
    let mut head: Option<(u32, u32, u16, u8, u32)> = None;
    let mut s = Sample::empty();
    let mut body: &[u8] = &[];
    let mut stereo = false;
    while r.left() >= 8 {
        let id = r.tag()?;
        let len = r.u32be()? as usize;
        let start = r.pos;
        match &id {
            b"VHDR" => {
                let one = r.u32be()?;
                let rep = r.u32be()?;
                r.skip(4)?;
                let rate = r.u16be()?;
                r.skip(1)?;
                let comp = r.u8()?;
                let vol = r.u32be()?;
                head = Some((one, rep, rate, comp, vol));
            }
            b"NAME" => s.name = crate::io::text(r.bytes_upto(len)).chars().take(32).collect(),
            b"AUTH" | b"(c) " => add_credit(&mut s.credit, &crate::io::text(r.bytes_upto(len))),
            b"CHAN" => stereo = len >= 4 && r.u32be()? == 6,
            b"BODY" => body = &data[start..(start + len).min(data.len())],
            _ => {}
        }
        r.seek((start + len + (len & 1)).min(data.len()))?;
    }
    let (one, rep, rate, comp, vol) = head.ok_or_else(|| Error::Bad("it has no VHDR chunk".into()))?;
    let raw: Vec<i16> = match comp {
        0 if sixteen => decode(body, Pcm::S16be, body.len() / 2),
        0 => decode(body, Pcm::S8, body.len()),
        1 => {
            // Fibonacci delta: a first byte, then four bits a step.
            let mut out = Vec::with_capacity(body.len().saturating_sub(2) * 2);
            let mut x = body.get(1).copied().unwrap_or(0) as i8;
            for &b in body.iter().skip(2) {
                for nib in [b >> 4, b & 15] {
                    x = x.wrapping_add(FIB[usize::from(nib)]);
                    out.push(i16::from(x) << 8);
                }
            }
            out
        }
        _ => return Err(Error::Unsupported("that 8SVX compression isn't read".into())),
    };
    // Stereo 8SVX is the left channel, then the right.
    let (pcm, ch) = if stereo { (crate::io::interleave(&raw), 2) } else { (raw, 1) };
    s.wave = Arc::new(Wave::new(if sixteen { 16 } else { 8 }, ch, pcm));
    s.c5 = u32::from(rate.max(1));
    s.volume = ((u64::from(vol) * 64 / 65536) as u8).min(64);
    if rep > 0 {
        s.looped = Loop::forward(one, one + rep).clamped(s.frames() as u32);
    }
    Ok(s)
}

fn write_8svx(s: &Sample) -> Vec<u8> {
    let mono = crate::sample_edit::convert(&s.wave, 1, 8);
    let frames = mono.frames() as u32;
    let (one, rep) = if s.looped.on() { (s.looped.start, s.looped.end - s.looped.start) } else { (frames, 0) };
    let mut body = Vec::new();
    let mut vhdr = Vec::new();
    vhdr.extend_from_slice(&one.to_be_bytes());
    vhdr.extend_from_slice(&rep.to_be_bytes());
    vhdr.extend_from_slice(&32u32.to_be_bytes());
    vhdr.extend_from_slice(&(s.c5.min(65535) as u16).to_be_bytes());
    vhdr.push(1);
    vhdr.push(0);
    vhdr.extend_from_slice(&(u32::from(s.volume.min(64)) * 1024).to_be_bytes());
    chunk_be(&mut body, b"VHDR", &vhdr);
    if !s.name.is_empty() {
        chunk_be(&mut body, b"NAME", s.name.as_bytes());
    }
    if !s.credit.is_empty() {
        chunk_be(&mut body, b"AUTH", s.credit.as_bytes());
    }
    let pcm: Vec<u8> = mono.pcm[..(one + rep).min(frames) as usize].iter().map(|&v| (v >> 8) as u8).collect();
    chunk_be(&mut body, b"BODY", &pcm);
    let mut out = b"FORM".to_vec();
    out.extend_from_slice(&(body.len() as u32 + 4).to_be_bytes());
    out.extend_from_slice(b"8SVX");
    out.extend_from_slice(&body);
    out
}

// ---- FLAC ----

/// Bits read highest first.
struct BitReader<'a> {
    b: &'a [u8],
    pos: usize,
    bit: u32,
}

impl BitReader<'_> {
    fn bit(&mut self) -> Option<u32> {
        let byte = *self.b.get(self.pos)?;
        let v = u32::from(byte >> (7 - self.bit)) & 1;
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.pos += 1;
        }
        Some(v)
    }

    fn bits(&mut self, n: u32) -> Option<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = v << 1 | self.bit()?;
        }
        Some(v)
    }

    fn signed(&mut self, n: u32) -> Option<i32> {
        if n == 0 {
            return Some(0);
        }
        let v = self.bits(n)?;
        Some(((v << (32 - n)) as i32) >> (32 - n))
    }

    fn unary(&mut self) -> Option<u32> {
        let mut n = 0;
        while self.bit()? == 0 {
            n += 1;
            if n > 1 << 20 {
                return None;
            }
        }
        Some(n)
    }

    fn align(&mut self) {
        if self.bit != 0 {
            self.bit = 0;
            self.pos += 1;
        }
    }

    /// A UTF-8-style coded number (the frame's number).
    fn utf8(&mut self) -> Option<u64> {
        let first = self.bits(8)? as u8;
        let lead = first.leading_ones();
        let extra = lead.saturating_sub(1);
        let mut v = u64::from(first & (0xFFu8.checked_shr(lead + 1).unwrap_or(0)));
        for _ in 0..extra {
            v = v << 6 | u64::from(self.bits(8)? & 0x3F);
        }
        Some(v)
    }
}

fn read_flac(data: &[u8]) -> Result<Sample> {
    let mut r = Reader::at(data, 4);
    let mut info: Option<(u32, u8, u8, u64)> = None;
    let mut name = String::new();
    let mut credit = String::new();
    loop {
        let head = r.u8()?;
        let len = r.u24be()? as usize;
        let start = r.pos;
        match head & 0x7F {
            0 => {
                let b = r.bytes(18)?;
                let rate = u32::from(b[10]) << 12 | u32::from(b[11]) << 4 | u32::from(b[12]) >> 4;
                let ch = ((b[12] >> 1) & 7) + 1;
                let bits = (((b[12] & 1) << 4) | (b[13] >> 4)) + 1;
                let total = u64::from(b[13] & 15) << 32 | u64::from(u32::from_be_bytes([b[14], b[15], b[16], b[17]]));
                info = Some((rate, ch, bits, total));
            }
            4 => {
                // Vorbis comments: the title and artist.
                let mut c = Reader::at(data, start);
                let vendor = c.u32le()? as usize;
                c.skip(vendor)?;
                let n = c.u32le()?;
                for _ in 0..n.min(1000) {
                    let l = c.u32le()? as usize;
                    let t = String::from_utf8_lossy(c.bytes(l)?).into_owned();
                    if let Some((k, v)) = t.split_once('=') {
                        match k.to_ascii_uppercase().as_str() {
                            "TITLE" => name = v.chars().take(32).collect(),
                            "ARTIST" | "COPYRIGHT" | "LICENSE" => add_credit(&mut credit, v),
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
        r.seek(start + len)?;
        if head & 0x80 != 0 {
            break;
        }
    }
    let (rate, ch, bits, total) = info.ok_or_else(|| Error::Bad("it has no STREAMINFO".into()))?;
    let nch = usize::from(ch);
    let mut out: Vec<i16> = Vec::with_capacity((total as usize).min(1 << 26) * nch.min(2));
    let mut br = BitReader { b: data, pos: r.pos, bit: 0 };
    let shift = i32::from(bits) - 16;
    let mut chans: Vec<Vec<i32>> = vec![Vec::new(); nch.max(2)];
    while br.pos + 2 < data.len() {
        // Find the next frame's sync.
        if br.bits(14) != Some(0x3FFE) {
            br.bit = 0;
            br.pos += 1;
            continue;
        }
        let Some(frame) = flac_frame(&mut br, bits, nch, &mut chans) else { break };
        for i in 0..frame {
            for c in 0..nch.min(2) {
                let v = chans[c][i];
                let v = if shift > 0 { v >> shift } else { v << -shift };
                out.push(v.clamp(-32768, 32767) as i16);
            }
        }
        if total > 0 && out.len() / nch.min(2) >= total as usize {
            break;
        }
    }
    if total > 0 {
        out.truncate(total as usize * nch.min(2));
    }
    Ok(Sample { name, credit, wave: Arc::new(Wave::new(if bits <= 8 { 8 } else { 16 }, ch.min(2), out)), c5: rate.max(1), ..Sample::empty() })
}

/// One FLAC frame (its sync read) into `chans`; returns its frames.
fn flac_frame(br: &mut BitReader, bits: u8, nch: usize, chans: &mut [Vec<i32>]) -> Option<usize> {
    br.bits(2)?;
    let bs_code = br.bits(4)?;
    let _rate_code = br.bits(4)?;
    let assign = br.bits(4)?;
    let size_code = br.bits(3)?;
    br.bits(1)?;
    br.utf8()?;
    let block = match bs_code {
        1 => 192,
        2..=5 => 576 << (bs_code - 2),
        6 => br.bits(8)? as usize + 1,
        7 => br.bits(16)? as usize + 1,
        8..=15 => 256 << (bs_code - 8),
        _ => return None,
    };
    match _rate_code {
        12 => {
            br.bits(8)?;
        }
        13 | 14 => {
            br.bits(16)?;
        }
        _ => {}
    }
    br.bits(8)?; // CRC-8
    let sample_bits = match size_code {
        1 => 8,
        2 => 12,
        4 => 16,
        5 => 20,
        6 => 24,
        7 => 32,
        _ => u32::from(bits),
    };
    let channels = if assign < 8 { assign as usize + 1 } else { 2 };
    if channels != nch.max(1) && assign < 8 {
        return None;
    }
    for (c, buf) in chans.iter_mut().enumerate().take(channels) {
        // The side channel has a bit more.
        let extra = match (assign, c) {
            (8, 1) | (9, 0) | (10, 1) => 1,
            _ => 0,
        };
        buf.clear();
        flac_subframe(br, sample_bits + extra, block, buf)?;
    }
    match assign {
        8 => {
            for i in 0..block {
                chans[1][i] = chans[0][i] - chans[1][i];
            }
        }
        9 => {
            for i in 0..block {
                chans[0][i] += chans[1][i];
            }
        }
        10 => {
            for i in 0..block {
                let (mid, side) = (chans[0][i], chans[1][i]);
                let mid = (mid << 1) | (side & 1);
                chans[0][i] = (mid + side) >> 1;
                chans[1][i] = (mid - side) >> 1;
            }
        }
        _ => {}
    }
    br.align();
    br.bits(16)?; // CRC-16
    Some(block)
}

fn flac_subframe(br: &mut BitReader, bits: u32, block: usize, out: &mut Vec<i32>) -> Option<()> {
    br.bits(1)?;
    let kind = br.bits(6)?;
    let wasted = if br.bit()? == 1 { br.unary()? + 1 } else { 0 };
    let bits = bits.checked_sub(wasted)?;
    match kind {
        0 => {
            let v = br.signed(bits)?;
            out.resize(block, v);
        }
        1 => {
            for _ in 0..block {
                out.push(br.signed(bits)?);
            }
        }
        8..=12 => {
            let order = (kind - 8) as usize;
            for _ in 0..order {
                out.push(br.signed(bits)?);
            }
            residual(br, block, order, out)?;
            for i in order..block {
                let p = match order {
                    0 => 0,
                    1 => out[i - 1],
                    2 => 2 * out[i - 1] - out[i - 2],
                    3 => 3 * out[i - 1] - 3 * out[i - 2] + out[i - 3],
                    _ => 4 * out[i - 1] - 6 * out[i - 2] + 4 * out[i - 3] - out[i - 4],
                };
                out[i] = out[i].wrapping_add(p);
            }
        }
        32..=63 => {
            let order = (kind - 31) as usize;
            for _ in 0..order {
                out.push(br.signed(bits)?);
            }
            let precision = br.bits(4)? + 1;
            let qshift = br.signed(5)?;
            let coefs: Vec<i64> = (0..order).map(|_| br.signed(precision).map(i64::from)).collect::<Option<_>>()?;
            residual(br, block, order, out)?;
            for i in order..block {
                let mut sum = 0i64;
                for (j, c) in coefs.iter().enumerate() {
                    sum += c * i64::from(out[i - 1 - j]);
                }
                out[i] = out[i].wrapping_add((sum >> qshift.max(0)) as i32);
            }
        }
        _ => return None,
    }
    if wasted > 0 {
        for v in out.iter_mut() {
            *v <<= wasted;
        }
    }
    Some(())
}

fn residual(br: &mut BitReader, block: usize, order: usize, out: &mut Vec<i32>) -> Option<()> {
    let method = br.bits(2)?;
    let pbits = if method == 1 { 5 } else { 4 };
    let escape = if method == 1 { 31 } else { 15 };
    let porder = br.bits(4)?;
    let parts = 1usize << porder;
    for p in 0..parts {
        let n = if porder == 0 { block - order } else if p == 0 { (block >> porder) - order } else { block >> porder };
        let k = br.bits(pbits)?;
        if k == escape {
            let raw = br.bits(5)?;
            for _ in 0..n {
                out.push(br.signed(raw)?);
            }
        } else {
            for _ in 0..n {
                let q = br.unary()?;
                let v = (q << k) | br.bits(k)?;
                out.push(((v >> 1) as i32) ^ -((v & 1) as i32));
            }
        }
    }
    Some(())
}

/// Bits written highest first.
struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, v: u64, bits: u32) {
        for i in (0..bits).rev() {
            self.acc = self.acc << 1 | (v >> i) & 1;
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.n = 0;
            }
        }
    }

    fn align(&mut self) {
        while self.n != 0 {
            self.put(0, 1);
        }
    }
}

fn crc8(b: &[u8]) -> u8 {
    let mut c = 0u8;
    for &x in b {
        c ^= x;
        for _ in 0..8 {
            c = if c & 0x80 != 0 { c << 1 ^ 0x07 } else { c << 1 };
        }
    }
    c
}

fn crc16(b: &[u8]) -> u16 {
    let mut c = 0u16;
    for &x in b {
        c ^= u16::from(x) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { c << 1 ^ 0x8005 } else { c << 1 };
        }
    }
    c
}

/// A waveform as FLAC: blocks of 4096 frames, each channel with the
/// fixed predictor (order 0 to 4) that leaves least, its residual
/// Rice-coded with the best parameter.
pub fn write_flac(w: &Wave, rate: u32) -> Vec<u8> {
    const BLOCK: usize = 4096;
    let ch = usize::from(w.channels.max(1));
    let bits: u32 = if w.bits == 8 { 8 } else { 16 };
    let frames = w.frames();
    let shift = 16 - bits;
    let mut out = b"fLaC".to_vec();
    // STREAMINFO, the last block of metadata.
    out.push(0x80);
    out.extend_from_slice(&[0, 0, 34]);
    let mut si = BitWriter { out: Vec::new(), acc: 0, n: 0 };
    si.put(BLOCK as u64, 16);
    si.put(BLOCK as u64, 16);
    si.put(0, 24);
    si.put(0, 24);
    si.put(u64::from(rate.clamp(1, 655_350)), 20);
    si.put((ch - 1) as u64, 3);
    si.put(u64::from(bits - 1), 5);
    si.put(frames as u64, 36);
    si.put(0, 64);
    si.put(0, 64);
    out.extend_from_slice(&si.out);
    for (k, start) in (0..frames).step_by(BLOCK).enumerate() {
        let n = BLOCK.min(frames - start);
        let mut f = BitWriter { out: Vec::new(), acc: 0, n: 0 };
        f.put(0x3FFE, 14);
        f.put(0, 2);
        f.put(7, 4); // block size: 16 bits after
        f.put(0, 4); // rate: from STREAMINFO
        f.put((ch - 1) as u64, 4);
        f.put(if bits == 8 { 1 } else { 4 }, 3);
        f.put(0, 1);
        // The frame's number, UTF-8 coded.
        let num = k as u32;
        let extra = match num {
            0..=0x7F => 0,
            0x80..=0x7FF => 1,
            0x800..=0xFFFF => 2,
            0x1_0000..=0x1F_FFFF => 3,
            0x20_0000..=0x3FF_FFFF => 4,
            _ => 5,
        };
        if extra == 0 {
            f.put(u64::from(num), 8);
        } else {
            let lead = (0xFF00u16 >> (extra + 1)) as u8;
            f.put(u64::from(lead | (num >> (6 * extra)) as u8), 8);
            for i in (0..extra).rev() {
                f.put(u64::from(0x80 | (num >> (6 * i)) & 0x3F), 8);
            }
        }
        f.put((n - 1) as u64, 16);
        let c8 = crc8(&f.out);
        f.put(u64::from(c8), 8);
        for c in 0..ch {
            let x: Vec<i32> = (0..n).map(|i| i32::from(w.pcm[(start + i) * ch + c]) >> shift).collect();
            flac_encode_subframe(&mut f, &x, bits);
        }
        f.align();
        let c16 = crc16(&f.out);
        f.put(u64::from(c16), 16);
        out.extend_from_slice(&f.out);
    }
    out
}

fn flac_encode_subframe(f: &mut BitWriter, x: &[i32], bits: u32) {
    let n = x.len();
    // Silence, constant.
    if x.iter().all(|&v| v == x[0]) {
        f.put(0, 1);
        f.put(0, 6);
        f.put(0, 1);
        f.put((x[0] as u32 & ((1u64 << bits) - 1) as u32) as u64, bits);
        return;
    }
    let residuals = |order: usize| -> Vec<i32> {
        (order..n)
            .map(|i| {
                x[i] - match order {
                    0 => 0,
                    1 => x[i - 1],
                    2 => 2 * x[i - 1] - x[i - 2],
                    3 => 3 * x[i - 1] - 3 * x[i - 2] + x[i - 3],
                    _ => 4 * x[i - 1] - 6 * x[i - 2] + 4 * x[i - 3] - x[i - 4],
                }
            })
            .collect()
    };
    let max_order = 4.min(n.saturating_sub(1));
    let (order, res) = (0..=max_order).map(|o| (o, residuals(o))).min_by_key(|(_, r)| r.iter().map(|v| i64::from(v.unsigned_abs())).sum::<i64>()).unwrap_or((0, residuals(0)));
    f.put(0, 1);
    f.put(8 + order as u64, 6);
    f.put(0, 1);
    for &v in &x[..order] {
        f.put(u64::from(v as u32) & ((1u64 << bits) - 1), bits);
    }
    // One partition, the Rice parameter that codes it shortest.
    let zig: Vec<u32> = res.iter().map(|&v| ((v << 1) ^ (v >> 31)) as u32).collect();
    let cost = |k: u32| -> u64 { zig.iter().map(|&u| u64::from(u >> k) + 1 + u64::from(k)).sum() };
    let k = (0..15).min_by_key(|&k| cost(k)).unwrap_or(0);
    f.put(0, 2);
    f.put(0, 4);
    f.put(u64::from(k), 4);
    for &u in &zig {
        let q = u >> k;
        for _ in 0..q {
            f.put(0, 1);
        }
        f.put(1, 1);
        f.put(u64::from(u & ((1 << k) - 1)), k);
    }
}

// ---- Impulse Tracker's ITS ----

fn write_its(s: &Sample) -> Vec<u8> {
    let w = &s.wave;
    let ch = usize::from(w.channels.max(1));
    let b16 = w.bits != 8;
    let mut h = Vec::with_capacity(80);
    h.extend_from_slice(b"IMPS");
    let mut fname = [0u8; 12];
    for (d, c) in fname.iter_mut().zip(s.filename.bytes().filter(u8::is_ascii)) {
        *d = c;
    }
    h.extend_from_slice(&fname);
    h.push(0);
    h.push(s.global_volume.min(64));
    let mut flags = 1u8;
    if b16 {
        flags |= 2;
    }
    if ch == 2 {
        flags |= 4;
    }
    if s.looped.on() {
        flags |= 0x10;
        if s.looped.kind == LoopKind::PingPong {
            flags |= 0x40;
        }
    }
    if s.sustain.on() {
        flags |= 0x20;
        if s.sustain.kind == LoopKind::PingPong {
            flags |= 0x80;
        }
    }
    h.push(flags);
    h.push(s.volume.min(64));
    let mut name = [0u8; 26];
    for (d, c) in name.iter_mut().zip(s.name.bytes()) {
        *d = c;
    }
    h.extend_from_slice(&name);
    h.push(1); // signed
    h.push(s.pan.map_or(32, |p| (u16::from(p) * 64 / 255) as u8 | 0x80));
    for v in [w.frames() as u32, s.looped.start, s.looped.end, s.c5, s.sustain.start, s.sustain.end, 80] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    let v = s.vibrato;
    h.extend_from_slice(&[v.speed, v.depth, v.sweep, v.kind.min(3)]);
    // Two channels one after the other.
    for c in 0..ch {
        for f in 0..w.frames() {
            let p = w.pcm[f * ch + c];
            if b16 {
                h.extend_from_slice(&p.to_le_bytes());
            } else {
                h.push((p >> 8) as u8);
            }
        }
    }
    h
}

// ---- FastTracker 2's XI ----

/// A rate for C-5 as XM's relative note and finetune.
fn relative_of(c5: u32) -> (i8, i8) {
    let semis = 12.0 * (f64::from(c5.max(1)) / 8363.0).log2();
    let rel = semis.floor();
    let ft = ((semis - rel) * 128.0).round();
    let (rel, ft) = if ft >= 128.0 { (rel + 1.0, ft - 128.0) } else { (rel, ft) };
    (rel.clamp(-96.0, 95.0) as i8, ft.clamp(-128.0, 127.0) as i8)
}

fn xm_env_points(e: &Envelope, pan: bool) -> [u8; 48] {
    let mut out = [0u8; 48];
    for (i, p) in e.points.iter().take(12).enumerate() {
        let v = if pan { (i16::from(p.value) + 32).clamp(0, 64) as u16 } else { p.value.clamp(0, 64) as u16 };
        out[i * 4..i * 4 + 2].copy_from_slice(&p.tick.to_le_bytes());
        out[i * 4 + 2..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

fn read_xi(data: &[u8]) -> Result<Loaded> {
    let mut r = Reader::at(data, 21);
    let name = r.text(22)?;
    r.skip(1 + 20 + 2)?;
    let keymap = r.bytes(96)?.to_vec();
    let mut vol_pts = [(0u16, 0u16); 12];
    let mut pan_pts = [(0u16, 0u16); 12];
    for p in &mut vol_pts {
        *p = (r.u16le()?, r.u16le()?);
    }
    for p in &mut pan_pts {
        *p = (r.u16le()?, r.u16le()?);
    }
    let (nvol, npan) = (r.u8()?, r.u8()?);
    let (vsus, vls, vle) = (r.u8()?, r.u8()?, r.u8()?);
    let (psus, pls, ple) = (r.u8()?, r.u8()?, r.u8()?);
    let (vtype, ptype) = (r.u8()?, r.u8()?);
    let vib = AutoVibrato { kind: r.u8()?, sweep: r.u8()?, depth: r.u8()?, speed: r.u8()? };
    let fadeout = r.u16le()?;
    r.skip(22)?;
    let n = usize::from(r.u16le()?).min(128);
    let mut ins = Instrument {
        name: name.clone(),
        volume_env: crate::formats::xm::envelope(&vol_pts, nvol.min(12), vtype, vsus, vls, vle, false),
        pan_env: crate::formats::xm::envelope(&pan_pts, npan.min(12), ptype, psus, pls, ple, true),
        fadeout: u32::from(fadeout) * 2,
        global_volume: 128,
        ..Default::default()
    };
    let mut heads = Vec::with_capacity(n);
    for _ in 0..n {
        let len = r.u32le()? as usize;
        let ls = r.u32le()?;
        let ll = r.u32le()?;
        let volume = r.u8()?;
        let ft = r.i8()?;
        let ty = r.u8()?;
        let pan = r.u8()?;
        let rel = r.i8()?;
        r.skip(1)?;
        let sname = r.text(22)?;
        heads.push((len, ls, ll, volume, ft, ty, pan, rel, sname));
    }
    let mut samples = Vec::with_capacity(n);
    for (len, ls, ll, volume, ft, ty, pan, rel, sname) in heads {
        let b16 = ty & 0x10 != 0;
        let fmt = if b16 { Pcm::D16le } else { Pcm::D8 };
        let bytes = r.bytes_upto(len);
        let pcm = decode(bytes, fmt, len / fmt.bytes());
        let div = if b16 { 2 } else { 1 };
        samples.push(Sample {
            name: sname,
            wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
            c5: relative_c5(rel, ft),
            volume: volume.min(64),
            pan: Some(pan),
            looped: Loop { kind: match ty & 3 {
                1 => LoopKind::Forward,
                2 | 3 => LoopKind::PingPong,
                _ => LoopKind::Off,
            }, start: ls / div, end: (ls + ll) / div },
            vibrato: vib,
            ..Sample::empty()
        });
    }
    ins.keymap = (0..120)
        .map(|k| {
            let xm: i32 = k - 12;
            let s = if (0..96).contains(&xm) { usize::from(keymap[xm as usize]) } else { 0 };
            (k as u8, if (0..96).contains(&xm) && s < samples.len() { (s + 1) as u16 } else { 0 })
        })
        .collect();
    for s in &mut samples {
        let frames = s.frames() as u32;
        s.looped = s.looped.clamped(frames);
    }
    Ok(Loaded { samples, instrument: Some(ins) })
}

/// An instrument and its samples (1-based in its keymap) as an XI file.
pub fn write_xi(ins: &Instrument, samples: &[Sample]) -> Vec<u8> {
    // The samples the keymap uses, in order, as the file's own.
    let mut used: Vec<u16> = Vec::new();
    for &(_, s) in &ins.keymap {
        if s > 0 && usize::from(s) <= samples.len() && !used.contains(&s) && used.len() < 16 {
            used.push(s);
        }
    }
    let mut out = b"Extended Instrument: ".to_vec();
    let mut name = [b' '; 22];
    for (d, c) in name.iter_mut().zip(ins.name.bytes()) {
        *d = c;
    }
    out.extend_from_slice(&name);
    out.push(0x1A);
    out.extend_from_slice(b"Neumetik            ");
    out.extend_from_slice(&0x0102u16.to_le_bytes());
    for k in 12..108 {
        let s = ins.keymap.get(k).map_or(0, |m| m.1);
        out.push(used.iter().position(|&u| u == s).unwrap_or(0) as u8);
    }
    out.extend_from_slice(&xm_env_points(&ins.volume_env, false));
    out.extend_from_slice(&xm_env_points(&ins.pan_env, true));
    out.push(ins.volume_env.points.len().min(12) as u8);
    out.push(ins.pan_env.points.len().min(12) as u8);
    let sus = |e: &Envelope| e.sustain.map_or(0, |s| s.0);
    let lp = |e: &Envelope| e.looped.unwrap_or((0, 0));
    let ty = |e: &Envelope| u8::from(e.enabled) | u8::from(e.sustain.is_some()) << 1 | u8::from(e.looped.is_some()) << 2;
    out.extend_from_slice(&[sus(&ins.volume_env), lp(&ins.volume_env).0, lp(&ins.volume_env).1]);
    out.extend_from_slice(&[sus(&ins.pan_env), lp(&ins.pan_env).0, lp(&ins.pan_env).1]);
    out.push(ty(&ins.volume_env));
    out.push(ty(&ins.pan_env));
    let vib = used.first().and_then(|&s| samples.get(usize::from(s) - 1)).map(|s| s.vibrato).unwrap_or_default();
    out.extend_from_slice(&[vib.kind.min(3), vib.sweep, vib.depth, vib.speed]);
    out.extend_from_slice(&((ins.fadeout / 2).min(0xFFF) as u16).to_le_bytes());
    out.extend_from_slice(&[0; 22]);
    out.extend_from_slice(&(used.len() as u16).to_le_bytes());
    let waves: Vec<Wave> = used.iter().map(|&s| crate::sample_edit::convert(&samples[usize::from(s) - 1].wave, 1, samples[usize::from(s) - 1].wave.bits)).collect();
    for (k, &u) in used.iter().enumerate() {
        let s = &samples[usize::from(u) - 1];
        let w = &waves[k];
        let b = if w.bits == 8 { 1 } else { 2 };
        let lp = s.looped.clamped(w.frames() as u32);
        out.extend_from_slice(&((w.frames() * b) as u32).to_le_bytes());
        out.extend_from_slice(&(lp.start * b as u32).to_le_bytes());
        out.extend_from_slice(&((lp.end - lp.start) * b as u32).to_le_bytes());
        out.push(s.volume.min(64));
        let (rel, ft) = relative_of(s.c5);
        out.push(ft as u8);
        let kind = match lp.kind {
            LoopKind::Off => 0,
            LoopKind::Forward => 1,
            LoopKind::PingPong => 2,
        };
        out.push(kind | if b == 2 { 0x10 } else { 0 });
        out.push(s.pan.unwrap_or(128));
        out.push(rel as u8);
        out.push(0);
        let mut sn = [b' '; 22];
        for (d, c) in sn.iter_mut().zip(s.name.bytes()) {
            *d = c;
        }
        out.extend_from_slice(&sn);
    }
    for w in &waves {
        // Deltas.
        let mut last = 0i16;
        for &v in &w.pcm {
            if w.bits == 8 {
                let x = (v >> 8) as i8;
                out.push((x as u8).wrapping_sub(last as u8));
                last = i16::from(x);
            } else {
                out.extend_from_slice(&v.wrapping_sub(last).to_le_bytes());
                last = v;
            }
        }
    }
    out
}

// ---- Gravis UltraSound PAT ----

fn read_pat(data: &[u8]) -> Result<Loaded> {
    let mut r = Reader::at(data, 22);
    let description = r.text(60)?;
    let _instruments = r.u8()?;
    r.skip(1 + 1)?;
    let waves = usize::from(r.u16le()?);
    r.seek(129)?;
    // The instrument, then its first layer.
    r.skip(2)?;
    let iname = r.text(16)?;
    r.skip(4 + 1 + 40)?;
    r.skip(1 + 1 + 4)?;
    let n = usize::from(r.u8()?).max(waves.min(1)).min(64);
    r.skip(40)?;
    let mut samples = Vec::new();
    let mut ranges = Vec::new();
    for _ in 0..n {
        let wname = r.text(7)?;
        let _fractions = r.u8()?;
        let size = r.u32le()? as usize;
        let ls = r.u32le()?;
        let le = r.u32le()?;
        let rate = u32::from(r.u16le()?);
        let low = r.u32le()?;
        let high = r.u32le()?;
        let root = r.u32le()?;
        r.skip(2 + 1 + 6 + 6 + 3 + 3)?;
        let modes = r.u8()?;
        r.skip(2 + 2 + 36)?;
        let b16 = modes & 1 != 0;
        let unsigned = modes & 2 != 0;
        let fmt = match (b16, unsigned) {
            (true, true) => Pcm::U16le,
            (true, false) => Pcm::S16le,
            (false, true) => Pcm::U8,
            (false, false) => Pcm::S8,
        };
        let bytes = r.bytes_upto(size);
        let pcm = decode(bytes, fmt, size / fmt.bytes());
        let div = if b16 { 2 } else { 1 };
        // The rate that plays middle C, from the pitch it plays at its own rate.
        let c5 = if root > 0 { (f64::from(rate) * 261_625.6 / f64::from(root)).round() as u32 } else { rate };
        let mut s = Sample {
            name: if wname.trim().is_empty() { iname.clone() } else { wname },
            wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, pcm)),
            c5: c5.max(1),
            ..Sample::empty()
        };
        if modes & 4 != 0 {
            s.looped = Loop { kind: if modes & 8 != 0 { LoopKind::PingPong } else { LoopKind::Forward }, start: ls / div, end: le / div }.clamped(s.frames() as u32);
        }
        if modes & 0x10 != 0 {
            crate::sample_edit::reverse(&mut s, 0, usize::MAX);
        }
        samples.push(s);
        ranges.push((low, high));
    }
    if samples.is_empty() {
        return bad("it has no waves");
    }
    // The waves' frequency ranges as a keymap.
    let hz = |n: usize| 261.6256 * 2f64.powf((n as f64 - 60.0) / 12.0) * 1000.0;
    let keymap = (0..120)
        .map(|k| {
            let f = hz(k);
            let s = ranges.iter().position(|&(lo, hi)| f >= f64::from(lo) && f <= f64::from(hi)).unwrap_or(if f < f64::from(ranges[0].0) { 0 } else { ranges.len() - 1 });
            (k as u8, (s + 1) as u16)
        })
        .collect();
    let name = if iname.trim().is_empty() { description } else { iname };
    let instrument = (samples.len() > 1).then(|| Instrument { name: name.clone(), keymap, global_volume: 128, ..Default::default() });
    if samples.len() == 1 && samples[0].name.trim().is_empty() {
        samples[0].name = name;
    }
    Ok(Loaded { samples, instrument })
}

fn write_pat(s: &Sample) -> Vec<u8> {
    let w = crate::sample_edit::convert(&s.wave, 1, s.wave.bits);
    let b16 = w.bits != 8;
    let bytes = w.frames() * if b16 { 2 } else { 1 };
    let mut out = Vec::new();
    out.extend_from_slice(b"GF1PATCH110\0ID#000002\0");
    let mut desc = [0u8; 60];
    for (d, c) in desc.iter_mut().zip(s.name.bytes()) {
        *d = c;
    }
    out.extend_from_slice(&desc);
    out.extend_from_slice(&[1, 14, 0]);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&127u16.to_le_bytes());
    out.extend_from_slice(&(bytes as u32 + 96).to_le_bytes());
    out.extend_from_slice(&[0; 36]);
    // The instrument.
    out.extend_from_slice(&1u16.to_le_bytes());
    let mut iname = [0u8; 16];
    for (d, c) in iname.iter_mut().zip(s.name.bytes()) {
        *d = c;
    }
    out.extend_from_slice(&iname);
    out.extend_from_slice(&(bytes as u32 + 96 + 47).to_le_bytes());
    out.push(1);
    out.extend_from_slice(&[0; 40]);
    // The layer.
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&(bytes as u32 + 96).to_le_bytes());
    out.push(1);
    out.extend_from_slice(&[0; 40]);
    // The wave: at its own rate it plays middle C.
    let mut wname = [0u8; 7];
    for (d, c) in wname.iter_mut().zip(s.name.bytes()) {
        *d = c;
    }
    out.extend_from_slice(&wname);
    out.push(0);
    out.extend_from_slice(&(bytes as u32).to_le_bytes());
    let k = if b16 { 2 } else { 1 };
    out.extend_from_slice(&(s.looped.start * k).to_le_bytes());
    out.extend_from_slice(&(s.looped.end * k).to_le_bytes());
    out.extend_from_slice(&(s.c5.min(65535) as u16).to_le_bytes());
    out.extend_from_slice(&8176u32.to_le_bytes());
    out.extend_from_slice(&12_543_854u32.to_le_bytes());
    out.extend_from_slice(&261_626u32.to_le_bytes());
    out.extend_from_slice(&0i16.to_le_bytes());
    out.push(7);
    out.extend_from_slice(&[63, 63, 63, 63, 63, 63]);
    out.extend_from_slice(&[245, 245, 245, 245, 245, 245]);
    out.extend_from_slice(&[0; 6]);
    let mut modes = 0u8;
    if b16 {
        modes |= 1;
    }
    if s.looped.on() {
        modes |= 4;
        if s.looped.kind == LoopKind::PingPong {
            modes |= 8;
        }
    }
    out.push(modes);
    out.extend_from_slice(&60i16.to_le_bytes());
    out.extend_from_slice(&1024u16.to_le_bytes());
    out.extend_from_slice(&[0; 36]);
    for &v in &w.pcm {
        if b16 {
            out.extend_from_slice(&v.to_le_bytes());
        } else {
            out.push((v >> 8) as u8);
        }
    }
    out
}

/// A sample file's artist, copyright and license lines, each kept
/// (`; ` between): who made it and on what terms (`repo::rights`).
fn add_credit(credit: &mut String, t: &str) {
    let t = t.trim();
    if t.is_empty() || credit.split("; ").any(|c| c == t) {
        return;
    }
    if !credit.is_empty() {
        credit.push_str("; ");
    }
    credit.push_str(t);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(bits: u8, channels: u8) -> Sample {
        let n = 5000;
        let pcm: Vec<i16> = (0..n * usize::from(channels)).map(|i| (((i / usize::from(channels)) as f32 * 0.05).sin() * 20000.0) as i16 + if i % 2 == 1 { 0 } else { 0 }).collect();
        let wave = crate::sample_edit::convert(&Wave::new(16, channels, pcm), channels, bits);
        Sample { name: "tone".into(), wave: Arc::new(wave), c5: 22050, looped: Loop::forward(1000, 4000), ..Sample::empty() }
    }

    #[test]
    fn every_kind_round_trips() {
        for (kind, _) in KINDS {
            for (bits, ch) in [(16, 1), (8, 1), (16, 2)] {
                let s = tone(bits, ch);
                let (bytes, lost) = write(&s, kind);
                let back = read(&bytes, &format!("x.{kind}")).unwrap_or_else(|e| panic!("{kind} {bits} {ch}: {e}"));
                let b = &back.samples[0];
                let mono_only = matches!(*kind, "8svx" | "xi" | "pat");
                let eight = *kind == "8svx";
                if !mono_only {
                    assert_eq!(b.wave.channels, ch, "{kind}");
                }
                if !eight && !mono_only {
                    assert_eq!(b.wave.pcm, s.wave.pcm, "{kind} {bits} {ch}");
                }
                if bits == 8 && ch == 1 {
                    // 8SVX keeps what plays: up to the loop's end.
                    let n = if eight { 4000 } else { s.frames() };
                    assert_eq!(b.wave.pcm, s.wave.pcm[..n], "{kind} 8-bit mono");
                }
                if *kind != "flac" {
                    assert_eq!((b.looped.start, b.looped.end), (1000, 4000), "{kind}");
                }
                assert!((i64::from(b.c5) - 22050).abs() <= 12, "{kind}: {}", b.c5);
                if *kind == "flac" {
                    assert!(!lost.is_empty());
                }
            }
        }
    }

    #[test]
    fn extended_numbers_round_trip() {
        for v in [8363.0, 22050.0, 44100.0, 96000.0] {
            assert_eq!(read_extended(&write_extended(v)), v);
        }
    }
}
