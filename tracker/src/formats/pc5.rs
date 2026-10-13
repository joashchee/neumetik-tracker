//! More of the PC's (and the Mac's) trackers: Jazz Jackrabbit 2's Galaxy
//! Sound System modules (.j2b, and the RIFF AM and AMFF inside),
//! DigiTrekker (.dtm), PlayerPro (.mad, .madk...), Ken's Digital
//! Music (.kdm, its waves beside it) and the Digital Sound Interface
//! Kit's first format (.dsm).

use std::sync::Arc;

use super::common::mod_ex_to_s3m;
use crate::io::{bad, decode, Error, Pcm, Reader, Result};
use crate::model::*;

// ---- Galaxy Sound System (Jazz Jackrabbit 2) ----
//
// A J2B is "MUSE", a mark (DEADBEAF for the newer RIFF AM, DEADBABE for
// the older AMFF), the file's length, a CRC, the packed and unpacked
// lengths, then a zlib stream: the RIFF module. Inside, a MAIN (AMFF)
// or INIT (AM) chunk: the title, flags, channels, speed, tempo, global
// volume and the channels' panning; ORDR, the orders; a PATT chunk a
// pattern (packed rows: a byte with the channel and what follows, an
// effect, a note and instrument, a volume); then each instrument with
// its samples. AMFF's are flat INST chunks; AM's are RIFF AI chunks
// holding an INST and a RIFF AS chunk a sample. Both were made by
// Epic's converter from S3M and IT, and play by IT's rules.

pub fn probe_j2b(d: &[u8]) -> bool {
    if d.len() >= 24 && &d[..4] == b"MUSE" && matches!(&d[4..8], [0xDE, 0xAD, 0xBE, 0xAF] | [0xDE, 0xAD, 0xBA, 0xBE]) {
        return true;
    }
    probe_am(d)
}

fn probe_am(d: &[u8]) -> bool {
    d.len() >= 12 + 8 + 73 && &d[..4] == b"RIFF" && matches!(&d[8..12], b"AMFF" | b"AM  ")
}

pub fn load_j2b(d: &[u8]) -> Result<Module> {
    if probe_am(d) {
        return load_am(d);
    }
    let mut r = Reader::at(d, 16);
    let packed = r.u32le()? as usize;
    let unpacked = r.u32le()? as usize;
    let body = d.get(24..24usize.saturating_add(packed).min(d.len())).unwrap_or(&[]);
    let am = crate::unpack::zlib(body, unpacked.min(64 << 20)).ok_or_else(|| Error::Bad("its zlib packing is damaged".into()))?;
    if !probe_am(&am) {
        return bad("it isn't a RIFF AM module inside");
    }
    let mut m = load_am(&am)?;
    m.format = "j2b".into();
    m.format_name = "Jazz Jackrabbit 2 (Galaxy Sound System)".into();
    Ok(m)
}

/// The chunks of a RIFF body from `at` to `end`: (tag, start, length).
/// AM pads them to even lengths; AMFF doesn't.
fn riff_chunks(d: &[u8], mut at: usize, end: usize, pad: bool) -> Vec<([u8; 4], usize, usize)> {
    let end = end.min(d.len());
    let mut out = Vec::new();
    while at + 8 <= end {
        let tag = [d[at], d[at + 1], d[at + 2], d[at + 3]];
        let n = u32::from_le_bytes([d[at + 4], d[at + 5], d[at + 6], d[at + 7]]) as usize;
        let start = at + 8;
        let len = n.min(end - start);
        out.push((tag, start, len));
        at = start.saturating_add(n);
        if pad && n % 2 == 1 {
            at += 1;
        }
    }
    out
}

/// A Galaxy effect (Epic's numbering) in the shared command set.
fn am_effect(cmd: u8, p: u8) -> (Fx, u8) {
    match cmd {
        0 if p != 0 => (Fx::Arpeggio, p),
        1 => (Fx::PortaUp, p),
        2 => (Fx::PortaDown, p),
        3 => (Fx::TonePorta, p),
        4 => (Fx::Vibrato, p),
        5 | 6 | 10 => (
            match cmd {
                5 => Fx::TonePortaVol,
                6 => Fx::VibratoVol,
                _ => Fx::VolSlide,
            },
            if p & 0xF0 != 0 { p & 0xF0 } else { p },
        ),
        7 => (Fx::Tremolo, p),
        8 if p == 0xA4 => (Fx::S3mEx, 0x91),
        8 if p <= 0x80 => (Fx::Panning, (u16::from(p) * 2).min(255) as u8),
        8 => (Fx::Panning, p),
        9 => (Fx::Offset, p),
        11 => (Fx::PositionJump, p),
        12 => (Fx::Volume, p.min(64)),
        13 => (Fx::PatternBreak, (p >> 4) * 10 + (p & 0x0F)),
        14 => mod_ex_to_s3m(p),
        15 if p <= 0x1F => (Fx::Speed, p),
        15 => (Fx::Tempo, p),
        16 => (Fx::GlobalVolume, p.min(64) * 2),
        17 => (Fx::GlobalVolSlide, if p & 0xF0 != 0 { p & 0xF0 } else { p }),
        18 => (Fx::KeyOff, p),
        19 => (Fx::EnvelopePosition, p),
        20 => (Fx::ChannelVolume, p.min(64)),
        21 => (Fx::ChannelVolSlide, p),
        22 => (Fx::PanningSlide, if p & 0xF0 != 0 { p & 0xF0 } else { p }),
        23 => (Fx::Retrig, p),
        24 => (Fx::Tremor, p),
        25 => match p >> 4 {
            1 => (Fx::PortaUp, 0xE0 | (p & 0x0F)),
            2 => (Fx::PortaDown, 0xE0 | (p & 0x0F)),
            _ => (Fx::None, 0),
        },
        _ => (Fx::None, 0),
    }
}

fn am_pattern(b: &[u8], channels: usize, am: bool) -> Pattern {
    let rows = usize::from(b.first().copied().unwrap_or(63)) + 1;
    let mut p = Pattern::new(rows, channels);
    let mut r = Reader::at(b, 1);
    let mut row = 0;
    while row < rows {
        let Ok(flags) = r.u8() else { break };
        if flags == 0 {
            row += 1;
            continue;
        }
        if flags & 0xE0 == 0 {
            continue;
        }
        let mut cell = Cell::default();
        if flags & 0x80 != 0 {
            let (Ok(param), Ok(cmd)) = (r.u8(), r.u8()) else { break };
            let (fx, v) = am_effect(cmd, param);
            if fx == Fx::Volume && flags & 0x20 == 0 {
                (cell.vol, cell.volp) = (Vol::Volume, v);
            } else {
                (cell.fx, cell.param) = (fx, v);
            }
        }
        if flags & 0x40 != 0 {
            let (Ok(instr), Ok(note)) = (r.u8(), r.u8()) else { break };
            cell.instr = instr;
            cell.note = match note {
                0x80 => NOTE_OFF,
                n if n > 0x80 => NOTE_FADE,
                n => n.min(NOTE_MAX),
            };
        }
        if flags & 0x20 != 0 {
            let Ok(v) = r.u8() else { break };
            let v = if am { (u16::from(v) * 64 / 127) as u8 } else { v };
            (cell.vol, cell.volp) = (Vol::Volume, v.min(64));
        }
        if let Some(c) = p.cell_mut(row, usize::from(flags & 0x1F).min(channels - 1)) {
            *c = cell;
        }
    }
    p
}

/// Epic's auto-vibrato waveforms (sine, square, ramp up, ramp down,
/// random) as the model's.
fn am_vibrato(kind: u8, sweep: u16, depth: u16, rate: u16) -> AutoVibrato {
    let speed = (rate / 16).min(255) as u8;
    let depth = (depth / 4).min(255) as u8;
    AutoVibrato {
        kind: [0, 2, 4, 1, 3].get(usize::from(kind)).copied().unwrap_or(0),
        speed,
        depth,
        // XM's sweep made IT's rate.
        sweep: if speed | depth != 0 { 255 - sweep.min(255) as u8 } else { 0 },
    }
}

/// An envelope of up to ten (tick × 16, value) points.
fn am_envelope(points: &[(u16, i32)], flags: u8, sustain: u8, loop_start: u8, loop_end: u8) -> Envelope {
    let mut env = Envelope::default();
    for (i, &(tick, value)) in points.iter().enumerate() {
        let mut tick = tick >> 4;
        if i == 0 {
            tick = 0;
        } else if let Some(prev) = env.points.last() {
            if tick <= prev.tick {
                tick = prev.tick + 1;
            }
        }
        env.points.push(EnvPoint { tick, value: value as i8 });
    }
    let n = env.points.len() as u8;
    env.enabled = flags & 1 != 0 && n > 0;
    if flags & 2 != 0 && sustain < n {
        env.sustain = Some((sustain, sustain));
    }
    if flags & 4 != 0 && loop_start <= loop_end && loop_end < n {
        env.looped = Some((loop_start, loop_end));
    }
    env
}

fn am_keymap(map: &[u8], first: usize, count: usize) -> Vec<(u8, u16)> {
    (0..120).map(|n| (n as u8, map.get(n).map_or(0, |&s| if usize::from(s) < count { (first + usize::from(s) + 1) as u16 } else { 0 }))).collect()
}

fn load_am(d: &[u8]) -> Result<Module> {
    let am = &d[8..12] == b"AM  ";
    let len = u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize;
    let chunks = riff_chunks(d, 12, 8usize.saturating_add(len), am);
    let main_tag = if am { *b"INIT" } else { *b"MAIN" };
    let &(_, main, main_len) = chunks.iter().find(|c| c.0 == main_tag).ok_or_else(|| Error::Bad("it has no song header".into()))?;
    let h = &d[main..main + main_len];
    if h.len() < 73 {
        return bad("its song header is cut short");
    }
    let title = crate::io::text(&h[..64]);
    let (flags, nch, speed, tempo, gvol) = (h[64], usize::from(h[65]).clamp(1, 64), h[66], h[67], h[72]);
    let channels: Vec<Channel> = (0..nch)
        .map(|c| {
            let p = h.get(73 + c).copied().unwrap_or(if am { 64 } else { 32 });
            let muted = if am { p > 128 } else { p >= 128 };
            let pan = if am { (u16::from(p.min(128)) * 2).min(255) } else { (u16::from(p.min(64)) * 4).min(255) } as u8;
            Channel { pan: if muted { 128 } else { pan }, muted, ..Default::default() }
        })
        .collect();

    let mut orders = Vec::new();
    if let Some(&(_, at, n)) = chunks.iter().find(|c| &c.0 == b"ORDR") {
        let b = &d[at..at + n];
        let count = usize::from(b.first().copied().unwrap_or(0)) + 1;
        for &o in b.iter().skip(1).take(count) {
            orders.push(match o {
                0xFF => ORDER_END,
                0xFE => ORDER_SKIP,
                o => u16::from(o),
            });
        }
    }

    let mut patterns: Vec<Arc<Pattern>> = Vec::new();
    for &(_, at, n) in chunks.iter().filter(|c| &c.0 == b"PATT") {
        let b = &d[at..at + n];
        if b.len() < 5 {
            continue;
        }
        let index = usize::from(b[0]);
        let size = u32::from_le_bytes([b[1], b[2], b[3], b[4]]) as usize;
        let body = &b[5..(5 + size).min(b.len())];
        while patterns.len() <= index {
            patterns.push(Arc::new(Pattern::new(64, nch)));
        }
        patterns[index] = Arc::new(am_pattern(body, nch, am));
    }

    let mut samples = Vec::new();
    let mut instruments: Vec<Instrument> = Vec::new();
    let put = |instruments: &mut Vec<Instrument>, index: usize, ins: Instrument| {
        if index >= 255 {
            return;
        }
        while instruments.len() <= index {
            instruments.push(Instrument::for_sample("", 0));
        }
        instruments[index] = ins;
    };
    if !am {
        for &(_, at, n) in chunks.iter().filter(|c| &c.0 == b"INST") {
            let b = &d[at..at + n];
            if b.len() < 225 {
                continue;
            }
            let index = usize::from(b[1]);
            let name = crate::io::text(&b[2..30]);
            let nsmp = usize::from(b[30]);
            let map = &b[31..151];
            let w = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
            let vib = am_vibrato(b[151], w(152), w(154), w(156));
            // The envelopes: flags, counts, sustains, loop starts and ends
            // (the volume's in the low nibbles, the panning's the high),
            // then ten points each.
            let e = &b[158..223];
            let env = |shift: u8, base: usize, pan: bool| {
                let nib = |v: u8| (v >> shift) & 0x0F;
                let n = usize::from(nib(e[1])).min(10);
                let pts: Vec<(u16, i32)> = (0..n)
                    .map(|i| {
                        let o = base + i * 3;
                        let v = i32::from(e[o + 2].min(64));
                        (u16::from_le_bytes([e[o], e[o + 1]]), if pan { v - 32 } else { v })
                    })
                    .collect();
                am_envelope(&pts, nib(e[0]), nib(e[2]), nib(e[3]), nib(e[4]))
            };
            let volume_env = env(0, 5, false);
            let pan_env = env(4, 35, true);
            let fadeout = u32::from(w(223)) << 5;
            let first = samples.len();
            let mut r = Reader::at(b, 225);
            for _ in 0..nsmp {
                let Ok(sh) = r.bytes(64) else { break };
                if &sh[..4] != b"SAMP" {
                    break;
                }
                let sw = |o: usize| u16::from_le_bytes([sh[o], sh[o + 1]]);
                let sl = |o: usize| u32::from_le_bytes([sh[o], sh[o + 1], sh[o + 2], sh[o + 3]]);
                let sflags = sw(38);
                let frames = sl(40) as usize;
                let b16 = sflags & 4 != 0;
                let fmt = if b16 { Pcm::S16le } else { Pcm::S8 };
                let data = r.bytes_upto(frames * fmt.bytes());
                samples.push(Sample {
                    name: crate::io::text(&sh[8..36]),
                    wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, decode(data, fmt, frames))),
                    c5: sl(52),
                    volume: sh[37].min(64),
                    pan: (sflags & 0x20 != 0).then(|| (u16::from(sh[36]) * 4).min(255) as u8),
                    looped: am_loop(sflags, sl(44), sl(48)),
                    vibrato: vib,
                    ..Sample::empty()
                });
            }
            let count = samples.len() - first;
            put(&mut instruments, index, Instrument { name, keymap: am_keymap(map, first, count), volume_env, pan_env, fadeout, global_volume: 128, ..Default::default() });
        }
    } else {
        for &(_, at, n) in chunks.iter().filter(|c| &c.0 == b"RIFF") {
            let b = &d[at..at + n];
            if b.len() < 4 + 8 + 330 || &b[..4] != b"AI  " || &b[4..8] != b"INST" {
                continue;
            }
            let ih = &b[12..];
            let w = |o: usize| u16::from_le_bytes([ih[o], ih[o + 1]]);
            let index = usize::from(ih[5]);
            let name = crate::io::text(&ih[6..38]);
            let map = &ih[38..166];
            let vib = am_vibrato(ih[166], w(167), w(169), w(171));
            // Three envelopes of 48 bytes from 180: flags, the count less
            // one (FF none), sustain, loop start and end, ten points
            // (tick, value), the fadeout.
            let env = |at: usize, scale: i32, offset: i32, centre: i32| {
                let e = &ih[at..at + 48];
                let count = e[2];
                if count == 0xFF || count == 0 {
                    return Envelope::default();
                }
                let n = (usize::from(count) + 1).min(10);
                let pts: Vec<(u16, i32)> = (0..n)
                    .map(|i| {
                        let o = 6 + i * 4;
                        let v = i32::from(i16::from_le_bytes([e[o + 2], e[o + 3]])) + offset;
                        (u16::from_le_bytes([e[o], e[o + 1]]), ((v + scale / 2) / scale).clamp(0, 64) - centre)
                    })
                    .collect();
                am_envelope(&pts, e[0], e[3], e[4], e[5])
            };
            let volume_env = env(180, 32767 / 64, 0, 0);
            let pitch_env = env(228, 8192 / 64, 4096, 32);
            let pan_env = env(276, 65536 / 64, 32768, 32);
            let fadeout = u32::from(w(180 + 46)) << 5;
            let nsmp = usize::from(w(324));
            let first = samples.len();
            for &(_, sat, sn) in riff_chunks(b, 12 + 330, b.len(), true).iter().filter(|c| &c.0 == b"RIFF").take(nsmp) {
                let s = &b[sat..sat + sn];
                if s.len() < 12 || &s[..4] != b"AS  " || &s[4..8] != b"SAMP" {
                    continue;
                }
                let body = &s[12..12 + (u32::from_le_bytes([s[8], s[9], s[10], s[11]]) as usize).min(s.len() - 12)];
                if body.len() < 60 {
                    continue;
                }
                let sw = |o: usize| u16::from_le_bytes([body[o], body[o + 1]]);
                let sl = |o: usize| u32::from_le_bytes([body[o], body[o + 1], body[o + 2], body[o + 3]]);
                let sflags = sw(40);
                let frames = sl(44) as usize;
                let b16 = sflags & 4 != 0;
                let fmt = if b16 { Pcm::S16le } else { Pcm::S8 };
                let start = (sl(0) as usize).saturating_add(4).min(body.len());
                let data = &body[start..];
                samples.push(Sample {
                    name: crate::io::text(&body[4..36]),
                    wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, decode(data, fmt, frames))),
                    c5: sl(56),
                    volume: (u32::from(sw(38).min(32767)) * 64 / 32767) as u8,
                    pan: (sflags & 0x20 != 0).then(|| (u32::from(sw(36).min(32767)) * 255 / 32767) as u8),
                    looped: am_loop(sflags, sl(48), sl(52)),
                    vibrato: vib,
                    ..Sample::empty()
                });
            }
            let count = samples.len() - first;
            let keymap = if nsmp == 0 { (0..120).map(|n| (n as u8, 0)).collect() } else { am_keymap(map, first, count) };
            put(&mut instruments, index, Instrument { name, keymap, volume_env, pitch_env, pan_env, fadeout, global_volume: 128, ..Default::default() });
        }
    }

    let mut m = Module {
        title,
        format: "am".into(),
        format_name: if am { "Galaxy Sound System (RIFF AM)" } else { "Galaxy Sound System (RIFF AMFF)" }.into(),
        tracker: "Galaxy Sound System".into(),
        kind: Kind::It,
        flags: Flags { linear: flags & 1 == 0, instruments: true, old_effects: true, compat_gxx: true, ..Default::default() },
        speed,
        tempo: u16::from(tempo),
        global_volume: (u16::from(gvol) * 2).min(128) as u8,
        mix_volume: 48,
        separation: 128,
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

fn am_loop(flags: u16, start: u32, end: u32) -> Loop {
    let kind = if flags & 8 == 0 {
        LoopKind::Off
    } else if flags & 0x10 != 0 {
        LoopKind::PingPong
    } else {
        LoopKind::Forward
    };
    Loop { kind, start, end }
}

// ---- DigiTrekker ----
//
// DigiTrekker 3's modules (Horst Beham's own documentation): IFF-like
// chunks, little-endian, all inside a SONG chunk. NAME; INFO, the
// counts (channels, orders, patterns, tracks, instruments); INIT, its
// sped (speed and tempo) and vpan (each channel's left and right volume)
// chunks; PSEQ, the orders; PATT, each pattern's tracks, one a channel;
// INST, a samp (name, length, loop, the rate of C-4, volume, bits,
// file name) or text chunk an instrument; TRAK, a trak chunk a track
// (its rows, then its notes, instruments, volumes, effects and the
// parameters' high and low bytes, a column at a time); SAMP, the
// samples. Its C-4 is the note a sample plays at its own rate (our C-5);
// its slides are in 128ths of a semitone, linear.

pub fn probe_dtm_dt(d: &[u8]) -> bool {
    d.len() >= 26 && &d[..4] == b"SONG" && (&d[8..12] == b"NAME" || &d[8..12] == b"INFO")
}

/// A DigiTrekker effect (one less than it's written) and its 16-bit
/// parameter, read into the cell.
fn dt_effect(cell: &mut Cell, fx: u8, hi: u8, lo: u8) {
    let p16 = u16::from(hi) << 8 | u16::from(lo);
    // 128ths of a semitone a frame made the model's 16ths.
    let slide = |v: u16| (v / 8).clamp(1, 0xDF) as u8;
    let fine = |up: bool, v: u8| {
        let (f, x) = if v >= 8 { (if up { Fx::PortaUp } else { Fx::PortaDown }, 0xF0 | (v / 8).min(15)) } else { (if up { Fx::PortaUp } else { Fx::PortaDown }, 0xE0 | (v / 2).clamp(1, 15)) };
        (f, x)
    };
    let volslide = |hi: u8, lo: u8| -> (Fx, u8) {
        match (hi, lo) {
            (0xFF, l) => (Fx::VolSlide, 0xF0 | l.clamp(1, 15)),
            (h, 0xFF) => (Fx::VolSlide, h.clamp(1, 15) << 4 | 0x0F),
            (h, _) if h != 0 => (Fx::VolSlide, h.min(15) << 4),
            (_, l) if l != 0 => (Fx::VolSlide, l.min(15)),
            _ => (Fx::VolSlide, 0),
        }
    };
    let (f, p) = match fx {
        0x01 if hi == 0xFF => fine(true, lo),
        0x02 if lo == 0xFF => fine(false, hi),
        0x01 => (Fx::PortaUp, if p16 == 0 { 0 } else { slide(p16) }),
        0x02 => (Fx::PortaDown, if p16 == 0 { 0 } else { slide(p16) }),
        0x03 => (Fx::TonePorta, if p16 == 0 { 0 } else { slide(p16) }),
        0x04 => (Fx::Vibrato, hi.min(15) << 4 | (lo / 16).min(15)),
        0x05 => (Fx::TonePortaVol, volslide(hi, lo).1),
        0x06 => (Fx::VibratoVol, volslide(hi, lo).1),
        0x07 => (Fx::Tremolo, hi.min(15) << 4 | (lo / 4).min(15)),
        // In 16 frames, as 256.
        0x09 => (Fx::Offset, (p16 / 16).min(255) as u8),
        0x0A => volslide(hi, lo),
        0x0B => (Fx::PositionJump, lo),
        0x0D => (Fx::PatternBreak, lo),
        0x0E => mod_ex_to_s3m((hi & 0x0F) << 4 | (lo & 0x0F)),
        // Frames a row, and frames a second (as a tempo, × 2.5): one
        // effect holds only one of them, the speed first.
        0x0F if hi != 0 => (Fx::Speed, hi),
        0x0F if lo != 0 => (Fx::Tempo, (u16::from(lo) * 5 / 2).clamp(32, 255) as u8),
        0x10 if p16 >= 32 => (Fx::Tempo, p16.min(255) as u8),
        0x11 => (Fx::Panning, dt_pan(hi, lo)),
        0x12 => {
            // Signed steps of the left and right volumes.
            let toward_right = i32::from(lo as i8) - i32::from(hi as i8);
            match toward_right {
                0 => (Fx::None, 0),
                t if t > 0 => (Fx::PanningSlide, t.min(15) as u8),
                t => (Fx::PanningSlide, ((-t).min(15) as u8) << 4),
            }
        }
        0x15 => {
            // dfxy: after d frames, the volume kept, lessened, raised or scaled.
            let delay = (hi >> 4).max(1);
            let amount = |v: u8| match v {
                0 => 0,
                1 => 1,
                2 => 2,
                3..=4 => 3,
                5..=8 => 4,
                _ => 5,
            };
            let x = match hi & 0x0F {
                1 => amount(lo),
                2 => 8 + amount(lo),
                3 if lo >> 4 == 2 && lo & 0x0F == 3 => 6,
                3 if lo >> 4 == 1 && lo & 0x0F == 2 => 7,
                3 if lo >> 4 == 3 && lo & 0x0F == 2 => 14,
                3 if lo >> 4 == 2 && lo & 0x0F == 1 => 15,
                _ => 0,
            };
            (Fx::Retrig, x << 4 | delay.min(15))
        }
        0x16 if hi | lo != 0 => (Fx::Tremor, hi.clamp(1, 15) << 4 | lo.clamp(1, 15)),
        _ => (Fx::None, 0),
    };
    if f != Fx::None {
        (cell.fx, cell.param) = (f, p);
    }
}

/// A left and a right volume (0 to 64 each) as a panning.
fn dt_pan(left: u8, right: u8) -> u8 {
    let (l, r) = (u32::from(left.min(64)), u32::from(right.min(64)));
    if l + r == 0 {
        128
    } else {
        (r * 255 / (l + r)) as u8
    }
}

pub fn load_dtm_dt(d: &[u8]) -> Result<Module> {
    let song_len = u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize;
    let chunks = riff_chunks(d, 8, 8usize.saturating_add(song_len), false);
    let find = |tag: &[u8; 4]| chunks.iter().find(|c| &c.0 == tag).map(|&(_, a, n)| &d[a..a + n]);
    let info = find(b"INFO").ok_or_else(|| Error::Bad("it has no INFO chunk".into()))?;
    if info.len() < 10 {
        return bad("its INFO chunk is cut short");
    }
    let w = |b: &[u8], o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let nch = usize::from(w(info, 0)).clamp(1, 64);
    let (nord, npat, ntrk, nins) = (usize::from(w(info, 2)), usize::from(w(info, 4)), usize::from(w(info, 6)), usize::from(w(info, 8)));
    let title = find(b"NAME").map(crate::io::text).unwrap_or_default();

    let (mut speed, mut tempo) = (6u8, 125u16);
    let mut channels: Vec<Channel> = (0..nch).map(|_| Channel::default()).collect();
    if let Some(&(_, at, n)) = chunks.iter().find(|c| &c.0 == b"INIT") {
        for (tag, a, len) in riff_chunks(d, at, at + n, false) {
            let b = &d[a..a + len];
            match &tag {
                // Two 16-bit words; the demo song's (an older version's)
                // two bytes.
                b"sped" if b.len() >= 4 && b[1] == 0 => {
                    speed = w(b, 0).clamp(1, 255) as u8;
                    tempo = w(b, 2).clamp(32, 999);
                }
                b"sped" if b.len() >= 2 => {
                    speed = b[0].max(1);
                    tempo = u16::from(b[1]).max(32);
                }
                // The left and right volumes add up to the channel's.
                b"vpan" => {
                    for (c, ch) in channels.iter_mut().enumerate() {
                        if let (Some(&l), Some(&r)) = (b.get(c * 2), b.get(c * 2 + 1)) {
                            ch.pan = dt_pan(l, r);
                            ch.volume = (u16::from(l) + u16::from(r)).min(64) as u8;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let orders: Vec<u16> = find(b"PSEQ").unwrap_or(&[]).iter().take(nord).map(|&o| u16::from(o)).collect();

    // The tracks, each a column of cells.
    let mut tracks: Vec<Vec<Cell>> = Vec::with_capacity(ntrk);
    if let Some(&(_, at, n)) = chunks.iter().find(|c| &c.0 == b"TRAK") {
        for (tag, a, len) in riff_chunks(d, at, at + n, false) {
            if &tag != b"trak" {
                continue;
            }
            let b = &d[a..a + len];
            if b.len() < 2 {
                tracks.push(Vec::new());
                continue;
            }
            let rows = usize::from(w(b, 0)).min(1024);
            let col = |k: usize, r: usize| b.get(2 + k * rows + r).copied().unwrap_or(0);
            let mut cells = vec![Cell::default(); rows];
            for (r, cell) in cells.iter_mut().enumerate() {
                cell.note = match col(0, r) {
                    0 => NOTE_NONE,
                    0x80 => NOTE_OFF,
                    p if p <= 96 => p + 12,
                    _ => NOTE_NONE,
                };
                cell.instr = col(1, r);
                if let v @ 1..=65 = col(2, r) {
                    (cell.vol, cell.volp) = (Vol::Volume, v - 1);
                }
                if let fx @ 1.. = col(3, r) {
                    dt_effect(cell, fx - 1, col(4, r), col(5, r));
                }
            }
            tracks.push(cells);
        }
    }
    let patt = find(b"PATT").unwrap_or(&[]);
    // No more patterns than the PATT chunk names tracks for.
    let npat = npat.min(patt.len() / (nch * 2));
    let mut patterns = Vec::with_capacity(npat);
    for p in 0..npat {
        let refs: Vec<Option<&Vec<Cell>>> = (0..nch).map(|c| patt.get(p * nch * 2 + c * 2..p * nch * 2 + c * 2 + 2).and_then(|b| tracks.get(usize::from(u16::from_le_bytes([b[0], b[1]]))))).collect();
        let rows = refs.iter().flatten().map(|t| t.len()).max().unwrap_or(64).clamp(1, 1024);
        let mut pat = Pattern::new(rows, nch);
        for (c, t) in refs.iter().enumerate() {
            for (r, cell) in t.iter().flat_map(|t| t.iter()).enumerate() {
                if let Some(x) = pat.cell_mut(r, c) {
                    *x = *cell;
                }
            }
        }
        patterns.push(Arc::new(pat));
    }

    // Instruments (samples or lines of text), then the samples' data in
    // the same order.
    let mut samples = Vec::with_capacity(nins);
    let mut bits = Vec::with_capacity(nins);
    if let Some(&(_, at, n)) = chunks.iter().find(|c| &c.0 == b"INST") {
        for (tag, a, len) in riff_chunks(d, at, at + n, false) {
            let b = &d[a..a + len];
            match &tag {
                b"samp" if b.len() >= 48 => {
                    let l = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
                    let b16 = b[47] == 16;
                    let div = if b16 { 2 } else { 1 };
                    let (ls, le) = (l(36) / div, l(40) / div);
                    samples.push(Sample {
                        name: crate::io::text(&b[..32]),
                        filename: b.get(48..61).map(crate::io::text).unwrap_or_default(),
                        c5: u32::from(w(b, 44)),
                        volume: b[46].min(64),
                        looped: if le > ls { Loop::forward(ls, le) } else { Loop::default() },
                        ..Sample::empty()
                    });
                    bits.push(Some(b16));
                }
                _ => {
                    samples.push(Sample { name: crate::io::text(b), ..Sample::empty() });
                    bits.push(None);
                }
            }
        }
    }
    if let Some(&(_, at, n)) = chunks.iter().find(|c| &c.0 == b"SAMP") {
        let mut k = 0;
        for (tag, a, len) in riff_chunks(d, at, at + n, false) {
            // Each instrument that's a sample takes the next samp chunk.
            while k < bits.len() && bits[k].is_none() {
                k += 1;
            }
            if k >= samples.len() {
                break;
            }
            if &tag == b"samp" {
                let fmt = if bits[k] == Some(true) { Pcm::S16le } else { Pcm::S8 };
                let b = &d[a..a + len];
                samples[k].wave = Arc::new(Wave::new(if fmt.is_16() { 16 } else { 8 }, 1, decode(b, fmt, b.len() / fmt.bytes())));
            }
            k += 1;
        }
    }

    let mut m = Module {
        title,
        format: "dtm".into(),
        format_name: "DigiTrekker".into(),
        tracker: "DigiTrekker 3".into(),
        kind: Kind::It,
        flags: Flags { linear: true, ..Default::default() },
        speed,
        tempo,
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

// ---- PlayerPro (MADF, MADG) ----
//
// PlayerPro's first module formats on the Mac (from its released
// source, in the public domain): "MADF" or "MADG", the title, 64
// sample headers (file name, length, finetune, volume, packing, bits,
// loop), the pattern count, the orders and the channels; each pattern
// (MADG gives each a header: rows, "NONE" or "MAD1" packing, a name,
// its length) a channel at a time, four bytes a cell (instrument, note,
// MOD's command and parameter; MAD1 drops what's empty, a flag byte
// saying what's kept); then the samples, signed, 16-bit ones
// big-endian. Its effects are MOD's.

pub fn probe_madfg(d: &[u8]) -> bool {
    d.len() >= MADFG_HEADER && (&d[..4] == b"MADG" || &d[..4] == b"MADF") && (1..=32).contains(&d[MADFG_HEADER - 2]) && d[3364] > 0 && d[3365] > 0 && d[3365] <= 128
}

/// The header: mark, title, 64 sample headers of 52 bytes, the pattern
/// count, the order count, 128 orders, the channels, a pad.
const MADFG_HEADER: usize = 4 + 32 + 64 * 52 + 1 + 1 + 128 + 1 + 1;

pub fn load_madfg(d: &[u8]) -> Result<Module> {
    const FINETUNE: [u32; 16] = [8363, 8413, 8463, 8529, 8581, 8651, 8723, 8757, 7895, 7941, 7985, 8046, 8107, 8169, 8232, 8280];
    let g = &d[..4] == b"MADG";
    let title = crate::io::text(&d[4..36]);
    let (npat, nord, nch) = (usize::from(d[3364]), usize::from(d[3365]), usize::from(d[3494]));
    let orders: Vec<u16> = d[3366..3366 + nord].iter().map(|&o| u16::from(o)).collect();
    let mut r = Reader::at(d, MADFG_HEADER);
    let mut patterns = Vec::with_capacity(npat);
    for _ in 0..npat {
        let (rows, packed, name, bytes) = if g {
            let rows = r.u32be()? as usize;
            let mode = r.tag()?;
            let name = r.text(20)?;
            let bytes = r.u32be()? as usize;
            r.u32be()?;
            (rows.clamp(1, 256), &mode == b"MAD1", name, bytes)
        } else {
            (64, false, String::new(), 0)
        };
        let mut cells = vec![[0u8; 4]; rows * nch];
        if packed {
            let b = r.bytes_upto(bytes);
            let mut p = Reader::new(b);
            for c in cells.iter_mut() {
                let Ok(flag) = p.u8() else { break };
                match flag {
                    3 => c.copy_from_slice(p.bytes(4)?),
                    2 => c[..2].copy_from_slice(p.bytes(2)?),
                    1 => c[2..].copy_from_slice(p.bytes(2)?),
                    _ => {}
                }
            }
        } else {
            let b = r.bytes_upto(rows * nch * 4);
            for (c, x) in cells.iter_mut().zip(b.chunks_exact(4)) {
                c.copy_from_slice(x);
            }
        }
        let mut pat = Pattern::new(rows, nch);
        pat.name = name;
        // A channel at a time.
        for ch in 0..nch {
            for row in 0..rows {
                let [ins, note, cmd, arg] = cells[ch * rows + row];
                let mut cell = Cell { instr: ins, ..Default::default() };
                if note != 0 {
                    cell.note = (u16::from(note) + 35).min(u16::from(NOTE_MAX)) as u8;
                }
                if cmd == 0x0C {
                    (cell.vol, cell.volp) = (Vol::Volume, arg.min(64));
                } else {
                    (cell.fx, cell.param) = super::common::mod_effect(cmd, arg, false);
                }
                if let Some(x) = pat.cell_mut(row, ch) {
                    *x = cell;
                }
            }
        }
        patterns.push(Arc::new(pat));
    }
    let mut samples = Vec::with_capacity(64);
    for i in 0..64 {
        let h = &d[36 + i * 52..36 + i * 52 + 52];
        let l = |o: usize| u32::from_be_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]);
        let size = l(32) as usize;
        let b16 = h[42] == 16;
        let fmt = if b16 { Pcm::S16be } else { Pcm::S8 };
        let data = r.bytes_upto(size);
        let div = if b16 { 2 } else { 1 };
        let (ls, ll) = (l(44) / div, l(48) / div);
        samples.push(Sample {
            name: crate::io::text(&h[..32]),
            wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, 1, decode(data, fmt, size / fmt.bytes()))),
            c5: FINETUNE[usize::from(h[36] & 15)],
            volume: h[37].min(64),
            looped: if ll > 2 { Loop::forward(ls, ls + ll) } else { Loop::default() },
            ..Sample::empty()
        });
    }
    while samples.last().is_some_and(|s| s.wave.is_empty() && s.name.is_empty()) {
        samples.pop();
    }
    let mut m = Module {
        title,
        format: "mad".into(),
        format_name: "PlayerPro".into(),
        tracker: if g { "PlayerPro (MADG)" } else { "PlayerPro (MADF)" }.into(),
        kind: Kind::Mod,
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels: (0..nch.max(1)).map(|c| Channel { pan: if c % 2 == 0 { 64 } else { 192 }, ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- PlayerPro (MADH, MADI, MADK) ----
//
// PlayerPro's later formats (from its released source, in the public
// domain), MADK its own from 5.0, MADI and MADH the two before. Each is
// a header (MADH's holds 64 instruments), the patterns, the instruments
// (MADI's and MADK's, as many as are used, each saying its number), then
// each instrument's samples in instrument order, each a header and its
// data; MADK ends in its effect plug-ins' settings, not read here. Every
// number is big-endian, the structures packed to two bytes.
//
// A pattern is a header (rows, "NONE" or "MAD1" packing, a name, its
// length) then a channel at a time, six bytes a cell: instrument, note
// (0 is XM's C-0, FE key off, FF none), command, argument, XM's volume
// column (FF none), a spare. MAD1 drops what's empty, a flag byte a cell
// saying what's kept. The commands are XM's 0 to F, 7 (tremolo) not
// played, then G (note off in multichannel mode), L (move the loop) and
// O (offset as a share of the sample), not kept. Instruments are XM's:
// a sample a note, volume and panning envelopes, a fadeout. A sample's
// rate is for XM's C-4, its relative note added.

pub fn probe_madk(d: &[u8]) -> bool {
    let Some(v) = MadVersion::of(d) else { return false };
    d.len() >= v.header() && d[293] > 0 && d[293] <= 99 && d[292] > 0 && d[294] > 0
}

#[derive(Clone, Copy, PartialEq)]
enum MadVersion {
    H,
    I,
    K,
}

impl MadVersion {
    fn of(d: &[u8]) -> Option<MadVersion> {
        match d.get(..4)? {
            b"MADH" => Some(MadVersion::H),
            b"MADI" => Some(MadVersion::I),
            b"MADK" => Some(MadVersion::K),
            _ => None,
        }
    }

    fn header(self) -> usize {
        match self {
            MadVersion::H => 16044,
            MadVersion::I => 1812,
            MadVersion::K => 7486,
        }
    }

    /// An instrument's length and where its fields are: its number (MADH
    /// has none), sample count, keymap, envelopes, envelope point counts,
    /// volume and panning sustain-loop triples, envelope flags, fadeout.
    fn instrument(self) -> [usize; 11] {
        match self {
            MadVersion::H => [242, 0, 34, 36, 132, 180, 228, 230, 233, 236, 238],
            MadVersion::I => [244, 33, 36, 38, 134, 182, 230, 232, 235, 238, 240],
            MadVersion::K => [300, 33, 36, 42, 138, 186, 282, 285, 288, 294, 296],
        }
    }
}

pub fn load_madk(d: &[u8]) -> Result<Module> {
    let Some(v) = MadVersion::of(d) else { return bad("not a PlayerPro module") };
    let be16 = |b: &[u8], o: usize| u16::from_be_bytes([b[o], b[o + 1]]);
    let be32 = |b: &[u8], o: usize| u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let title = crate::io::text(&d[4..36]);
    let (npat, nch, nord) = (usize::from(d[292]), usize::from(d[293]), usize::from(d[294]));
    let (ord_at, speed_at) = if v == MadVersion::H { (295, 552) } else { (297, 1296) };
    let orders: Vec<u16> = d[ord_at..ord_at + nord.min(256)].iter().map(|&o| u16::from(o)).collect();
    let (speed, tempo) = (be16(d, speed_at), be16(d, speed_at + 2));
    let mut r = Reader::at(d, v.header());

    let mut patterns = Vec::with_capacity(npat);
    for _ in 0..npat {
        let rows = (r.u32be()? as usize).clamp(1, 999);
        let mode = r.tag()?;
        let name = r.text(32)?;
        let bytes = r.u32be()? as usize;
        r.u32be()?;
        let mut cells = vec![[0u8, 0xFF, 0, 0, 0xFF]; rows * nch];
        if &mode == b"MAD1" {
            let mut p = Reader::new(r.bytes_upto(bytes));
            'cells: for c in cells.iter_mut() {
                let Ok(flag) = p.u8() else { break };
                for (bit, x) in c.iter_mut().enumerate() {
                    if flag & 1 << bit != 0 {
                        let Ok(b) = p.u8() else { break 'cells };
                        *x = b;
                    }
                }
            }
        } else {
            let b = r.bytes_upto(rows * nch * 6);
            for (c, x) in cells.iter_mut().zip(b.chunks_exact(6)) {
                c.copy_from_slice(&x[..5]);
            }
        }
        let mut pat = Pattern::new(rows, nch);
        pat.name = name;
        // A channel at a time.
        for ch in 0..nch {
            for row in 0..rows {
                let [ins, note, cmd, arg, vol] = cells[ch * rows + row];
                let mut cell = Cell { instr: ins, ..Default::default() };
                cell.note = match note {
                    0xFE => NOTE_OFF,
                    0..=95 => note + 13,
                    _ => 0,
                };
                if vol != 0xFF {
                    (cell.vol, cell.volp) = super::xm::volume_column(vol);
                }
                if cmd <= 0x0F && cmd != 7 {
                    (cell.fx, cell.param) = super::common::mod_effect(cmd, arg, true);
                }
                if let Some(x) = pat.cell_mut(row, ch) {
                    *x = cell;
                }
            }
        }
        patterns.push(Arc::new(pat));
    }

    // The instruments, by number.
    let [isize, no, ns, what, venv, penv, npts, vsus, psus, flags, fade] = v.instrument();
    let records: Vec<&[u8]> = match v {
        MadVersion::H => (0..64).map(|i| &d[556 + i * isize..556 + (i + 1) * isize]).collect(),
        _ => (0..usize::from(d[295]).min(255)).map(|_| r.bytes(isize)).collect::<Result<_>>()?,
    };
    let mut slots: Vec<Option<&[u8]>> = vec![None; 255];
    for (i, h) in records.iter().enumerate() {
        let n = if v == MadVersion::H { i } else { usize::from(h[no]) };
        if let Some(s) = slots.get_mut(n) {
            *s = Some(h);
        }
    }
    let used = slots.iter().rposition(Option::is_some).map_or(0, |n| n + 1);
    let mut instruments = Vec::with_capacity(used);
    let mut samples = Vec::new();
    for h in &slots[..used] {
        let Some(h) = *h else {
            instruments.push(Instrument { global_volume: 128, keymap: (0..120).map(|n| (n as u8, 0)).collect(), ..Default::default() });
            continue;
        };
        let count = usize::from(be16(h, ns)).min(64);
        let first = samples.len();
        for _ in 0..count {
            let s = r.bytes(56)?;
            let size = be32(s, 0) as usize;
            let (ls, ll) = (be32(s, 4), be32(s, 8));
            let c2spd = be16(s, 14);
            let (b16, stereo) = (s[17] == 16, v != MadVersion::H && s[51] != 0);
            let (relative, name) = if v == MadVersion::H { (s[19] as i8, &s[20..52]) } else { (s[18] as i8, &s[19..51]) };
            let fmt = if b16 { Pcm::S16be } else { Pcm::S8 };
            let data = r.bytes_upto(size);
            let ch: u8 = if stereo { 2 } else { 1 };
            let div = fmt.bytes() as u32 * u32::from(ch);
            let kind = if s[16] == 1 { LoopKind::PingPong } else { LoopKind::Forward };
            let rate = f64::from(if c2spd == 0 { 8363 } else { c2spd }) * 2f64.powf(f64::from(relative) / 12.0);
            samples.push(Sample {
                name: crate::io::text(name),
                wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, ch, decode(data, fmt, size / fmt.bytes()))),
                c5: rate.round().clamp(1.0, 2_000_000.0) as u32,
                volume: s[12].min(64),
                global_volume: 64,
                looped: if ll > 2 { Loop { kind, start: ls / div, end: (ls + ll) / div } } else { Loop::default() },
                ..Sample::empty()
            });
        }
        let points = |at: usize| -> Vec<(u16, u16)> { (0..12).map(|i| (be16(h, at + i * 4), be16(h, at + i * 4 + 2))).collect() };
        let keys = &h[what..what + 96];
        instruments.push(Instrument {
            name: crate::io::text(&h[..32]),
            global_volume: 128,
            volume_env: super::xm::envelope(&points(venv), h[npts].min(12), h[flags], h[vsus], h[vsus + 1], h[vsus + 2], false),
            pan_env: super::xm::envelope(&points(penv), h[npts + 1].min(12), h[flags + 1], h[psus], h[psus + 1], h[psus + 2], true),
            fadeout: u32::from(be16(h, fade)) * 2,
            // PlayerPro's 96 notes sit an octave up here, as XM's do.
            keymap: (0..120)
                .map(|n| {
                    let k = n as i32 - 12;
                    let s = if (0..96).contains(&k) { usize::from(keys[k as usize]) } else { count };
                    (n as u8, if s < count { (first + s + 1) as u16 } else { 0 })
                })
                .collect(),
            ..Default::default()
        });
    }

    let channels = (0..nch)
        .map(|c| match v {
            MadVersion::H => Channel { pan: if c % 2 == 0 { 64 } else { 192 }, ..Default::default() },
            _ => Channel { pan: (u16::from(d[1300 + c].min(64)) * 4).min(255) as u8, volume: d[1556 + c].min(64), ..Default::default() },
        })
        .collect();
    let mut m = Module {
        title,
        format: "mad".into(),
        format_name: "PlayerPro".into(),
        tracker: match v {
            MadVersion::H => "PlayerPro (MADH)",
            MadVersion::I => "PlayerPro (MADI)",
            MadVersion::K => "PlayerPro (MADK)",
        }
        .into(),
        message: if v == MadVersion::H { String::new() } else { crate::io::message(&d[36..275]) },
        kind: Kind::Xm,
        flags: Flags { linear: v != MadVersion::H && d[286] != 0, instruments: true, ..Default::default() },
        speed: speed.clamp(1, 255) as u8,
        tempo: tempo.clamp(32, 999),
        global_volume: if v == MadVersion::H || d[291] == 0 { 128 } else { d[291].min(64) * 2 },
        mix_volume: 32,
        separation: 128,
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

// ---- Ken's Digital Music ----
//
// Ken Silverman's music (KDM: Ken's own engine, KDMENG, describes it):
// not rows but notes in time, 120 ticks a second. A version (0), the
// notes and tracks; each track's wave, quantize, and left and right
// volumes; then the notes a column at a time: when, which track, the
// pitch (a semitone a step), the left and right volumes (0 both: the
// note off), and a pitch, a volume and (in later songs) a panning
// effect, each one of eight curves run for 256 ticks. The waves are in WAVES.KWV beside it,
// shared by every song: a name, length, loop and finetune each, then
// unsigned 8-bit data. Here the ticks become rows at the tracks'
// quantize (with note delays for what falls between), the notes share
// 16 channels as Ken's engine's 16 voices did (a new note taking the
// voice nearest its end when all are sounding), the fades are
// written out row by row, the vibratos and arpeggio become ticked
// effects; the short pitch scoops and the panning curves aren't kept.

pub fn probe_kdm(d: &[u8]) -> bool {
    if d.len() < 12 || d[..4] != [0; 4] {
        return false;
    }
    let notes = u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize;
    let tracks = u32::from_le_bytes([d[8], d[9], d[10], d[11]]) as usize;
    // Older songs have no panning effects: ten bytes a note, not eleven.
    notes > 0 && notes <= 8192 && tracks > 0 && tracks <= 256 && (12 + tracks * 4 + notes * 11 == d.len() || 12 + tracks * 4 + notes * 10 == d.len())
}

pub fn load_kdm(d: &[u8], companion: super::Companion) -> Result<Module> {
    let notes = u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize;
    let tracks = u32::from_le_bytes([d[8], d[9], d[10], d[11]]) as usize;
    let col = |k: usize| &d[12 + k * tracks..12 + (k + 1) * tracks];
    let (tr_wave, tr_quant) = (col(0), col(1));
    let at = 12 + tracks * 4;
    let time: Vec<u32> = d[at..at + notes * 4].chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let nc = |k: usize| &d[at + notes * 4 + k * notes..at + notes * 4 + (k + 1) * notes];
    let (n_track, n_freq, n_vol1, n_vol2, n_frq, n_vol) = (nc(0), nc(1), nc(2), nc(3), nc(4), nc(5));

    // The waves.
    let kwv = companion("waves.kwv").ok_or_else(|| Error::Bad("its sounds are in WAVES.KWV, which isn't beside it".into()))?;
    if kwv.len() < 8 || kwv[..4] != [0; 4] {
        return bad("WAVES.KWV beside it is damaged");
    }
    let nwaves = (u32::from_le_bytes([kwv[4], kwv[5], kwv[6], kwv[7]]) as usize).min(256);
    let mut heads = Vec::with_capacity(nwaves);
    let mut data_at = 8 + nwaves * 32;
    for i in 0..nwaves {
        let h = kwv.get(8 + i * 32..8 + i * 32 + 32).ok_or_else(|| Error::Bad("WAVES.KWV beside it is cut short".into()))?;
        let l = |o: usize| i32::from_le_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]);
        let len = l(16).max(0) as usize;
        heads.push((crate::io::text(&h[..16]), data_at, len, l(20).max(0) as u32, l(24).max(0) as u32, l(28)));
        data_at = data_at.saturating_add(len);
    }
    // A sample for each wave the tracks use.
    let mut sample_of = vec![0u8; 256];
    let mut samples = Vec::new();
    for &w in tr_wave {
        let w = usize::from(w);
        if w >= heads.len() || sample_of[w] != 0 || samples.len() >= 255 {
            continue;
        }
        let (name, from, len, rep_start, rep_len, finetune) = &heads[w];
        let b = kwv.get(*from..(from + len).min(kwv.len())).unwrap_or(&[]);
        // Ken's rate: a step of the table at 11025 × 2093 / 2^23 by (finetune + 748), two octaves up for C-5.
        let c5 = (11025.0 * 2093.0 * f64::from(finetune + 748) * 4.0 / 8_388_608.0).round().clamp(1.0, 2_000_000.0) as u32;
        samples.push(Sample {
            name: name.clone(),
            wave: Arc::new(Wave::new(8, 1, decode(b, Pcm::U8, b.len()))),
            c5,
            looped: if *rep_len > 0 { Loop::forward(*rep_start, rep_start + rep_len) } else { Loop::default() },
            ..Sample::empty()
        });
        sample_of[w] = samples.len() as u8;
    }

    // 120 of Ken's ticks a second are 80 of ours (tempo 200); a row is
    // the coarsest quantize's step.
    let quant = tr_quant.iter().copied().filter(|&q| q > 0).max().unwrap_or(8);
    let speed = (80 / u32::from(quant)).clamp(1, 32) as u8;
    let t0 = time.first().copied().unwrap_or(0);
    let tick_of = |t: u32| ((u64::from(t.saturating_sub(t0)) * 2 + 1) / 3) as usize;
    let total_rows = (tick_of(time.last().copied().unwrap_or(t0)) / usize::from(speed) + 2).min(64 * 1024);

    // Channels: each track's, as many as it needs. (track, busy until, wave, pitch).
    struct Voice {
        track: u8,
        until: usize,
        pitch: u8,
    }
    let mut voices: Vec<Voice> = Vec::new();
    let mut cells: Vec<Vec<Cell>> = Vec::new();
    // The last row each channel's note wrote its lasting effects to.
    let mut lasting: Vec<usize> = Vec::new();
    let put = |cells: &mut Vec<Vec<Cell>>, ch: usize, row: usize, f: &dyn Fn(&mut Cell)| {
        if let Some(c) = cells.get_mut(ch).and_then(|c| c.get_mut(row)) {
            f(c);
        }
    };
    for i in 0..notes {
        let track = n_track[i];
        let Some(&w) = tr_wave.get(usize::from(track)) else { continue };
        let smp = sample_of[usize::from(w)];
        let tick = tick_of(time[i]);
        let (row, delay) = (tick / usize::from(speed), (tick % usize::from(speed)).min(15) as u8);
        if row >= total_rows {
            continue;
        }
        let pitch = n_freq[i];
        let (v1, v2) = (u32::from(n_vol1[i]), u32::from(n_vol2[i]));
        if v1 == 0 {
            // A note off: the track's voice playing this pitch stops.
            if let Some(ch) = voices.iter().position(|v| v.track == track && v.pitch == pitch && v.until > tick) {
                voices[ch].until = tick;
                put(&mut cells, ch, row, &|c: &mut Cell| {
                    if c.note == NOTE_NONE {
                        c.note = NOTE_CUT;
                        if delay > 0 && c.fx == Fx::None {
                            (c.fx, c.param) = (Fx::S3mEx, 0xD0 | delay);
                        }
                    }
                });
            }
            continue;
        }
        // A free voice (the track's own first), a new one up to 16, or
        // the one nearest its end.
        let free = |v: &usize| voices[*v].until <= tick && cells[*v][row].note == NOTE_NONE;
        let ch = match (0..voices.len()).filter(free).find(|&v| voices[v].track == track).or_else(|| (0..voices.len()).find(free)) {
            Some(ch) => ch,
            None if voices.len() < 16 => {
                voices.push(Voice { track, until: 0, pitch });
                cells.push(vec![Cell::default(); total_rows]);
                lasting.push(0);
                voices.len() - 1
            }
            None => match (0..voices.len()).filter(|&v| cells[v][row].note == NOTE_NONE).min_by_key(|&v| voices[v].until) {
                Some(ch) => ch,
                None => continue,
            },
        };
        // How long it sounds: to its end if it plays once, or until a note off.
        let s = samples.get(usize::from(smp).wrapping_sub(1));
        let ticks = match s {
            Some(s) if !s.looped.on() => {
                let rate = f64::from(s.c5) * 2f64.powf((f64::from(pitch) - 25.0) / 12.0);
                (s.frames() as f64 / rate.max(1.0) * 80.0).ceil() as usize
            }
            _ => usize::MAX / 4,
        };
        voices[ch] = Voice { track, until: tick + ticks.max(1), pitch };
        let volume = ((v1 + v2) / 4).min(64) as u8;
        let pan = ((128 + (v2 as i32 - v1 as i32) * 4).clamp(0, 255)) as u8;
        let note = (u16::from(pitch) + 36).clamp(u16::from(NOTE_MIN), u16::from(NOTE_MAX)) as u8;
        let frq = n_frq[i];
        let vol_fx = n_vol[i];
        cells[ch][row] = Cell {
            note,
            instr: smp,
            vol: Vol::Volume,
            volp: if vol_fx == 7 { 0 } else { volume },
            fx: if delay > 0 { Fx::S3mEx } else { Fx::Panning },
            param: if delay > 0 { 0xD0 | delay } else { pan },
        };
        // The pitch effects that become ticked ones.
        let ticked = match frq {
            1 => Some((Fx::Vibrato, 0x44)),
            4 => Some((Fx::Vibrato, 0x1F)),
            5 => Some((Fx::Vibrato, 0x3F)),
            6 => Some((Fx::Arpeggio, 0x47)),
            _ => None,
        };
        // What the note before wrote past here is its no longer.
        for r in row + 1..=lasting[ch].min(total_rows - 1) {
            let c = &mut cells[ch][r];
            if c.note == NOTE_NONE {
                (c.vol, c.volp) = (Vol::None, 0);
            }
            if matches!(c.fx, Fx::Vibrato | Fx::Arpeggio) {
                (c.fx, c.param) = (Fx::None, 0);
            }
        }
        lasting[ch] = row;
        if let Some((fx, p)) = ticked {
            if delay == 0 {
                (cells[ch][row].fx, cells[ch][row].param) = (fx, p);
            }
        }
        // The rows after: the fade or tremolo, the vibrato carried on,
        // for the 256 ticks the curves run (to the voice's next note).
        let rows_on = (256 * 2 / 3usize).div_ceil(usize::from(speed));
        for k in 1..=rows_on {
            let r = row + k;
            if r >= total_rows || cells[ch][r].note != NOTE_NONE {
                break;
            }
            let t = (k * usize::from(speed)) as f64 * 1.5;
            let c = &mut cells[ch][r];
            match vol_fx {
                7 => (c.vol, c.volp) = (Vol::Volume, (f64::from(volume) * (t / 120.0).min(1.0)).round() as u8),
                8 => (c.vol, c.volp) = (Vol::Volume, (f64::from(volume) * (1.0 - t / 120.0).max(0.0)).round() as u8),
                _ => {}
            }
            if let Some((fx, _)) = ticked {
                if c.fx == Fx::None {
                    (c.fx, c.param) = (fx, if fx == Fx::Arpeggio { 0x47 } else { 0 });
                }
            }
            lasting[ch] = r;
        }
    }
    let nch = voices.len().max(1);
    let npat = total_rows.div_ceil(64).max(1);
    let mut patterns = Vec::with_capacity(npat);
    for p in 0..npat {
        let rows = (total_rows - p * 64).min(64);
        let mut pat = Pattern::new(rows, nch);
        for (ch, col) in cells.iter().enumerate() {
            for r in 0..rows {
                if let Some(c) = pat.cell_mut(r, ch) {
                    *c = col[p * 64 + r];
                }
            }
        }
        patterns.push(Arc::new(pat));
    }
    let mut m = Module {
        format: "kdm".into(),
        format_name: "Ken's Digital Music".into(),
        tracker: "Ken's Digital Music".into(),
        kind: Kind::It,
        flags: Flags { linear: true, ..Default::default() },
        speed,
        tempo: 200,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels: (0..nch).map(|_| Channel::default()).collect(),
        orders: (0..npat as u16).collect(),
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Digital Sound Interface Kit, the first format ----
//
// DSIK's modules before its RIFF ones (MilkyPlay's loader, BSD, reads
// them): "DSM" 10h, the title, the channels, orders, samples and
// patterns, speed, tempo and volume, 128 orders, then paragraph
// pointers to each sample's header ("DSI" 10h: a name, where its data
// is in paragraphs, its length, loop start and end, rate of C-4,
// volume) and each pattern: packed rows, each cell a flag byte (the
// channel, and which of note, sample, volume and effect follow), the
// flag bytes XORed with a key that counts up byte by byte from 3.
// Notes are two octaves down from ours; effects are MOD's.

pub fn probe_dsm1(d: &[u8]) -> bool {
    d.len() >= 0xB0 && &d[..4] == b"DSM\x10" && (1..=16).contains(&d[0x25])
}

pub fn load_dsm1(d: &[u8]) -> Result<Module> {
    let title = crate::io::text(&d[4..32]);
    let (nch, nord, nins, npat, speed, tempo, gvol) = (usize::from(d[0x25]), usize::from(d[0x26]).min(128), usize::from(d[0x27]), usize::from(d[0x28]), d[0x29], d[0x2A], d[0x2B]);
    let orders: Vec<u16> = d[0x30..0x30 + nord].iter().map(|&o| if o == 0xFF { ORDER_END } else { u16::from(o) }).collect();
    let mut r = Reader::at(d, 0xB0);
    let ins_ptrs: Vec<usize> = (0..nins).map(|_| r.u16le().map(|p| usize::from(p) * 16)).collect::<Result<_>>()?;
    let pat_ptrs: Vec<usize> = (0..npat).map(|_| r.u16le().map(|p| usize::from(p) * 16)).collect::<Result<_>>()?;
    let mut samples = Vec::with_capacity(nins);
    for &at in &ins_ptrs {
        let Some(h) = d.get(at..at + 47).filter(|h| &h[..4] == b"DSI\x10") else {
            samples.push(Sample::empty());
            continue;
        };
        let w = |o: usize| u16::from_le_bytes([h[o], h[o + 1]]);
        let data_at = (usize::from(h[35]) << 16 | usize::from(w(36))) * 16;
        let (len, ls, le) = (usize::from(w(38)), u32::from(w(40)), u32::from(w(42)));
        let b = d.get(data_at..(data_at + len).min(d.len())).unwrap_or(&[]);
        samples.push(Sample {
            name: crate::io::text(&h[4..34]),
            wave: Arc::new(Wave::new(8, 1, decode(b, Pcm::S8, b.len()))),
            c5: u32::from(w(44)),
            volume: h[46].min(64),
            looped: if le > ls { Loop::forward(ls, le) } else { Loop::default() },
            ..Sample::empty()
        });
    }
    let mut patterns = Vec::with_capacity(npat);
    for &at in &pat_ptrs {
        let mut pat = Pattern::new(64, nch);
        if at == 0 || at + 2 > d.len() {
            patterns.push(Arc::new(pat));
            continue;
        }
        let size = usize::from(u16::from_le_bytes([d[at], d[at + 1]])).saturating_sub(2);
        let b = &d[at + 2..(at + 2 + size).min(d.len())];
        let (mut i, mut row, mut key) = (0usize, 0usize, 3u8);
        while i < b.len() && row < 64 {
            let flag = b[i] ^ key;
            i += 1;
            key = key.wrapping_add(1);
            if flag == 0 {
                row += 1;
                continue;
            }
            let mut take = |n: usize| {
                let v = b.get(i..i + n).map(|s| s.to_vec()).unwrap_or_else(|| vec![0; n]);
                i += n;
                key = key.wrapping_add(n as u8);
                v
            };
            let mut cell = Cell::default();
            if flag & 0x80 != 0 {
                let n = take(1)[0];
                if n != 0 {
                    cell.note = (u16::from(n) + 24).min(u16::from(NOTE_MAX)) as u8;
                }
            }
            if flag & 0x40 != 0 {
                cell.instr = take(1)[0];
            }
            if flag & 0x20 != 0 {
                let v = take(1)[0];
                if v <= 64 {
                    (cell.vol, cell.volp) = (Vol::Volume, v);
                }
            }
            if flag & 0x10 != 0 {
                let e = take(2);
                let (cmd, p) = (e[0], e[1]);
                if cmd <= 0x0F {
                    let (fx, v) = match cmd {
                        0x05 if p == 0 => (Fx::TonePorta, 0),
                        0x06 if p == 0 => (Fx::Vibrato, 0),
                        0x0E if p >> 4 == 8 => (Fx::Panning, (p & 0x0F) * 0x11),
                        _ => super::common::mod_effect(cmd, p, false),
                    };
                    (cell.fx, cell.param) = (fx, v);
                }
            }
            if let Some(c) = pat.cell_mut(row, usize::from(flag & 0x0F)) {
                *c = cell;
            }
        }
        patterns.push(Arc::new(pat));
    }
    let mut m = Module {
        title,
        format: "dsm1".into(),
        format_name: "Digital Sound Interface Kit (first format)".into(),
        tracker: "Digital Sound Interface Kit".into(),
        kind: Kind::Xm,
        speed: speed.max(1),
        tempo: u16::from(tempo).max(32),
        global_volume: (u16::from(gvol.min(64)) * 2) as u8,
        mix_volume: 48,
        separation: 128,
        channels: (0..nch).map(|c| Channel { pan: if matches!(c % 4, 0 | 3) { 64 } else { 192 }, ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}
