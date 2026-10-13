//! Windows-era trackers: MadTracker 2 (.mt2) and Graoumf Tracker 2
//! (.gt2).

use std::sync::Arc;

use super::common::{mod_effect, s3m_effect};
use crate::io::{decode, Pcm, Reader, Result};
use crate::model::*;

// ---- MadTracker 2 ----
//
// "MT20", a version (2.00 to 2.xx), the tracker's name, the title, counts,
// the tempo as samples a tick; 256 orders; the drums' settings (eight
// channels more, each one sample, with patterns of their own an order
// plays); chunks of extras (the artist, a message, the tracks' and
// patterns' names, VST settings); the patterns, packed a channel at a
// time; drum patterns; automation (passed over); 255 instruments, each
// its envelopes and a map of notes to groups (a sample, its volume and
// pitch); 256 samples' headers; the instruments' groups; the samples,
// as deltas, a stereo one's right channel the difference from its left.

pub fn probe_mt2(d: &[u8]) -> bool {
    if d.len() < 126 + 256 || &d[..4] != b"MT20" {
        return false;
    }
    let le16 = |i: usize| u16::from_le_bytes([d[i], d[i + 1]]);
    let (version, nord, nch) = (le16(8), le16(106), le16(112));
    (0x200..0x300).contains(&version) && (1..=64).contains(&nch) && nord <= 256 && le16(122) < 256 && le16(124) < 256
}

/// A cell: its note, instrument, volume, panning and effect (a command
/// and two parameters).
fn mt2_cell(b: [u8; 7]) -> Cell {
    let [note, instr, vol, pan, fx, p1, p2] = b;
    let mut c = Cell { instr, ..Default::default() };
    if note != 0 {
        c.note = if note > 96 { NOTE_OFF } else { note + 12 };
    }
    (c.vol, c.volp) = match vol {
        0x10..=0x90 => (Vol::Volume, (vol - 0x10) / 2),
        0xA0..=0xAF => (Vol::VolSlideDown, vol & 0x0F),
        0xB0..=0xBF => (Vol::VolSlideUp, vol & 0x0F),
        0xC0..=0xCF => (Vol::FineVolDown, vol & 0x0F),
        0xD0..=0xDF => (Vol::FineVolUp, vol & 0x0F),
        _ => (Vol::None, 0),
    };
    let wide = ((u16::from(p2) << 4) | u16::from(p1 >> 4)).min(255) as u8;
    (c.fx, c.param) = if fx == 0 && p1 == 0 && p2 == 0 {
        (Fx::None, 0)
    } else {
        match fx {
            // FastTracker 2's command and parameter.
            0x00 => mod_effect(p2, p1, true),
            // Slides on every tick, finer than FastTracker's.
            0x01 => (Fx::PortaUp, wide),
            0x02 => (Fx::PortaDown, wide),
            0x03 => (Fx::TonePorta, wide),
            0x04 => (Fx::Vibrato, (p2 & 0xF0) | (p1 >> 4)),
            0x08 if p1 != 0 => (Fx::Panning, p1),
            0x08 if p2 == 1 || p2 == 2 => (Fx::S3mEx, 0x91),
            0x0C => (Fx::Volume, (p2 / 2).min(64)),
            0x0F if p2 != 0 => (Fx::Tempo, p2),
            0x0F => (Fx::Speed, p1 & 0x0F),
            // Impulse Tracker's command and parameter.
            0x10 => s3m_effect(p2, p1, true),
            // The gapper: on x + 1 ticks, off y + 1.
            0x1D => (Fx::Tremor, p1),
            // The filter's cutoff, through IT's macro.
            0x20 => (Fx::Midi, p2 >> 1),
            0x22 => (Fx::Midi, (p2 & 0xF0) >> 1),
            0x24 => (Fx::S3mEx, 0x9F),
            0x80 => (Fx::ChannelVolume, p2 / 4),
            0x9D => {
                (c.vol, c.volp) = (Vol::Offset, p2 >> 3);
                (Fx::S3mEx, 0xD0 | p1.min(15))
            }
            _ => (Fx::None, 0),
        }
    };
    if pan != 0 {
        if c.fx == Fx::None {
            (c.fx, c.param) = (Fx::Panning, pan);
        } else if c.vol == Vol::None {
            (c.vol, c.volp) = (Vol::Panning, pan / 4);
        }
    }
    c
}

fn mt2_envelope(b: &[u8], centred: bool) -> Envelope {
    let (flags, n, sus, ls, le) = (b[0], b[1].min(16), b[2], b[3], b[4]);
    let points: Vec<EnvPoint> = (0..usize::from(n))
        .map(|i| {
            let (x, y) = (u16::from_le_bytes([b[8 + i * 4], b[9 + i * 4]]), u16::from_le_bytes([b[10 + i * 4], b[11 + i * 4]]).min(64) as i16);
            EnvPoint { tick: x, value: if centred { (y - 32) as i8 } else { y as i8 } }
        })
        .collect();
    Envelope {
        enabled: flags & 1 != 0 && n > 0,
        sustain: (flags & 2 != 0 && sus < n).then_some((sus, sus)),
        looped: (flags & 4 != 0 && ls <= le && le < n).then_some((ls, le)),
        points,
        ..Default::default()
    }
}

/// A string ended by a NUL.
fn cstring(r: &mut Reader) -> String {
    let rest = &r.data[r.pos.min(r.data.len())..];
    let n = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    let s = crate::io::text(&rest[..n]);
    r.pos = (r.pos + n + 1).min(r.data.len());
    s
}

pub fn load_mt2(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 8);
    let version = r.u16le()?;
    let tracker = r.text(32)?;
    let title = r.text(64)?;
    let nord = usize::from(r.u16le()?);
    let restart = r.u16le()?;
    let npat = usize::from(r.u16le()?);
    let nch = usize::from(r.u16le()?);
    let spt = r.u16le()?;
    let (speed, lpb) = (r.u8()?.clamp(1, 31), r.u8()?.clamp(1, 32));
    let flags = r.u32le()?;
    let nins = usize::from(r.u16le()?);
    let nsmp = usize::from(r.u16le()?);
    let order_bytes = r.bytes(256)?;
    let drums = r.u16le()? != 0;
    let drum_data = if drums { Some(r.bytes(274)?) } else { None };
    let extra_len = r.u32le()? as usize;
    let mut extra = Reader::new(r.bytes(extra_len)?);
    let total = nch + if drums { 8 } else { 0 };

    let mut patterns: Vec<Pattern> = Vec::with_capacity(npat);
    for _ in 0..npat {
        let rows = usize::from(r.u16le()?);
        let size = (r.u32le()? as usize + 1) & !1;
        let data = r.bytes_upto(size);
        let mut p = Pattern::new(rows.clamp(1, 1024), total);
        if rows == 0 {
            patterns.push(p);
            continue;
        }
        let mut pr = Reader::new(data);
        if flags & 1 != 0 {
            // A channel at a time: a byte saying which fields follow,
            // 0xFF first for a count of rows it's repeated over.
            let (mut row, mut ch) = (0usize, 0usize);
            while let Ok(mut info) = pr.u8() {
                let mut repeat = 0usize;
                if info == 0xFF {
                    repeat = usize::from(pr.u8().unwrap_or(0));
                    info = pr.u8().unwrap_or(0);
                }
                if info & 0x7F != 0 {
                    let mut b = [0u8; 7];
                    for (bit, v) in b.iter_mut().enumerate() {
                        if info & (1 << bit) != 0 {
                            *v = pr.u8().unwrap_or(0);
                        }
                    }
                    let cell = mt2_cell(b);
                    for k in 0..=repeat.min(rows - row - 1) {
                        if let Some(c) = p.cell_mut(row + k, ch) {
                            *c = cell;
                        }
                    }
                }
                row += repeat + 1;
                while row >= rows {
                    row -= rows;
                    ch += 1;
                }
                if ch >= nch {
                    break;
                }
            }
        } else {
            for row in 0..rows {
                for ch in 0..nch {
                    let Ok(b) = pr.bytes(7) else { break };
                    p.cells[row * total + ch] = mt2_cell(b.try_into().expect("seven bytes"));
                }
            }
        }
        patterns.push(p);
    }

    // The extras.
    let mut tempo_spt = f64::from(spt);
    let (mut artist, mut message) = (String::new(), String::new());
    let mut chnames: Vec<String> = Vec::new();
    let mut nvst = 0usize;
    while extra.left() >= 8 {
        let id = extra.tag()?;
        let len = extra.u32le()? as usize;
        let mut c = Reader::new(extra.bytes_upto(len));
        match &id {
            b"BPM+" => {
                if let Ok(b) = c.bytes(8) {
                    let v = f64::from_le_bytes(b.try_into().expect("eight bytes"));
                    if v > 1e-8 {
                        tempo_spt = v;
                    }
                }
            }
            b"TRKL" => {
                while !c.eof() && chnames.len() < nch {
                    chnames.push(cstring(&mut c));
                }
            }
            b"PATN" => {
                for p in patterns.iter_mut() {
                    if c.eof() {
                        break;
                    }
                    p.name = cstring(&mut c);
                }
            }
            b"MSG\0" => message = crate::io::message(c.data.get(1..).unwrap_or(&[])),
            b"SUM\0" => {
                c.skip(6).ok();
                let a = cstring(&mut c);
                if a != "Unregistered" {
                    artist = a;
                }
            }
            b"VST2" => nvst = c.u32le().unwrap_or(0) as usize,
            _ => {}
        }
    }

    // The drums: eight channels more, each playing one sample at middle
    // C, their patterns chosen order by order.
    let mut orders: Vec<u16> = order_bytes[..nord].iter().map(|&o| u16::from(o)).collect();
    let mut drum_samples = [0u16; 8];
    if let Some(dd) = drum_data {
        let ndp = usize::from(u16::from_le_bytes([dd[0], dd[1]]));
        for (i, s) in drum_samples.iter_mut().enumerate() {
            *s = u16::from_le_bytes([dd[2 + i * 2], dd[3 + i * 2]]);
        }
        let mut drum_pats = Vec::with_capacity(ndp);
        for _ in 0..ndp {
            let rows = usize::from(r.u16le()?);
            drum_pats.push(r.bytes_upto(rows * 32));
        }
        let mut used: Vec<Option<u8>> = vec![None; patterns.len()];
        for (o, order) in orders.iter_mut().enumerate() {
            let dp = dd[18 + o];
            let Some(cells) = drum_pats.get(usize::from(dp)) else { continue };
            let mut at = usize::from(*order);
            if at >= patterns.len() {
                continue;
            }
            match used[at] {
                None => used[at] = Some(dp),
                Some(u) if u == dp => continue,
                Some(_) => {
                    // This pattern has another drum pattern elsewhere: a copy.
                    patterns.push(patterns[at].clone());
                    used.push(Some(dp));
                    at = patterns.len() - 1;
                    *order = at as u16;
                }
            }
            let p = &mut patterns[at];
            for row in 0..p.rows.min(cells.len() / 32) {
                for k in 0..8 {
                    let b = &cells[row * 32 + k * 4..row * 32 + k * 4 + 4];
                    let mut c = Cell::default();
                    if b[0] & 0x80 != 0 {
                        c.note = NOTE_MIDDLE_C;
                        c.instr = (nins + 1 + k).min(255) as u8;
                        let delay = (b[0] & 0x1F).min(15);
                        if delay > 0 {
                            (c.fx, c.param) = (Fx::S3mEx, 0xD0 | delay);
                        }
                        // 128 is full volume: the other instruments are halved to match.
                        (c.vol, c.volp) = (Vol::Volume, ((u16::from(b[1]) + 3) / 4).min(64) as u8);
                    }
                    p.cells[row * total + nch + k] = c;
                }
            }
        }
    }

    // Automation, passed over.
    if flags & 2 != 0 {
        let envs = if flags & 8 != 0 { total } else { nch } + if version >= 0x250 { nvst } else { 0 } + usize::from(flags & 0x10 != 0);
        for _ in 0..npat * envs {
            if r.left() < 4 {
                break;
            }
            let mut f = if version >= 0x203 {
                let f = r.u32le()?;
                r.skip(4)?;
                f
            } else {
                let f = u32::from(r.u16le()?);
                r.skip(2)?;
                f
            };
            while f != 0 {
                if f & 1 != 0 {
                    r.skip((4 + 4 * 64).min(r.left()))?;
                }
                f >>= 1;
            }
        }
    }

    // Instruments: 255 written, however many there are.
    let mut instruments = Vec::with_capacity(nins + 8);
    let mut groups_of: Vec<Option<(usize, [u8; 96], AutoVibrato)>> = Vec::with_capacity(nins);
    for i in 0..255 {
        let name = r.text(32)?;
        let mut len = r.u32le()? as usize;
        if len == 32 {
            len += 108 + 72 * 4;
        }
        if version > 0x201 && len > 0 {
            len += 4;
        }
        let chunk = r.bytes_upto(len);
        if i >= nins {
            continue;
        }
        let mut inst = Instrument { name, keymap: (0..120).map(|n| (n as u8, if i < nsmp { (i + 1) as u16 } else { 0 })).collect(), global_volume: 64, ..Default::default() };
        if chunk.len() < 106 {
            instruments.push(inst);
            groups_of.push(None);
            continue;
        }
        let mut c = Reader::new(chunk);
        let h = c.bytes(106)?;
        let ngroups = usize::from(u16::from_le_bytes([h[0], h[1]]));
        let map: [u8; 96] = h[2..98].try_into().expect("96 bytes");
        let vib = AutoVibrato { kind: [0, 2, 4, 1][usize::from(h[98] & 3)], sweep: h[99], depth: h[100], speed: h[101] };
        let fade = u16::from_le_bytes([h[102], h[103]]);
        let nna = u16::from_le_bytes([h[104], h[105]]);
        let iflags = if version >= 0x201 { c.u16le().unwrap_or(0) } else { 0 };
        let mut mask = if version >= 0x202 { c.u32le().unwrap_or(0) } else { 3 };
        for env in 0..4 {
            if mask & 1 != 0 {
                let Ok(b) = c.bytes(72) else { break };
                match env {
                    0 => inst.volume_env = mt2_envelope(b, false),
                    1 => inst.pan_env = mt2_envelope(b, true),
                    2 => inst.pitch_env = mt2_envelope(b, true),
                    _ => {
                        let e = mt2_envelope(b, true);
                        if e.enabled {
                            inst.pitch_env = Envelope { filter: true, ..e };
                        }
                    }
                }
            }
            mask >>= 1;
        }
        inst.nna = Nna::from_u8((nna & 3) as u8);
        inst.dct = ((nna >> 8) & 3) as u8;
        inst.dca = [0, 2, 1, 2][usize::from((nna >> 12) & 3)];
        inst.fadeout = if !inst.volume_env.enabled && inst.nna != Nna::Fade { 65535 } else { u32::from(fade) * 2 };
        // A VST or MIDI instrument plays no samples.
        let plugged = iflags & 4 != 0;
        if plugged {
            inst.keymap.iter_mut().for_each(|k| k.1 = 0);
        }
        groups_of.push((!plugged).then_some((ngroups, map, vib)));
        instruments.push(inst);
    }

    struct Head {
        name: String,
        frames: usize,
        b16: bool,
        stereo: bool,
        external: bool,
        rate: u32,
        looped: Loop,
        volume: u8,
        pan: Option<u8>,
        surround: bool,
        note: i8,
        synced: bool,
    }
    let mut heads: Vec<Head> = Vec::with_capacity(nsmp);
    for i in 0..256 {
        let name = r.text(32)?;
        let len = r.u32le()? as usize;
        let chunk = r.bytes_upto(len);
        if i >= nsmp {
            continue;
        }
        let mut h = Head { name, frames: 0, b16: false, stereo: false, external: false, rate: 8363, looped: Loop::default(), volume: 64, pan: None, surround: false, note: 49, synced: false };
        if chunk.len() >= 26 {
            let s = chunk;
            let le32 = |k: usize| u32::from_le_bytes([s[k], s[k + 1], s[k + 2], s[k + 3]]);
            let (bytes, rate, depth, chans, sflags, ltype) = (le32(0) as usize, le32(4), s[8], s[9], s[10], s[11]);
            h.b16 = depth > 1;
            h.stereo = chans > 1;
            h.frames = bytes / if h.b16 { 2 } else { 1 } / if h.stereo { 2 } else { 1 };
            h.rate = rate;
            let kind = match ltype {
                1 => LoopKind::Forward,
                2 => LoopKind::PingPong,
                _ => LoopKind::Off,
            };
            h.looped = Loop { kind, start: le32(12), end: le32(16) };
            h.volume = (u16::from_le_bytes([s[20], s[21]]) >> 9).min(64) as u8;
            let pan = s[22] as i8;
            if pan == -128 {
                h.surround = true;
            } else {
                h.pan = Some((i16::from(pan) + 128) as u8);
            }
            h.note = s[23] as i8;
            h.synced = sflags & 2 != 0;
            h.external = sflags & 5 != 0;
        }
        heads.push(h);
    }

    // The groups: each instrument's notes to samples, with a volume and pitch.
    let mut sample_gv = vec![64u8; nsmp];
    let mut sample_fine = vec![0i8; nsmp];
    let mut sample_vib = vec![AutoVibrato::default(); nsmp];
    for (i, g) in groups_of.iter().enumerate() {
        let Some((n, map, vib)) = g else { continue };
        let mut groups = Vec::with_capacity(*n);
        for _ in 0..*n {
            groups.push(r.bytes(8)?);
        }
        let inst = &mut instruments[i];
        inst.keymap.iter_mut().for_each(|k| k.1 = 0);
        for (note, &gi) in map.iter().enumerate() {
            let Some(grp) = groups.get(usize::from(gi)) else { continue };
            let s = usize::from(grp[0]) + 1;
            inst.keymap[note + 12].1 = s as u16;
            if let Some(h) = heads.get(s - 1) {
                sample_gv[s - 1] = (u16::from(grp[1]) * 2).min(64) as u8;
                sample_fine[s - 1] = grp[2] as i8;
                sample_vib[s - 1] = *vib;
                // A sample kept to the beat always plays as middle C.
                if h.synced {
                    inst.keymap[note + 12].0 = NOTE_MIDDLE_C - 1;
                }
            }
        }
    }
    // The drums' instruments.
    if drums {
        for (k, &s) in drum_samples.iter().enumerate() {
            let mut inst = Instrument::for_sample(&format!("Drum {}", k + 1), s + 1);
            inst.global_volume = 128;
            instruments.push(inst);
        }
    }

    let mut samples = Vec::with_capacity(nsmp);
    for (i, h) in heads.into_iter().enumerate() {
        let (wave, filename) = if h.external {
            let n = r.u32le().unwrap_or(0) as usize;
            r.skip(12.min(r.left()))?;
            (Wave::default(), crate::io::text(r.bytes_upto(n)))
        } else if h.stereo {
            let fmt = if h.b16 { Pcm::D16le } else { Pcm::D8 };
            let left = decode(r.bytes_upto(h.frames * fmt.bytes()), fmt, h.frames);
            let right = decode(r.bytes_upto(h.frames * fmt.bytes()), fmt, h.frames);
            let mut pcm = Vec::with_capacity(left.len() * 2);
            for (j, &l) in left.iter().enumerate() {
                // The right channel is the difference from the left.
                let r = right.get(j).copied().unwrap_or(0);
                let r = if h.b16 { l.wrapping_add(r) } else { (((l >> 8) as i8).wrapping_add((r >> 8) as i8) as i16) << 8 };
                pcm.extend([l, r]);
            }
            (Wave::new(if h.b16 { 16 } else { 8 }, 2, pcm), String::new())
        } else {
            let fmt = if h.b16 { Pcm::D16le } else { Pcm::D8 };
            (Wave::new(if h.b16 { 16 } else { 8 }, 1, decode(r.bytes_upto(h.frames * fmt.bytes()), fmt, h.frames)), String::new())
        };
        let semis = 49.0 - f64::from(h.note) + f64::from(sample_fine[i]) / 128.0;
        samples.push(Sample {
            name: h.name,
            filename,
            wave: Arc::new(wave),
            c5: (f64::from(h.rate.max(1)) * 2f64.powf(semis / 12.0)).round().clamp(1.0, 2_000_000.0) as u32,
            volume: h.volume,
            global_volume: sample_gv[i],
            pan: h.pan,
            looped: h.looped,
            vibrato: sample_vib[i],
            ..Sample::empty()
        });
        let _ = h.surround;
    }

    let mut m = Module {
        title,
        artist,
        message,
        format: "mt2".into(),
        format_name: format!("MadTracker {}.{:02x}", version >> 8, version & 0xFF),
        tracker: if tracker.is_empty() { "MadTracker 2".into() } else { tracker },
        kind: Kind::Xm,
        flags: Flags { linear: true, instruments: true, compat_gxx: true, ..Default::default() },
        speed,
        // A tick is as many samples (at 44.1 kHz) as it says.
        tempo: if tempo_spt > 1.0 { (110_250.0 / tempo_spt).round().clamp(32.0, 999.0) as u16 } else { 125 },
        time: TimeSig { rows_per_beat: u16::from(lpb), rows_per_measure: u16::from(lpb) * 4 },
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        restart,
        channels: (0..total)
            .map(|c| Channel { name: chnames.get(c).cloned().unwrap_or_else(|| if c >= nch { format!("Drums {}", c - nch + 1) } else { String::new() }), ..Default::default() })
            .collect(),
        orders,
        patterns: patterns.into_iter().map(Arc::new).collect(),
        samples,
        instruments,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Graoumf Tracker 2 ----
//
// "GT2", a version (1 to 9), the header's size, title, comment, date and
// tracker, then (to version 5, the Falcon's) speed, tempo, master
// volume and the tracks' panning; then chunks: PATS the channels, SONG
// the orders, XCOM a comment, TCN1 and TCN2 settings, TVOL and MIXP the
// tracks' levels, SAMP or SAM2 a sample, INST an instrument, VENV, TENV
// and PENV the old instruments' envelope programs, PATD a pattern (cells
// of five bytes, Graoumf Tracker 1's effects and more), TNAM names.

pub fn probe_gt2(d: &[u8]) -> bool {
    if d.len() < 236 || &d[..3] != b"GT2" || d[3] > 9 {
        return false;
    }
    let be16 = |i: usize| u16::from_be_bytes([d[i], d[i + 1]]);
    let year = be16(202);
    if !(1980..=9999).contains(&year) {
        return false;
    }
    d[3] > 5 || (be16(228) > 0 && be16(230) > 0 && be16(232) <= 0xFFF && be16(234) <= 99)
}

/// A Graoumf Tracker 2 effect: Graoumf Tracker 1's, with 8xyz a roll and
/// a balance, and the commands from B0 on.
fn gt2_effect(cell: &mut Cell, fx: u8, p: u8) {
    let p4 = p.min(15);
    let slide = p.min(14);
    let div128 = ((u16::from(p) + 127) / 128).min(14) as u8;
    let (f, v) = match fx {
        0x80..=0x8F => {
            if cell.vol == Vol::None {
                (cell.vol, cell.volp) = (Vol::Panning, (u16::from(fx & 0x0F) * 64 / 15) as u8);
            }
            (Fx::Retrig, p)
        }
        0xB0 => (Fx::Tremor, p),
        0xB1 => (Fx::S3mEx, 0xB0 | p4),
        // The instrument's envelopes on and off (there's one of each).
        0xB3 => (Fx::S3mEx, 0x77 + u8::from(p != 0)),
        0xB4 => (Fx::S3mEx, 0x7B + if p != 0 { 2 } else { 0 }),
        0xB5 => (Fx::S3mEx, 0x79 + u8::from(p != 0)),
        0xB6 => (Fx::S3mEx, 0x7B + if p != 0 { 3 } else { 0 }),
        0xBA => (Fx::Offset, ((u16::from(p) + 15) / 16) as u8),
        0xBB => (Fx::Offset, 1),
        0xC0 if matches!(p >> 4, 1 | 2) => (Fx::S3mEx, 0x9D + (p >> 4)),
        0xC2 => (Fx::ChannelVolume, p.saturating_mul(4).min(64)),
        0xC3 => (Fx::ChannelVolume, (super::pc2::graoumf_log(u16::from(p) << 4) / 64).min(64) as u8),
        0xC4 | 0xC6 => (Fx::ChannelVolSlide, div128 << 4),
        0xC5 | 0xC7 => (Fx::ChannelVolSlide, div128),
        0xC8 => (Fx::ChannelVolSlide, slide << 4 | 0x0F),
        0xC9 => (Fx::ChannelVolSlide, slide | 0xF0),
        0xB2 | 0xB7..=0xB9 | 0xBC..=0xFF => return,
        _ => return super::pc2::gtk_effect(cell, fx, p),
    };
    (cell.fx, cell.param) = (f, v);
}

/// A version 1 or later instrument's envelope (282 bytes): points of a
/// duration and a value, loops, a fade-out and an LFO.
fn gt2_envelope(b: &[u8], which: usize) -> Envelope {
    let be16 = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
    let n = usize::from(be16(0)).min(64);
    let flags = be16(12);
    let (lo, hi): (i32, i32) = match which {
        0 => (0, 4096),
        2 => (-128, 127),
        _ => (-9999 * 16 / 99, 9999 * 16 / 99),
    };
    let mut tick = 0u16;
    let points: Vec<EnvPoint> = (0..n)
        .map(|i| {
            let (dur, v) = (be16(26 + i * 4), be16(28 + i * 4) as i16);
            let x = (i32::from(v) - lo) * 64 / (hi - lo);
            let p = EnvPoint { tick, value: if which == 0 { x.clamp(0, 64) as i8 } else { (x.clamp(0, 64) - 32) as i8 } };
            tick = tick.saturating_add(dur.max(1));
            p
        })
        .collect();
    let k = points.len();
    let pair = |a: u16, z: u16| (usize::from(a) <= usize::from(z) && usize::from(z) < k).then_some((a as u8, z as u8));
    Envelope {
        enabled: k > 0,
        looped: if flags & 0x10 != 0 { pair(be16(14), be16(16)) } else { None },
        sustain: if flags & 0x20 != 0 { pair(be16(20), be16(22)) } else { None },
        points,
        ..Default::default()
    }
}

pub fn load_gt2(d: &[u8]) -> Result<Module> {
    let version = d[3];
    let mut r = Reader::at(d, 4);
    let header = r.u32be()? as usize;
    let title = r.text(32)?;
    let comment = crate::io::message(r.bytes(160)?);
    r.skip(4)?;
    let tracker = r.text(24)?;
    let (mut speed, mut tempo, mut gv) = (6u16, 125u16, None);
    let mut pans = Vec::new();
    if version <= 5 {
        speed = r.u16be()?.max(1);
        tempo = r.u16be()?.max(1);
        gv = Some((u32::from(r.u16be()?) * 128 / 4095).min(128) as u8);
        let n = usize::from(r.u16be()?);
        for _ in 0..n {
            pans.push(r.u16be()?);
        }
    }
    r.seek(header.max(r.pos).min(d.len()))?;
    let mut chunks: Vec<([u8; 4], &[u8])> = Vec::new();
    while r.left() >= 8 {
        let id = r.tag()?;
        let len = (r.u32be()? as usize).max(8) - 8;
        if &id == b"ENDC" {
            break;
        }
        chunks.push((id, r.bytes_upto(len)));
    }
    let first = |id: &[u8; 4]| chunks.iter().find(|c| &c.0 == id).map(|c| c.1);
    let all = |id: &'static [u8; 4]| chunks.iter().filter(move |c| &c.0 == id).map(|c| c.1);
    let nch = first(b"PATS")
        .filter(|c| c.len() >= 2)
        .map(|c| usize::from(u16::from_be_bytes([c[0], c[1]])))
        .filter(|&n| (1..=64).contains(&n))
        .ok_or_else(|| crate::io::Error::Bad("it says nothing of its channels".into()))?;
    let mut channels: Vec<Channel> =
        (0..nch).map(|c| Channel { pan: pans.get(c).map_or(super::common::amiga_pan(c), |&p| (u32::from(p) * 255 / 4095).min(255) as u8), ..Default::default() }).collect();
    let mut sr = Reader::new(first(b"SONG").ok_or_else(|| crate::io::Error::Bad("it has no order list".into()))?);
    let nord = usize::from(sr.u16be()?);
    let restart = sr.u16be()?;
    let orders: Vec<u16> = (0..nord.min(sr.left() / 2)).map(|_| sr.u16be()).collect::<Result<_>>()?;
    let message = match first(b"XCOM").filter(|c| c.len() >= 3) {
        Some(c) => crate::io::message(&c[2..(2 + usize::from(u16::from_be_bytes([c[0], c[1]]))).min(c.len())]),
        None => comment,
    };
    if let Some(c) = first(b"TCN1").filter(|c| c.first() == Some(&0) && c.len() >= 2652) {
        let mute = u32::from_be_bytes([c[2648], c[2649], c[2650], c[2651]]);
        for (i, ch) in channels.iter_mut().enumerate().take(32) {
            ch.muted = mute & (1 << i) == 0;
        }
    }
    let mut linear = false;
    let mut time = TimeSig::default();
    if let Some(c) = first(b"TCN2").filter(|c| c.len() >= 12) {
        let be16 = |i: usize| u16::from_be_bytes([c[i], c[i + 1]]);
        tempo = be16(2).clamp(32, 999);
        speed = be16(6).clamp(1, 255);
        let beat = 16 / be16(10).clamp(1, 16);
        time = TimeSig { rows_per_beat: beat, rows_per_measure: beat * be16(8).clamp(1, 16) };
        if be16(0) >= 1 && c.len() >= 14 {
            linear = be16(12) == 0;
        }
    }
    if let Some(c) = first(b"TVOL").filter(|c| c.len() >= 2) {
        let n = usize::from(u16::from_be_bytes([c[0], c[1]])).min(nch).min((c.len() - 2) / 2);
        for (i, ch) in channels.iter_mut().enumerate().take(n) {
            ch.volume = (u16::from_be_bytes([c[2 + i * 2], c[3 + i * 2]]).min(4096) / 64) as u8;
        }
    }
    if let Some(c) = first(b"MIXP").filter(|c| c.len() >= 56) {
        let be16 = |i: usize| u16::from_be_bytes([c[i], c[i + 1]]);
        if be16(34) == 4 && be16(48) == 0x101 {
            gv = Some((u32::from(be16(40).min(0x4000)) * 128 / 0x1000).min(128) as u8);
            for t in 0..usize::from(be16(38)) {
                let Some(m) = c.get(56 + t * 8..64 + t * 8) else { break };
                let idx = usize::from(u16::from_be_bytes([m[2], m[3]]));
                if m[0] == 0 && idx < nch {
                    channels[idx].pan = (u32::from(u16::from_be_bytes([m[6], m[7]])) * 255 / 0xFFF).min(255) as u8;
                    channels[idx].volume = (u32::from(u16::from_be_bytes([m[4], m[5]])) * 64 / 0x1000).min(64) as u8;
                }
            }
        }
    }

    let mut samples: Vec<Sample> = Vec::new();
    let put = |samples: &mut Vec<Sample>, n: usize, s: Sample| {
        if samples.len() < n {
            samples.resize(n, Sample::empty());
        }
        samples[n - 1] = s;
    };
    for c in all(b"SAM2") {
        let mut cr = Reader::new(c);
        let Ok(h) = cr.bytes(78) else { continue };
        let be16 = |i: usize| u16::from_be_bytes([h[i], h[i + 1]]);
        let be32 = |i: usize| u32::from_be_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
        let num = usize::from(be16(0));
        let (kind, bits, little, chans, pan, vol, ft, ltype, coding, fnlen) =
            (be16(30), be16(32), be16(34) != 0, be16(36).clamp(1, 2), be16(38) as i16, be16(40), be16(42) as i16, be16(44), be16(48), usize::from(be16(50)));
        if num == 0 || num > 255 || coding > 1 || kind > 1 {
            continue;
        }
        let (rate, len, ls, ll, at) = (be32(54), be32(58), be32(62), be32(66), be32(74) as usize);
        let b16 = bits != 8;
        let mut wave = Wave::default();
        let mut filename = String::new();
        if cr.seek(at.saturating_sub(8)).is_ok() {
            let points = len as usize * usize::from(chans);
            if kind == 0 {
                let fmt = match (b16, little) {
                    (false, _) => Pcm::S8,
                    (true, true) => Pcm::S16le,
                    (true, false) => Pcm::S16be,
                };
                wave = Wave::new(if b16 { 16 } else { 8 }, chans as u8, decode(cr.bytes_upto(points * fmt.bytes()), fmt, points));
            }
            filename = crate::io::text(cr.bytes_upto(fnlen));
        }
        let looped = match ltype {
            0 => Loop::default(),
            2 => Loop { kind: LoopKind::PingPong, start: ls, end: ls.saturating_add(ll) },
            _ => Loop::forward(ls, ls.saturating_add(ll)),
        };
        put(
            &mut samples,
            num,
            Sample {
                name: crate::io::text(&h[2..30]),
                filename,
                wave: Arc::new(wave),
                c5: (f64::from(rate) * 2.0 * 2f64.powf(f64::from(ft) / 96.0)).round().clamp(1.0, 2_000_000.0) as u32,
                global_volume: (vol / 4).min(64) as u8,
                pan: (pan > 0).then(|| (u32::from(pan as u16) * 255 / 4095).min(255) as u8),
                looped,
                ..Sample::empty()
            },
        );
    }
    for c in all(b"SAMP") {
        let mut cr = Reader::new(c);
        let Ok(h) = cr.bytes(56) else { continue };
        let be16 = |i: usize| u16::from_be_bytes([h[i], h[i + 1]]);
        let be32 = |i: usize| u32::from_be_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
        let num = usize::from(be16(0));
        let (flags, pan, bits, rate) = (be16(30), be16(32) as i16, be16(34), be16(36));
        let (mut len, mut ls, ll) = (be32(38), be32(42), be32(46));
        let (vol, ft, coding) = (be16(50) as i16, be16(52) as i16, be16(54));
        if num == 0 || num > 255 || coding != 0 {
            continue;
        }
        let b16 = bits == 16;
        let mut le = ls.saturating_add(ll);
        if b16 {
            (len, ls, le) = (len / 2, ls / 2, le / 2);
        }
        let chans = if flags & 1 != 0 { 2 } else { 1 };
        let points = len as usize * chans;
        let fmt = if b16 { Pcm::S16be } else { Pcm::S8 };
        let wave = Wave::new(if b16 { 16 } else { 8 }, chans as u8, decode(cr.bytes_upto(points * fmt.bytes()), fmt, points));
        let looped = if ls > 0 || ll > 2 { Loop { kind: if flags & 2 != 0 { LoopKind::PingPong } else { LoopKind::Forward }, start: ls, end: le } } else { Loop::default() };
        put(
            &mut samples,
            num,
            Sample {
                name: crate::io::text(&h[2..30]),
                wave: Arc::new(wave),
                c5: (f64::from(rate) * 2.0 * 2f64.powf(f64::from(ft) / 96.0)).round().clamp(1.0, 2_000_000.0) as u32,
                global_volume: (vol.max(0) / 4).min(64) as u8,
                pan: (pan > 0).then(|| (u32::from(pan as u16) * 255 / 4095).min(255) as u8),
                looped,
                ..Sample::empty()
            },
        );
    }

    let mut instruments: Vec<Instrument> = Vec::new();
    for c in all(b"INST") {
        let mut cr = Reader::new(c);
        let Ok(h) = cr.bytes(308) else { continue };
        let be16 = |i: usize| u16::from_be_bytes([h[i], h[i + 1]]);
        let num = usize::from(be16(0));
        if num == 0 || num > 255 || be16(30) != 0 {
            continue;
        }
        let (velocity, pan, iversion) = (be16(32), be16(34) as i16, be16(50));
        let (vol_env, tone_env, pan_env, cut_env) = (be16(36), be16(38), be16(40), be16(42));
        let mut inst = Instrument {
            name: crate::io::text(&h[2..30]),
            pan: (pan > 0).then(|| (u32::from(pan as u16) * 255 / 4095).min(255) as u8),
            keymap: (0..120).map(|n| ((n as i32 + i32::from(h[53 + n * 2] as i8)).clamp(0, 119) as u8, u16::from(h[52 + n * 2]))).collect(),
            global_volume: 128,
            ..Default::default()
        };
        for &(_, s) in &inst.keymap {
            if let Some(smp) = samples.get_mut(usize::from(s).wrapping_sub(1)) {
                smp.volume = (velocity / 4).min(64) as u8;
            }
        }
        if iversion == 0 {
            // The old envelopes are programs, not kept: a note let go
            // with one fades, without one stops.
            inst.fadeout = if vol_env != 0 { 1024 } else { 65535 };
        } else {
            let ext_len = if iversion == 1 { 98 } else { 102 };
            let ext = cr.bytes(ext_len).unwrap_or(&[]);
            let nenv = if ext.len() == 102 { usize::from(ext[101]) } else { 0 };
            let filtered = ext.first().is_some_and(|_| u16::from_be_bytes([ext[0], ext[1]]) & 1 != 0);
            for e in 0..nenv {
                let Ok(b) = cr.bytes(282) else { break };
                let on = [vol_env != 0, tone_env != 0, pan_env != 0, cut_env != 0 && filtered];
                if e >= 4 || !on[e] || (e == 3 && inst.pitch_env.enabled) {
                    continue;
                }
                let env = gt2_envelope(b, [0, 1, 2, 1][e]);
                let flags = u16::from_be_bytes([b[12], b[13]]);
                match e {
                    0 => {
                        inst.fadeout = if flags & 1 != 0 { u32::from(u16::from_be_bytes([b[10], b[11]]) & 0x7FFF) * 2 } else { 0 };
                        inst.volume_env = env;
                    }
                    2 => inst.pan_env = env,
                    _ => {
                        if e == 1 && flags & 2 != 0 {
                            // The LFO as the samples' auto-vibrato.
                            let be = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
                            let vib = AutoVibrato {
                                kind: [0, 2, 0, 4, 1, 3].get(usize::from(be(8))).copied().unwrap_or(0),
                                sweep: (255 / be(6).max(1)) as u8,
                                depth: (u32::from(be(2)) * 64 / 100).min(255) as u8,
                                speed: (256 / be(4).max(1)).min(255) as u8,
                            };
                            for &(_, s) in &inst.keymap {
                                if let Some(smp) = samples.get_mut(usize::from(s).wrapping_sub(1)) {
                                    smp.vibrato = vib;
                                }
                            }
                        }
                        inst.pitch_env = Envelope { filter: e == 3, ..env };
                    }
                }
            }
        }
        if instruments.len() < num {
            instruments.resize(num, Instrument { keymap: (0..120).map(|n| (n as u8, 0)).collect(), global_volume: 128, ..Default::default() });
        }
        instruments[num - 1] = inst;
    }

    let mut patterns: Vec<Arc<Pattern>> = Vec::new();
    for c in all(b"PATD") {
        let mut pr = Reader::new(c);
        let Ok(num) = pr.u16be() else { continue };
        let name = pr.text(16).unwrap_or_default();
        let (coding, rows, tracks) = (pr.u16be().unwrap_or(9), usize::from(pr.u16be().unwrap_or(0)), usize::from(pr.u16be().unwrap_or(0)));
        if coding > 1 || rows == 0 || pr.left() < rows * tracks * 5 {
            continue;
        }
        let mut p = Pattern::new(rows.min(1024), nch);
        p.name = name;
        for row in 0..rows.min(1024) {
            for ch in 0..tracks {
                let b = pr.bytes(5)?;
                if ch >= nch {
                    continue;
                }
                let cell = &mut p.cells[row * nch + ch];
                if (1..=120).contains(&b[0]) {
                    cell.note = b[0] + 1;
                }
                cell.instr = b[1];
                if b[4] != 0 {
                    (cell.vol, cell.volp) = (Vol::Volume, if coding == 0 { b[4] / 4 } else { b[4].saturating_sub(0x10) }.min(64));
                }
                gt2_effect(cell, b[2], b[3]);
            }
        }
        let num = usize::from(num);
        if patterns.len() <= num {
            patterns.resize(num + 1, Arc::new(Pattern::new(64, nch)));
        }
        patterns[num] = Arc::new(p);
    }
    if let Some(c) = first(b"TNAM").filter(|c| c.len() >= 2) {
        let n = usize::from(u16::from_be_bytes([c[0], c[1]]));
        for t in c[2..].chunks_exact(36).take(n) {
            let idx = usize::from(u16::from_be_bytes([t[2], t[3]]));
            if t[0] == 0 && t[1] == 0 && idx < nch {
                channels[idx].name = crate::io::text(&t[4..36]);
            }
        }
    }
    let has_instruments = !instruments.is_empty();
    let mut m = Module {
        title,
        message,
        format: "gt2".into(),
        format_name: format!("Graoumf Tracker 2 (version {version})"),
        tracker: if tracker.is_empty() { "Graoumf Tracker 2".into() } else { tracker },
        kind: Kind::It,
        flags: Flags { linear, instruments: has_instruments, ..Default::default() },
        speed: speed.min(255) as u8,
        tempo,
        time,
        // By default, a level that leaves room for every track.
        global_volume: gv.unwrap_or((192 / (3 + nch)) as u8).max(1),
        mix_volume: 128,
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
