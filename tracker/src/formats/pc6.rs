//! Formats whose samples are packed lossily, now Neumetik decodes Ogg
//! Vorbis (`crate::vorbis`): Un4seen's MO3 (an IT, XM, S3M, MOD or MTM
//! made small) and Skale Tracker (.skm).

use std::sync::Arc;

use super::common::{mod_effect, relative_c5, s3m_effect};
use crate::io::{bad, decode, Pcm, Reader, Result};
use crate::model::*;

// ---- MO3 ----
//
// "MO3", a version (0 to 5), the unpacked music's length (and from
// version 5 its packed length), then the music packed by MO3's LZ
// scheme (control bits taken most significant first, a sentinel bit
// marking each byte's end): the title and message, a header (channels,
// orders, patterns, tracks, instruments, samples, speed, tempo, flags
// saying which tracker it came from, volumes, the channels' volume and
// panning, MIDI macros), the orders, each pattern's tracks and length,
// the tracks (rows of commands: a byte with how many commands and how
// many rows they last, then pairs of command and value), the
// instruments (IT's and XM's in one layout), the samples' headers, then
// the samples: raw, packed by MO3's own delta schemes, Ogg Vorbis (its
// headers shared with another sample's, from version 5) or MP3 (not
// read: no MP3 decoder yet). Described by OpenMPT's Load_mo3.cpp (BSD),
// itself from the UNMO3 project.

pub fn probe_mo3(d: &[u8]) -> bool {
    d.len() >= 12 && &d[..3] == b"MO3" && d[3] <= 5 && {
        let size = u32::from_le_bytes([d[4], d[5], d[6], d[7]]);
        size > 422 && size < 0x2000_0000
    }
}

/// MO3's control bits: a byte at a time, most significant first, with a
/// marker bit to say when the byte's used up.
struct Ctrl {
    data: u16,
}

impl Ctrl {
    fn bit(&mut self, r: &mut Reader) -> Option<u32> {
        self.data <<= 1;
        let mut carry = self.data > 0xFF;
        self.data &= 0xFF;
        if self.data == 0 {
            let b = r.u8().ok()?;
            self.data = (u16::from(b) << 1) + 1;
            carry = self.data > 0xFF;
            self.data &= 0xFF;
        }
        Some(carry as u32)
    }

    /// A length: a 1, then the first bit of each pair, until a pair's
    /// second bit is 0.
    fn length(&mut self, r: &mut Reader, mut len: i32) -> Option<i32> {
        len += 1;
        loop {
            len = (len << 1) + self.bit(r)? as i32;
            if self.bit(r)? == 0 {
                return Some(len);
            }
        }
    }
}

/// The music unpacked to `size` bytes, and where in `d` the packing ended.
fn mo3_unpack(d: &[u8], at: usize, size: usize) -> (Vec<u8>, usize) {
    let mut r = Reader::at(d, at);
    let mut out = Vec::with_capacity(size.min(64 << 20));
    let Ok(first) = r.u8() else { return (out, r.pos) };
    out.push(first);
    let mut c = Ctrl { data: 0 };
    let mut offset: i32 = 0;
    while out.len() < size {
        let Some(bit) = c.bit(&mut r) else { break };
        if bit == 0 {
            let Ok(b) = r.u8() else { break };
            out.push(b);
            continue;
        }
        let Some(mut len) = c.length(&mut r, 0) else { break };
        let mut adjust = 0;
        len -= 3;
        if len < 0 {
            len += 1;
        } else {
            let Ok(b) = r.u8() else { break };
            offset = !((len << 8) | i32::from(b));
            len = 0;
            if offset < -1280 {
                adjust += 1;
            }
            adjust += 1;
            if offset < -32000 {
                adjust += 1;
            }
        }
        if offset >= 0 || (out.len() as i64) < -(offset as i64) {
            break;
        }
        let (Some(a), Some(b)) = (c.bit(&mut r), c.bit(&mut r)) else { break };
        len = (len << 2) | (a << 1 | b) as i32;
        if len == 0 {
            let Some(l) = c.length(&mut r, 0) else { break };
            len = l + 2;
        }
        len += adjust;
        if len <= 0 || out.len() + len as usize > size {
            break;
        }
        let from = out.len() - (-(offset as i64)) as usize;
        for i in 0..len as usize {
            let v = out[from + i];
            out.push(v);
        }
    }
    (out, r.pos)
}

/// MO3's delta-packed samples: each delta's length in bits follows the
/// last's; `predict` adds MO3's prediction from the two points before.
fn mo3_delta(d: &[u8], frames: usize, channels: usize, sixteen: bool, predict: bool) -> Vec<i16> {
    let mut out = vec![0i16; frames * channels];
    let mut r = Reader::new(d);
    let mut c = Ctrl { data: 0 };
    let (shift, mask): (u32, u32) = if sixteen { (15, 0xFFFF) } else { (7, 0xFF) };
    let mut dh: u32 = if sixteen { 8 } else { 4 };
    let mut previous: i32 = 0;
    let mut next: i32 = 0;
    let sign = |v: u32| -> i32 {
        if sixteen {
            v as u16 as i16 as i32
        } else {
            v as u8 as i8 as i32
        }
    };
    'all: for ch in 0..channels {
        for f in 0..frames {
            let mut val: u32 = 0;
            // The delta's first part: bit pairs (triples for wide 16-bit
            // deltas) until a 0.
            loop {
                let Some(b) = c.bit(&mut r) else { break 'all };
                val = ((val << 1) + b) & mask;
                if sixteen && dh < 5 {
                    let Some(b) = c.bit(&mut r) else { break 'all };
                    val = ((val << 1) + b) & mask;
                }
                let Some(more) = c.bit(&mut r) else { break 'all };
                if more == 0 {
                    break;
                }
            }
            for _ in 0..dh {
                let Some(b) = c.bit(&mut r) else { break 'all };
                val = ((val << 1) + b) & mask;
            }
            let mut cl = 1;
            if val >= 4 {
                cl = shift;
                while (1 << cl) & val == 0 && cl > 1 {
                    cl -= 1;
                }
            }
            dh = (dh + cl) >> 1;
            let positive = val & 1 != 0;
            val >>= 1;
            if !positive {
                val = !val & mask;
            }
            let point = if predict {
                let delta = sign(val);
                let v = (val.wrapping_add(next as u32)) & mask;
                let s = sign(v);
                let (lo, hi) = if sixteen { (-32768, 32767) } else { (-128, 127) };
                next = (s * 2 + (delta >> 1) - previous).clamp(lo, hi);
                previous = s;
                s
            } else {
                let v = (val.wrapping_add(previous as u32)) & mask;
                previous = sign(v);
                previous
            };
            out[f * channels + ch] = if sixteen { point as i16 } else { (point as i16) << 8 };
        }
    }
    out
}

/// MO3's commands in the IT and S3M command set (`s3m_effect`'s letters).
fn mo3_s3m_letter(cmd: u8) -> Option<u8> {
    Some(match cmd {
        0x03 => 10,
        0x06 => 7,
        0x07 => 8,
        0x0A => 18,
        0x0B => 24,
        0x0C => 15,
        0x0E => 2,
        0x10 => 3,
        0x16 => 22,
        0x21 => 1,
        0x22 => 4,
        0x23 => 5,
        0x24 => 6,
        0x25 => 9,
        0x26 => 17,
        0x27 => 21,
        0x28 => 13,
        0x29 => 14,
        0x2A => 16,
        0x2B => 19,
        0x2C => 20,
        0x2D => 23,
        0x2E => 25,
        0x2F => 26,
        0x36 => 27,
        _ => return None,
    })
}

/// MO3's commands in MOD's and FastTracker 2's (`mod_effect`'s numbers).
fn mo3_mod_number(cmd: u8) -> Option<u8> {
    Some(match cmd {
        0x03 => 0x0,
        0x04 => 0x1,
        0x05 => 0x2,
        0x06 => 0x3,
        0x07 => 0x4,
        0x08 => 0x5,
        0x09 => 0x6,
        0x0A => 0x7,
        0x0B => 0x8,
        0x0C => 0x9,
        0x0D => 0xA,
        0x0E => 0xB,
        0x0F => 0xC,
        0x10 => 0xD,
        0x11 => 0xE,
        0x12 => 0xF,
        0x13 => 0x1D,
        0x16 => 0x10,
        0x17 => 0x11,
        0x18 => 0x14,
        0x19 => 0x15,
        0x1A => 0x19,
        0x1C => 0x1B,
        0x1D | 0x1E => 0x21,
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Mo3Type {
    Mod,
    Mtm,
    S3m,
    Xm,
    It,
}

struct Mo3Sample {
    sample: Sample,
    flags: u16,
    length: usize,
    packed: i32,
    delay: u16,
    shared: i16,
    chunk: (usize, usize),
}

fn mo3_envelope(r: &mut Reader, shift: u32, centred: bool, xm: bool) -> Result<Envelope> {
    let flags = r.u8()?;
    let n = r.u8()?.min(25);
    let (ss, se, ls, le) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
    let mut points = Vec::with_capacity(usize::from(n));
    for i in 0..25 {
        let t = r.i16le()?;
        let v = r.i16le()?;
        if i < n {
            let mut tick = t.max(0) as u16;
            if let Some(last) = points.last().map(|p: &EnvPoint| p.tick) {
                if tick < last {
                    tick = last + 1;
                }
            }
            let v = (i32::from(v) >> shift).clamp(0, 64);
            points.push(EnvPoint { tick, value: if centred { (v - 32) as i8 } else { v as i8 } });
        }
    }
    let se = if xm { ss } else { se };
    Ok(Envelope {
        enabled: flags & 1 != 0 && n > 0,
        sustain: (flags & 2 != 0 && ss <= se && se < n).then_some((ss, se)),
        looped: (flags & 4 != 0 && ls <= le && le < n).then_some((ls, le)),
        filter: flags & 0x10 != 0,
        carry: flags & 0x20 != 0,
        points,
        release: None,
    })
}

pub fn load_mo3(d: &[u8]) -> Result<Module> {
    let version = d[3];
    let size = u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize;
    let (start, packed_end) = if version >= 5 {
        let packed = u32::from_le_bytes([d[8], d[9], d[10], d[11]]) as usize;
        (12, Some(12usize.saturating_add(packed)))
    } else {
        (8, None)
    };
    let (music, used) = mo3_unpack(d, start, size);
    let mut samples_at = packed_end.unwrap_or(used);
    if music.len() < size.min(500) {
        return bad("its packed music is damaged");
    }
    let mut r = Reader::new(&music);
    let cstr = |r: &mut Reader| -> Result<String> {
        let rest = &r.data[r.pos..];
        let n = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        let s = crate::io::text(&rest[..n]);
        r.skip((n + 1).min(rest.len()))?;
        Ok(s)
    };
    let title = cstr(&mut r)?;
    let message = cstr(&mut r)?;
    let nch = usize::from(r.u8()?);
    let norders = usize::from(r.u16le()?);
    let restart = r.u16le()?;
    let npat = usize::from(r.u16le()?);
    let ntracks = usize::from(r.u16le()?);
    let nins = usize::from(r.u16le()?);
    let nsmp = usize::from(r.u16le()?);
    let speed = r.u8()?;
    let tempo = r.u8()?;
    let flags = r.u32le()?;
    let gv = r.u8()?;
    let sep = r.u8()?;
    let sample_volume = r.i8()?;
    let chn_vol = r.bytes(64)?.to_vec();
    let chn_pan = r.bytes(64)?.to_vec();
    let sfx = r.bytes(16)?.to_vec();
    let fixed = r.bytes(256)?.to_vec();
    if nch == 0 || nch > 64 || nins >= 256 || nsmp >= 4000 {
        return bad("its header doesn't add up");
    }
    let ty = if flags & 0x100 != 0 {
        Mo3Type::It
    } else if flags & 2 != 0 {
        Mo3Type::S3m
    } else if flags & 0x80 != 0 {
        Mo3Type::Mod
    } else if flags & 8 != 0 {
        Mo3Type::Mtm
    } else {
        Mo3Type::Xm
    };
    let s3m_like = matches!(ty, Mo3Type::S3m | Mo3Type::It);
    let separators = !matches!(ty, Mo3Type::Mod | Mo3Type::Xm);
    let orders: Vec<u16> = r
        .bytes(norders)?
        .iter()
        .map(|&o| match o {
            0xFF if separators => ORDER_END,
            0xFE if separators => ORDER_SKIP,
            o => u16::from(o),
        })
        .collect();
    let track_of = r.bytes(npat * nch * 2)?.to_vec();
    let lens = r.bytes(npat * 2)?.to_vec();
    let mut tracks = Vec::with_capacity(ntracks);
    for _ in 0..ntracks {
        let n = r.u32le()? as usize;
        if n >= 0x20_0000 {
            return bad("a track is too long");
        }
        tracks.push(r.bytes(n)?);
    }

    // Patterns: each channel's track, its rows of commands.
    let note_offset = match ty {
        Mo3Type::It => 1,
        Mo3Type::Mtm => 14,
        _ => 13,
    };
    let mut patterns = Vec::with_capacity(npat);
    for p in 0..npat {
        let rows = usize::from(u16::from_le_bytes([lens[p * 2], lens[p * 2 + 1]])).clamp(1, 1024);
        let mut pat = Pattern::new(rows, nch);
        for ch in 0..nch {
            let i = (p * nch + ch) * 2;
            let t = usize::from(u16::from_le_bytes([track_of[i], track_of[i + 1]]));
            let Some(track) = tracks.get(t) else { continue };
            let mut tr = Reader::new(track);
            let mut row = 0;
            while row < rows {
                let Ok(b) = tr.u8() else { break };
                if b == 0 {
                    break;
                }
                let (count, rep) = (b & 15, usize::from(b >> 4));
                let mut c = Cell::default();
                for _ in 0..count {
                    let (Ok(cmd), Ok(v)) = (tr.u8(), tr.u8()) else { break };
                    mo3_command(&mut c, cmd, v, ty, note_offset);
                }
                for _ in 0..rep {
                    if row >= rows {
                        break;
                    }
                    pat.cells[row * nch + ch] = c;
                    row += 1;
                }
            }
        }
        patterns.push(Arc::new(pat));
    }

    // Instruments (IT's sample mode stores them too, unused).
    let instrument_mode = ty == Mo3Type::Xm || (flags & 0x200 != 0 && s3m_like);
    let mut instruments = Vec::new();
    let mut xm_vibrato = Vec::new();
    for _ in 0..nins {
        let name = cstr(&mut r)?;
        let filename = if version >= 5 { cstr(&mut r)? } else { String::new() };
        let ins_start = r.pos;
        let _flags = r.u32le()?;
        let map = r.bytes(480)?;
        let xm = ty == Mo3Type::Xm;
        let volume_env = mo3_envelope(&mut r, 0, false, xm)?;
        let pan_env = mo3_envelope(&mut r, 0, true, xm)?;
        let pitch_env = mo3_envelope(&mut r, 5, true, xm)?;
        let vib = AutoVibrato { kind: r.u8()?, sweep: r.u8()?, depth: r.u8()?, speed: r.u8()? };
        let fadeout = r.u16le()?;
        let (_mch, _mbank, _mpatch, _mbend) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
        let gvol = r.u8()?;
        let pan = r.u16le()?;
        let (nna, pps, ppc, dct, dca) = (r.u8()?, r.i8()?, r.u8()?, r.u8()?, r.u8()?);
        let (vswing, pswing) = (r.u16le()?, r.u16le()?);
        let (cutoff, res) = (r.u8()?, r.u8()?);
        r.seek(ins_start + 826)?;
        let keymap = (0..120)
            .map(|n| {
                let at = n * 4;
                let note = u16::from_le_bytes([map[at], map[at + 1]]);
                let smp = u16::from_le_bytes([map[at + 2], map[at + 3]]).wrapping_add(1);
                if xm {
                    // XM's 96 notes, an octave up here.
                    let src = n.wrapping_sub(12);
                    let smp = if src < 96 { u16::from_le_bytes([map[src * 4 + 2], map[src * 4 + 3]]).wrapping_add(1) } else { 0 };
                    (n as u8, smp)
                } else {
                    ((note.min(119)) as u8, smp)
                }
            })
            .collect();
        xm_vibrato.push(vib);
        instruments.push(Instrument {
            name,
            filename,
            keymap,
            volume_env,
            pan_env,
            pitch_env,
            fadeout: u32::from(fadeout) * 2,
            global_volume: if ty == Mo3Type::It { gvol.min(128) } else { 128 },
            pan: (pan <= 256).then(|| pan.min(255) as u8),
            nna: Nna::from_u8(nna),
            dct,
            dca,
            pitch_pan_separation: pps,
            pitch_pan_center: ppc,
            random_volume: vswing.min(100) as u8,
            random_pan: (pswing.min(256) / 4) as u8,
            cutoff: (cutoff & 0x80 != 0).then_some(cutoff & 0x7F),
            resonance: (res & 0x80 != 0).then_some(res & 0x7F),
            ..Default::default()
        });
    }
    if !instrument_mode {
        instruments.clear();
    }

    // The samples' headers.
    let hertz = version >= 5 || flags & 1 == 0;
    let mut heads: Vec<Mo3Sample> = Vec::with_capacity(nsmp);
    for _ in 0..nsmp {
        let name = cstr(&mut r)?;
        let filename = if version >= 5 { cstr(&mut r)? } else { String::new() };
        let freq = r.u32le()?;
        let transpose = r.i8()?;
        let volume = r.u8()?;
        let pan = r.u16le()?;
        let length = r.u32le()? as usize;
        let ls = r.u32le()?;
        let le = r.u32le()?;
        let sflags = r.u16le()?;
        let (vtype, vsweep, vdepth, vrate) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
        let sgv = r.u8()?;
        let ss = r.u32le()?;
        let se = r.u32le()?;
        let packed = r.i32le()?;
        let delay = r.u16le()?;
        let shared = if version >= 5 && sflags & 0xF000 == 0x7000 { r.i16le()? } else { 0 };
        let c5 = if s3m_like {
            if hertz {
                freq.clamp(1, 9_999_999)
            } else {
                (8363.0 * 2f64.powf(f64::from(freq as i32 + 1408) / 1536.0)).round().clamp(1.0, 9_999_999.0) as u32
            }
        } else {
            let ft = if ty == Mo3Type::Mtm { freq as u8 as i8 } else { (freq as u8).wrapping_sub(128) as i8 };
            relative_c5(transpose, ft)
        };
        let lk = |on: bool, ping: bool| if !on { LoopKind::Off } else if ping { LoopKind::PingPong } else { LoopKind::Forward };
        let sample = Sample {
            name,
            filename,
            c5,
            volume: volume.min(64),
            global_volume: if ty == Mo3Type::It { sgv.min(64) } else { 64 },
            pan: (pan <= 256).then(|| pan.min(255) as u8),
            looped: Loop { kind: lk(sflags & 0x10 != 0, sflags & 0x20 != 0), start: ls, end: le },
            sustain: Loop { kind: lk(sflags & 0x100 != 0, sflags & 0x200 != 0), start: ss, end: se },
            vibrato: AutoVibrato { kind: vtype & 7, speed: vrate, depth: vdepth, sweep: vsweep },
            ..Sample::empty()
        };
        heads.push(Mo3Sample { sample, flags: sflags, length: length.min(64 << 20), packed, delay, shared, chunk: (0, 0) });
    }
    if ty == Mo3Type::Xm {
        // An XM instrument's auto-vibrato is its samples'.
        for (ins, vib) in instruments.iter().zip(&xm_vibrato) {
            for &(_, s) in &ins.keymap {
                if let Some(h) = heads.get_mut(usize::from(s).wrapping_sub(1)) {
                    h.sample.vibrato = *vib;
                }
            }
        }
    }
    // What's after the samples' headers (plugins, OpenMPT's extensions)
    // isn't kept, only the tracker's version and the rows a beat.
    let mut tracker = String::new();
    let mut time = TimeSig::default();
    if flags & 0x10_0000 == 0 {
        while r.left() >= 8 {
            let (Ok(id), Ok(n)) = (r.tag(), r.u32le()) else { break };
            let Ok(chunk) = r.bytes(n as usize) else { break };
            match &id {
                b"VERS" if ty == Mo3Type::Xm => tracker = crate::io::text(&chunk[..chunk.len().min(32)]),
                b"PRHI" if chunk.len() >= 2 && chunk[0] > 0 && chunk[1] >= chunk[0] => time = TimeSig { rows_per_beat: u16::from(chunk[0]), rows_per_measure: u16::from(chunk[1]) },
                _ => {}
            }
        }
    }

    // The samples' data, in a first pass (an Ogg sample's headers may be
    // a later sample's).
    for h in heads.iter_mut() {
        let compression = h.flags & 0xF000;
        let ch = if h.flags & 0x400 != 0 { 2 } else { 1 };
        let b16 = h.flags & 1 != 0;
        if compression == 0 && h.packed == 0 {
            let bytes = h.length * ch * if b16 { 2 } else { 1 };
            let end = samples_at.saturating_add(bytes).min(d.len());
            let raw = decode(&d[samples_at.min(end)..end], if b16 { Pcm::S16le } else { Pcm::S8 }, h.length * ch);
            let pcm = if ch == 2 && raw.len() == h.length * 2 { crate::io::interleave(&raw) } else { raw };
            let ch = if ch == 2 && pcm.len() == h.length * 2 { 2 } else { 1 };
            h.sample.wave = Arc::new(Wave::new(if b16 { 16 } else { 8 }, ch, pcm));
            samples_at = end;
        } else if h.packed > 0 {
            let end = samples_at.saturating_add(h.packed as usize).min(d.len());
            h.chunk = (samples_at.min(end), end);
            samples_at = end;
        }
    }
    for i in 0..heads.len() {
        let h = &heads[i];
        if h.packed < 0 {
            let from = i as i64 + i64::from(h.packed);
            if from >= 0 && (from as usize) < i {
                heads[i].sample.wave = heads[from as usize].sample.wave.clone();
            }
            continue;
        }
        if h.length == 0 || h.chunk.1 <= h.chunk.0 {
            continue;
        }
        let ch = if h.flags & 0x400 != 0 { 2 } else { 1 };
        let b16 = h.flags & 1 != 0;
        let data = &d[h.chunk.0..h.chunk.1];
        let wave = match h.flags & 0xF000 {
            0x2000 | 0x4000 => {
                // At best two bits a point.
                let frames = h.length.min(data.len().saturating_mul(4) / ch);
                Some(Wave::new(if b16 { 16 } else { 8 }, ch as u8, mo3_delta(data, frames, ch, b16, h.flags & 0xF000 == 0x4000)))
            }
            0x3000 | 0x7000 => {
                let from = i as i64 + i64::from(h.shared);
                let header = (h.shared != 0 && h.delay > 0 && from >= 0 && (from as usize) < heads.len() && from as usize != i).then(|| {
                    let (a, b) = heads[from as usize].chunk;
                    &d[a..b.min(a + usize::from(h.delay))]
                });
                let ogg: Vec<u8> = match header {
                    Some(hd) => [hd, data].concat(),
                    None => data.to_vec(),
                };
                crate::vorbis::decode_ogg(&ogg).filter(|v| v.channels <= 2).map(|v| {
                    let vch = usize::from(v.channels);
                    let mut pcm = v.pcm;
                    pcm.truncate(h.length * vch);
                    Wave::new(16, v.channels, pcm)
                })
            }
            0x8000 => {
                if let Some(b) = data.get(..11) {
                    heads[i].sample.fm = Some(FmPatch::from_s3m(b.try_into().expect("eleven bytes")));
                }
                None
            }
            _ => None,
        };
        // MP3 samples (no decoder yet) stay silent.
        if let Some(w) = wave {
            heads[i].sample.wave = Arc::new(w);
        }
    }
    let samples: Vec<Sample> = heads.into_iter().map(|h| h.sample).collect();

    let pan = |i: usize| -> (u8, bool) {
        if ty == Mo3Type::Xm {
            return (128, false);
        }
        match chn_pan[i] {
            127 => (128, true),
            255 => (255, false),
            p => (p, false),
        }
    };
    let channels = (0..nch)
        .map(|i| {
            let (p, surround) = pan(i.min(63));
            Channel { pan: p, surround, volume: if ty == Mo3Type::It { chn_vol[i.min(63)].min(64) } else { 64 }, ..Default::default() }
        })
        .collect();
    let preamp = if sample_volume < 0 { i32::from(sample_volume) + 52 } else { (f64::from(sample_volume) * 3.1 / 20.0).exp() as i32 + 51 };
    let (kind, fmt_name, orig) = match ty {
        Mo3Type::It => (Kind::It, "Impulse Tracker", "it"),
        Mo3Type::S3m => (Kind::S3m, "Scream Tracker 3", "s3m"),
        Mo3Type::Xm => (Kind::Xm, "FastTracker 2", "xm"),
        Mo3Type::Mod => (Kind::Mod, "ProTracker", "mod"),
        Mo3Type::Mtm => (Kind::Mod, "MultiTracker", "mtm"),
    };
    let macros = (sfx.iter().any(|&b| b != 0) || fixed.chunks(2).any(|c| c[1] != 0)).then(|| MidiMacros {
        parametered: sfx.iter().map(|&b| if b != 0 { format!("F0F0{:02X}z", b - 1) } else { String::new() }).collect(),
        fixed: fixed.chunks(2).map(|c| if c[1] != 0 { format!("F0F0{:02X}{:02X}", c[1] - 1, c[0]) } else { String::new() }).collect(),
    });
    let mut m = Module {
        title,
        message,
        format: "mo3".into(),
        format_name: format!("MO3 ({fmt_name})"),
        tracker: if tracker.is_empty() { format!("MO3 {version}, from {orig}") } else { format!("MO3 {version} ({tracker})") },
        kind,
        flags: Flags {
            linear: flags & 1 != 0,
            amiga_limits: ty == Mo3Type::S3m && flags & 0x10 != 0,
            instruments: instrument_mode,
            // MO3 keeps these two the other way round from IT.
            old_effects: ty == Mo3Type::It && flags & 0x800 == 0,
            compat_gxx: ty == Mo3Type::It && flags & 0x400 == 0,
            fast_slides: ty == Mo3Type::S3m && flags & 4 != 0,
            vblank: flags & 0x8_0000 != 0,
        },
        speed: if speed == 0 { 6 } else { speed },
        tempo: if tempo == 0 { 125 } else { u16::from(tempo) },
        global_volume: match ty {
            Mo3Type::It => gv.min(128),
            Mo3Type::S3m => gv.min(64) * 2,
            _ => 128,
        },
        mix_volume: if s3m_like { preamp.clamp(16, 128) as u8 } else { 32 },
        separation: if ty == Mo3Type::It { sep.min(128) } else { 128 },
        restart: restart.min(norders.saturating_sub(1) as u16),
        channels,
        orders,
        patterns,
        samples,
        instruments,
        time,
        macros,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

fn mo3_command(c: &mut Cell, cmd: u8, v: u8, ty: Mo3Type, note_offset: u8) {
    let s3m_like = matches!(ty, Mo3Type::S3m | Mo3Type::It);
    match cmd {
        0x01 => {
            c.note = match v {
                0..=119 => (v + note_offset).min(NOTE_MAX),
                0xFF => NOTE_OFF,
                0xFE => NOTE_CUT,
                _ => NOTE_FADE,
            }
        }
        0x02 => c.instr = v.wrapping_add(1),
        // The volume column's, where the tracker had one.
        0x06 if c.vol == Vol::None && ty == Mo3Type::Xm && v & 15 == 0 => (c.vol, c.volp) = (Vol::TonePorta, v >> 4),
        0x06 if c.vol == Vol::None && ty == Mo3Type::It && [0u8, 1, 4, 8, 16, 32, 64, 96, 128, 255].contains(&v) => (c.vol, c.volp) = (Vol::TonePorta, v),
        0x07 if c.vol == Vol::None && ty == Mo3Type::It && v < 10 => (c.vol, c.volp) = (Vol::VibratoDepth, v),
        0x0B if c.vol == Vol::None && ty == Mo3Type::It && v == 0xFF => (c.vol, c.volp) = (Vol::Panning, 64),
        0x0B if c.vol == Vol::None && ((ty == Mo3Type::It && v & 3 == 0) || (ty == Mo3Type::Xm && v & 15 == 0)) => (c.vol, c.volp) = (Vol::Panning, v / 4),
        0x0F if ty != Mo3Type::Mod && c.vol == Vol::None && v <= 64 => (c.vol, c.volp) = (Vol::Volume, v),
        0x0F if s3m_like => (c.fx, c.param) = (Fx::Volume, v.min(64)),
        0x14 | 0x15 => {
            let up = v & 0xF0 != 0;
            c.vol = match (cmd == 0x14, up) {
                (true, true) => Vol::VolSlideUp,
                (true, false) => Vol::VolSlideDown,
                (false, true) => Vol::FineVolUp,
                (false, false) => Vol::FineVolDown,
            };
            c.volp = if up { v >> 4 } else { v & 15 };
        }
        0x1B => (c.vol, c.volp) = if v & 0xF0 != 0 { (Vol::PanSlideRight, v >> 4) } else { (Vol::PanSlideLeft, v & 15) },
        0x1D => (c.fx, c.param) = (Fx::ExtraFinePorta, 0x10 | (v & 15)),
        0x1E => (c.fx, c.param) = (Fx::ExtraFinePorta, 0x20 | (v & 15)),
        0x1F => (c.vol, c.volp) = (Vol::VibratoSpeed, v),
        0x20 => (c.vol, c.volp) = (Vol::VibratoDepth, v),
        0x22 => {
            c.fx = match c.fx {
                Fx::TonePorta => Fx::TonePortaVol,
                Fx::Vibrato => Fx::VibratoVol,
                _ => Fx::VolSlide,
            };
            c.param = v;
        }
        0x30 => {
            c.volp = v % 10;
            c.vol = match v {
                0..=9 => Vol::FineVolUp,
                10..=19 => Vol::FineVolDown,
                20..=29 => Vol::VolSlideUp,
                30..=39 => Vol::VolSlideDown,
                _ => Vol::None,
            };
        }
        0x31 => (c.vol, c.volp) = (Vol::PortaDown, v),
        0x32 => (c.vol, c.volp) = (Vol::PortaUp, v),
        0x34 if (223..=232).contains(&v) => (c.vol, c.volp) = (Vol::Offset, v - 223),
        0x35 => (c.fx, c.param) = (Fx::XParam, v),
        0x37 => (c.fx, c.param) = (Fx::DelayCut, v),
        0x2E => (c.fx, c.param) = (Fx::Panbrello, v),
        0x2F => (c.fx, c.param) = (Fx::Midi, v),
        _ if s3m_like => {
            if let Some(l) = mo3_s3m_letter(cmd) {
                (c.fx, c.param) = s3m_effect(l, v, ty == Mo3Type::It);
            }
        }
        _ => {
            if let Some(n) = mo3_mod_number(cmd) {
                (c.fx, c.param) = mod_effect(n, v, ty == Mo3Type::Xm);
            }
        }
    }
}

// ---- Skale Tracker ----
//
// No description of it was published; this is what the files show.
// Chunks of (u16 type, u16 id, u32 length) all through: type 1 holds
// more chunks or a structure, type 3 (and 4 and 6 in later versions)
// data. At the top: "ALIM3" and the version (7000 or 7600); the song
// header (the title, the comment in lines ended by CR, then the tempo,
// speed, channels and version); the song (the channel count again, 256
// orders, how many play, the restart, then a chunk a pattern); the
// instruments; and two the player doesn't need. A pattern: its number,
// its rows, then records of 01, a kind (01 a track, FF the end) and a
// length; a track is its channel, its last and first row, then a cell a
// row: a flag byte, top bit set naming what follows (effect, note,
// instrument, a 16-bit volume word, parameter), clear being the effect
// of a full cell (note, instrument, volume word, parameter). Notes count
// from C-0 at 0 (81 is key off); effects are FastTracker 2's; the
// volume word is a volume of 0 to 128, or with its top bit set, an XM
// volume column byte. An instrument: its number and name, a 96-note
// keymap (six bytes a note, the third its sample) in the later version,
// then its samples: number, name, volume (0 to 128), panning, finetune
// (128ths of a semitone), relative note, bits (8 or 16), the loop's
// start, end and kind; then their data: the points, then (later
// version) the channels and the packed length, and the PCM or an Ogg
// Vorbis stream.

pub fn probe_skm(d: &[u8]) -> bool {
    d.len() >= 17 && d[..4] == [1, 0, 0xFE, 0xFF] && &d[8..13] == b"ALIM3"
}

/// The chunks in `d`: (type, id, body).
fn skm_chunks(d: &[u8]) -> Vec<(u16, u16, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= d.len() {
        let t = u16::from_le_bytes([d[at], d[at + 1]]);
        let id = u16::from_le_bytes([d[at + 2], d[at + 3]]);
        let n = u32::from_le_bytes([d[at + 4], d[at + 5], d[at + 6], d[at + 7]]) as usize;
        let end = (at + 8).saturating_add(n).min(d.len());
        out.push((t, id, &d[at + 8..end]));
        at = end;
    }
    out
}

fn cstring(d: &[u8], at: usize) -> Option<(String, usize)> {
    let n = d.get(at..)?.iter().position(|&b| b == 0)?;
    Some((crate::io::text(&d[at..at + n]), at + n + 1))
}

/// A sample: its header, then a data chunk (type 3 or 4) right after it.
fn skm_sample(body: &[u8]) -> Option<(u8, Sample)> {
    let idx = *body.first()?;
    let (name, at) = cstring(body, 1)?;
    let h = body.get(at..at + 14)?;
    let (vol, pan, ft, rel, bits) = (h[0], h[1], h[2] as i8, h[3] as i8, h[4]);
    let ls = u32::from_le_bytes([h[5], h[6], h[7], h[8]]);
    let le = u32::from_le_bytes([h[9], h[10], h[11], h[12]]);
    let kind = h[13];
    let chunk = skm_chunks(&body[at + 14..]);
    let &(t, _, data) = chunk.first()?;
    if !(t == 3 || t == 4) || data.len() < 5 {
        return None;
    }
    let points = u32::from_le_bytes(data[..4].try_into().ok()?) as usize;
    let b16 = bits == 16;
    let (channels, pcm_bytes) = if t == 4 && data.len() >= 9 {
        let n = u32::from_le_bytes(data[5..9].try_into().ok()?) as usize;
        (data[4].clamp(1, 2), &data[9..(9 + n).min(data.len())])
    } else {
        (1, &data[5..])
    };
    let wave = if pcm_bytes.starts_with(b"OggS") {
        let v = crate::vorbis::decode_ogg(pcm_bytes)?;
        let mut pcm = v.pcm;
        pcm.truncate(points.max(1) / usize::from(v.channels.max(1)) * usize::from(v.channels.max(1)));
        Wave::new(16, v.channels.min(2), pcm)
    } else {
        let raw = decode(pcm_bytes, if b16 { Pcm::S16le } else { Pcm::S8 }, points.min(pcm_bytes.len() / if b16 { 2 } else { 1 }));
        Wave::new(if b16 { 16 } else { 8 }, channels, raw)
    };
    let kind = match kind {
        1 => LoopKind::Forward,
        2 => LoopKind::PingPong,
        _ => LoopKind::Off,
    };
    Some((
        idx,
        Sample {
            name,
            wave: Arc::new(wave),
            c5: relative_c5(rel, ft),
            volume: (vol / 2).min(64),
            pan: Some(pan.min(255)),
            looped: Loop { kind, start: ls, end: le },
            ..Sample::empty()
        },
    ))
}

/// An instrument's samples: the type 1, id 1 chunks in it that hold one.
fn skm_samples(ins: &[u8]) -> Vec<(usize, u8, Sample)> {
    let mut out = Vec::new();
    let mut j = 0;
    while j + 8 <= ins.len() {
        if ins[j..j + 4] == [1, 0, 1, 0] {
            let n = u32::from_le_bytes(ins[j + 4..j + 8].try_into().unwrap()) as usize;
            if let Some(body) = ins.get(j + 8..j + 8 + n) {
                if let Some((idx, s)) = skm_sample(body) {
                    out.push((j, idx, s));
                    j += 8 + n;
                    continue;
                }
            }
        }
        j += 1;
    }
    out
}

fn skm_volume(w: u16) -> (Vol, u8) {
    if w & 0x8000 != 0 {
        super::xm::volume_column(w as u8)
    } else {
        (Vol::Volume, (w.min(128) / 2) as u8)
    }
}

pub fn load_skm(d: &[u8]) -> Result<Module> {
    let top = skm_chunks(d);
    let mut title = String::new();
    let mut message = String::new();
    let (mut tempo, mut speed, mut nch) = (125u16, 6u8, 0usize);
    let mut song: Option<&[u8]> = None;
    let mut inst_chunk: Option<&[u8]> = None;
    for &(t, id, body) in &top {
        match (t, id) {
            (3, 1) => {
                if let Some((t, at)) = cstring(body, 0) {
                    title = t;
                    if let Some((m, _)) = cstring(body, at) {
                        message = m.split('\r').map(str::trim_end).collect::<Vec<_>>().join("\n").trim_end().to_string();
                    }
                }
                if body.len() >= 8 {
                    let tail = &body[body.len() - 8..];
                    tempo = u16::from(tail[0]);
                    speed = tail[1];
                    nch = usize::from(u16::from_le_bytes([tail[2], tail[3]]));
                }
            }
            (1, 2) => song = Some(body),
            (1, 3) => inst_chunk = Some(body),
            _ => {}
        }
    }
    let song = song.filter(|s| s.len() >= 259).ok_or_else(|| crate::io::Error::Bad("it has no song".into()))?;
    let norders = usize::from(song[257]).max(1);
    let restart = u16::from(song[258]);
    let orders: Vec<u16> = song[1..1 + norders].iter().map(|&o| u16::from(o)).collect();
    nch = nch.max(usize::from(song[0]));

    // Patterns, by their numbers.
    let mut raw: Vec<(usize, usize, Vec<(usize, usize, Cell)>)> = Vec::new();
    for (t, _, p) in skm_chunks(&song[259..]) {
        if t != 1 || p.len() < 4 {
            continue;
        }
        let idx = usize::from(p[0]);
        let rows = usize::from(u16::from_le_bytes([p[1], p[2]])).clamp(1, 1024);
        let mut cells = Vec::new();
        let mut pos = 3;
        while pos + 4 <= p.len() && p[pos + 1] == 1 {
            let n = usize::from(u16::from_le_bytes([p[pos + 2], p[pos + 3]]));
            let Some(tr) = p.get(pos + 4..pos + 4 + n) else { break };
            pos += 4 + n;
            if tr.len() < 3 {
                continue;
            }
            let (ch, last, first) = (usize::from(tr[0]), usize::from(tr[1]), usize::from(tr[2]));
            nch = nch.max(ch + 1);
            let mut r = Reader::new(&tr[3..]);
            let mut row = first;
            while row <= last.max(first) {
                let Ok(flag) = r.u8() else { break };
                let mut c = Cell::default();
                let (mut fx, mut note, mut ins, mut vol, mut param) = (None, None, None, None, None);
                if flag & 0x80 != 0 {
                    if flag & 1 != 0 {
                        fx = r.u8().ok();
                    }
                    if flag & 2 != 0 {
                        note = r.u8().ok();
                    }
                    if flag & 4 != 0 {
                        ins = r.u8().ok();
                    }
                    if flag & 8 != 0 {
                        vol = r.u16le().ok();
                    }
                    if flag & 16 != 0 {
                        param = r.u8().ok();
                    }
                } else {
                    fx = Some(flag);
                    note = r.u8().ok().filter(|&n| n != 0);
                    ins = r.u8().ok();
                    vol = r.u16le().ok();
                    param = r.u8().ok();
                }
                if let Some(n) = note {
                    c.note = if n >= 0x80 { NOTE_OFF } else { (n + 13).min(NOTE_MAX) };
                }
                c.instr = ins.unwrap_or(0);
                if let Some(v) = vol {
                    (c.vol, c.volp) = skm_volume(v);
                }
                if fx.is_some() || param.is_some() {
                    (c.fx, c.param) = mod_effect(fx.unwrap_or(0), param.unwrap_or(0), true);
                }
                cells.push((row, ch, c));
                row += 1;
            }
        }
        raw.push((idx, rows, cells));
    }
    let nch = nch.clamp(1, 64);
    let npat = raw.iter().map(|p| p.0 + 1).max().unwrap_or(1);
    let mut patterns: Vec<Pattern> = (0..npat).map(|_| Pattern::new(64, nch)).collect();
    for (idx, rows, cells) in raw {
        let mut p = Pattern::new(rows, nch);
        for (row, ch, c) in cells {
            if let Some(x) = p.cell_mut(row, ch) {
                *x = c;
            }
        }
        patterns[idx] = p;
    }

    // Instruments, by their numbers, and their samples.
    let mut instruments: Vec<Instrument> = Vec::new();
    let mut samples: Vec<Sample> = Vec::new();
    for (t, _, ins) in inst_chunk.map(skm_chunks).unwrap_or_default() {
        if !(t == 3 || t == 6) || ins.len() < 2 {
            continue;
        }
        let idx = usize::from(ins[0]);
        let Some((name, after)) = cstring(ins, 1) else { continue };
        let smps = skm_samples(ins);
        let first = samples.len();
        // The later version's keymap: six bytes a note, its sample third.
        let keymap: Vec<u16> = if t == 6 && ins.len() >= after + 576 && smps.first().is_some_and(|s| s.0 >= after + 576) {
            (0..96).map(|n| u16::from(ins[after + n * 6 + 2])).collect()
        } else {
            vec![0; 96]
        };
        let by_idx: Vec<u8> = smps.iter().map(|s| s.1).collect();
        for (_, _, s) in smps {
            samples.push(s);
        }
        while instruments.len() <= idx {
            instruments.push(Instrument::for_sample("", 0));
        }
        let count = samples.len() - first;
        instruments[idx] = Instrument {
            name,
            keymap: (0..120)
                .map(|n| {
                    let k = n as i32 - 12;
                    let local = if (0..96).contains(&k) { keymap[k as usize] } else { 0 };
                    let pos = by_idx.iter().position(|&i| u16::from(i) == local).or((count > 0).then_some(0));
                    (n as u8, pos.map_or(0, |p| (first + p + 1) as u16))
                })
                .collect(),
            global_volume: 128,
            ..Default::default()
        };
    }

    let mut m = Module {
        title,
        message,
        format: "skm".into(),
        format_name: "Skale Tracker".into(),
        tracker: "Skale Tracker".into(),
        kind: Kind::Xm,
        flags: Flags { linear: true, instruments: true, ..Default::default() },
        speed: speed.max(1),
        tempo: tempo.clamp(32, 999),
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        restart,
        channels: (0..nch).map(|_| Channel::default()).collect(),
        orders,
        patterns: patterns.into_iter().map(Arc::new).collect(),
        samples,
        instruments,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}
