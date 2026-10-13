//! More of the Amiga's trackers: SoundTracker Pro II (.stp) and
//! Pumatracker (.puma).

use std::sync::Arc;

use super::common::{amiga_pan, mod_effect};
use crate::io::{decode, Pcm, Reader, Result};
use crate::model::*;

// ---- SoundTracker Pro II ----
//
// "STP3", a version (0 to 2), the orders, the speed (and a fraction of
// one) and a CIA timer for the tempo; samples, each its number, a path,
// a name and a header (later versions add loops, several a sample);
// patterns of four-byte cells (instrument, note, command, parameter),
// any number of channels from version 1; scripts and a drum pad passed
// over; the samples. Many of its effects are "auto" ones that go on, row
// after row, until they're stopped: here they're written into each row
// they play on, every order its own pattern so they carry on from one
// to the next as they're heard.

pub fn probe_stp(d: &[u8]) -> bool {
    d.len() >= 204 && &d[..4] == b"STP3" && u16::from_be_bytes([d[4], d[5]]) <= 2 && d[6] <= 128 && u16::from_be_bytes([d[140], d[141]]) != 0 && u16::from_be_bytes([d[148], d[149]]) == 50
}

/// SoundTracker Pro II's CIA timer as beats a minute (3546 is 125).
fn stp_tempo(cia: u16) -> u8 {
    (125.0 * 3546.0 / f64::from(cia.max(1))).round().clamp(32.0, 255.0) as u8
}

/// The auto effects running on a channel.
#[derive(Clone, Copy, Default)]
struct StpAuto {
    tone: bool,
    vibrato: bool,
    tremolo: bool,
    /// A pitch slide each row: porta up or down, or a fine one.
    porta: Option<(Fx, u8)>,
    /// A fine volume slide each row (ModEx EAx or EBx).
    volume: Option<u8>,
    /// A global volume slide.
    global: Option<u8>,
}

impl StpAuto {
    /// Fills a cell with the auto effects it doesn't stop or set: the
    /// first in the effect column, a volume slide or a slide to the note
    /// in the volume column.
    fn fill(&self, cell: &mut Cell, written: (bool, bool)) {
        let mut put = |fx: Fx, p: u8, vol: Option<(Vol, u8)>| {
            if cell.fx == Fx::None && !written.0 {
                (cell.fx, cell.param) = (fx, p);
            } else if let Some((v, x)) = vol.filter(|_| cell.vol == Vol::None && !written.1) {
                (cell.vol, cell.volp) = (v, x);
            }
        };
        if self.tone {
            put(Fx::TonePorta, 0, Some((Vol::TonePorta, 0)));
        }
        if let Some((fx, p)) = self.porta {
            put(fx, p, None);
        }
        if self.vibrato {
            put(Fx::Vibrato, 0, None);
        }
        if let Some(p) = self.volume {
            let vol = if p >> 4 == 0xA { (Vol::FineVolUp, p & 0x0F) } else { (Vol::FineVolDown, p & 0x0F) };
            put(Fx::ModEx, p, Some(vol));
        }
        if self.tremolo {
            put(Fx::Tremolo, 0, None);
        }
        if let Some(p) = self.global {
            put(Fx::GlobalVolSlide, p, None);
        }
    }
}

/// A cell's command read, starting or stopping the channel's auto
/// effects. Returns whether it wrote the effect and the volume column.
fn stp_effect(cell: &mut Cell, cmd: u8, p: u8, auto: &mut StpAuto, frac: &mut u8) -> (bool, bool) {
    // Volume slides have their nibbles swapped, and up and down add up.
    let total = -i32::from(p >> 4) + i32::from(p & 0x0F);
    let slide = if total > 0 { (total << 4) as u8 } else { (-total) as u8 };
    let pitch_slide = matches!(cmd, 0x01..=0x06 | 0x09 | 0x0A);
    if pitch_slide && auto.porta.is_some() {
        auto.porta = None;
    }
    fn set(cell: &mut Cell, f: Fx, v: u8) -> (bool, bool) {
        (cell.fx, cell.param) = (f, v);
        (true, false)
    }
    if cmd & 0xF0 == 0xF0 {
        let cia = u16::from(cmd & 0x0F) << 8 | u16::from(p);
        return if cia != 0 { set(cell, Fx::Tempo, stp_tempo(cia)) } else { (false, false) };
    }
    match cmd {
        0x00 if p != 0 => set(cell, Fx::Arpeggio, p),
        0x01 if p != 0 => set(cell, Fx::PortaUp, p),
        0x02 if p != 0 => set(cell, Fx::PortaDown, p),
        0x03..=0x06 => {
            let fx = match cmd {
                0x03 => (Fx::ModEx, 0x10 | p.min(15)),
                0x04 => (Fx::ModEx, 0x20 | p.min(15)),
                0x05 => (Fx::PortaUp, p),
                _ => (Fx::PortaDown, p),
            };
            auto.porta = (p != 0).then_some(fx);
            if p != 0 {
                set(cell, fx.0, fx.1)
            } else {
                (false, false)
            }
        }
        0x07 => {
            auto.global = None;
            set(cell, Fx::GlobalVolume, p.min(64) * 2)
        }
        0x08 => {
            auto.global = (total != 0).then_some(if total < 0 { slide } else { slide & 0xF0 });
            match auto.global {
                Some(g) => set(cell, Fx::GlobalVolSlide, g),
                None => (false, false),
            }
        }
        0x09 => set(cell, Fx::ModEx, 0x10 | p.min(15)),
        0x0A => set(cell, Fx::ModEx, 0x20 | p.min(15)),
        0x0B | 0x20 if cmd == 0x0B || p & 0xF0 != 0 => {
            // A fine slide each row: down when it has a low nibble.
            let s = if cmd == 0x20 { p >> 4 } else { slide };
            let e = if s & 0x0F != 0 { 0xB0 | (s & 0x0F) } else { 0xA0 | (s >> 4) };
            auto.volume = (s != 0).then_some(e);
            if s != 0 {
                set(cell, Fx::ModEx, e)
            } else {
                (false, false)
            }
        }
        0x20 => set(cell, Fx::ModEx, 0xC0 | (p & 0x0F)),
        0x0C => {
            auto.volume = None;
            (cell.vol, cell.volp) = (Vol::Volume, p.min(64));
            (false, true)
        }
        0x0D => {
            auto.volume = None;
            if total == 0 {
                return (false, false);
            }
            (cell.vol, cell.volp) = if total < 0 { (Vol::VolSlideDown, slide & 0x0F) } else { (Vol::VolSlideUp, slide >> 4) };
            (false, true)
        }
        // The filter: the other way round to MOD's.
        0x0E => set(cell, Fx::ModEx, u8::from(p == 0)),
        0x0F => {
            *frac = p & 0x0F;
            if p >> 4 != 0 {
                set(cell, Fx::Speed, p >> 4)
            } else {
                (false, false)
            }
        }
        0x10 => {
            auto.vibrato = p != 0;
            set(cell, Fx::Vibrato, p)
        }
        0x11 => {
            auto.tremolo = p & 0x0F != 0;
            set(cell, Fx::Tremolo, p)
        }
        0x12 => set(cell, Fx::PatternBreak, p),
        0x13 => {
            auto.tone = p != 0;
            set(cell, Fx::TonePorta, p)
        }
        0x14 => set(cell, Fx::PositionJump, p),
        0x1D => {
            auto.volume = None;
            match total {
                t if t < 0 => set(cell, Fx::ModEx, 0xB0 | (slide & 0x0F)),
                t if t > 0 => set(cell, Fx::ModEx, 0xA0 | (slide >> 4)),
                _ => (false, false),
            }
        }
        0x21 => set(cell, Fx::ModEx, 0xD0 | p.min(15)),
        0x22 => set(cell, Fx::ModEx, 0x90 | p.min(15)),
        0x49 => set(cell, Fx::Offset, p),
        0x4E if matches!(p >> 4, 0x6 | 0xE) => set(cell, Fx::ModEx, p),
        0x4F => {
            if p < 0x20 {
                *frac = 0;
            }
            let (f, v) = mod_effect(0x0F, p, false);
            if f != Fx::None {
                set(cell, f, v)
            } else {
                (false, false)
            }
        }
        _ => (false, false),
    }
}

/// A string ended by a NUL, `most` bytes at most (the NUL with them).
fn stp_cstring(c: &mut Reader, most: usize) -> Result<String> {
    let rest = c.peek(most.min(c.left())).unwrap_or(&[]);
    let n = rest.iter().position(|&b| b == 0).map_or(rest.len(), |i| i + 1);
    Ok(crate::io::text(c.bytes(n)?))
}

pub fn load_stp(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 4);
    let version = r.u16be()?;
    let nord = usize::from(r.u8()?);
    let mut rows = usize::from(r.u8()?);
    let order_list = r.bytes(128)?.to_vec();
    let speed = r.u16be()?;
    let mut frac = r.u16be()? as u8;
    let timer = r.u16be()?;
    r.skip(2 + 4 + 2 + 50)?;
    let nsmp = usize::from(r.u16be()?);
    let struct_size = usize::from(r.u16be()?);
    struct Head {
        index: usize,
        path: String,
        name: String,
        len: u32,
        vol: u8,
        ls: u32,
        ll: u32,
        ft: i8,
    }
    let mut heads: Vec<Head> = Vec::with_capacity(nsmp);
    for _ in 0..nsmp {
        let index = usize::from(r.u16be()?);
        let size = if version == 2 { (r.u32be()? as usize).saturating_sub(2) } else { struct_size };
        let mut c = Reader::new(r.bytes(size)?);
        let (path, name) = if version < 2 {
            let path = c.text(31)?;
            c.skip(1)?;
            (path, c.text(30)?)
        } else {
            let path = stp_cstring(&mut c, 257)?;
            c.skip(1)?;
            let name = stp_cstring(&mut c, 31)?;
            if c.pos % 2 == 1 {
                c.skip(1)?;
            }
            (path, name)
        };
        let len = c.u32be()?;
        let vol = c.u8()?;
        c.skip(1)?;
        let (ls, ll) = (c.u32be()?, c.u32be()?);
        c.skip(4)?;
        let ft = c.u8().unwrap_or(0) as i8;
        // Several loops a sample from version 1: the first is kept.
        let (mut ls, mut ll) = (ls, ll);
        if version >= 1 {
            let n = usize::from(r.u16be()?);
            let loops = r.bytes(n * 8)?;
            if n > 0 {
                (ls, ll) = (u32::from_be_bytes([loops[0], loops[1], loops[2], loops[3]]), u32::from_be_bytes([loops[4], loops[5], loops[6], loops[7]]));
            }
        }
        heads.push(Head { index, path, name, len, vol, ls, ll, ft: if version == 2 { ft } else { 0 } });
    }
    let npat_v0 = if version == 0 { usize::from(r.u16be()?) } else { 128 };
    let mut nch = 4;
    let mut raw: Vec<Option<(usize, usize, &[u8])>> = vec![None; 128];
    for i in 0..npat_v0 {
        let (index, chans) = if version > 0 {
            let index = r.u16be()?;
            if index == 0xFFFF {
                break;
            }
            rows = usize::from(r.u16be()?);
            (usize::from(index), usize::from(r.u16be()?))
        } else {
            (i, 4)
        };
        if chans == 0 || chans > 64 {
            break;
        }
        let Ok(cells) = r.bytes(rows * chans * 4) else { break };
        nch = nch.max(chans);
        if index >= raw.len() {
            raw.resize(index + 1, None);
        }
        raw[index] = Some((rows.max(1), chans, cells));
    }
    if version > 0 {
        while let Ok(n) = r.u16be() {
            if n == 0xFFFF {
                break;
            }
            r.skip(2)?;
            let len = r.u32be()? as usize;
            r.skip(len.min(r.left()))?;
        }
        r.skip((17 * 2).min(r.left()))?;
    }
    let most = heads.iter().map(|h| h.index).max().unwrap_or(0);
    let mut samples = vec![Sample::empty(); most];
    let mut by_index: Vec<(usize, usize)> = heads.iter().enumerate().map(|(i, h)| (h.index, i)).collect();
    by_index.sort_unstable();
    for (index, i) in by_index {
        let Head { ref path, ref name, len, vol, ls, ll, ft, .. } = heads[i];
        if index == 0 {
            continue;
        }
        let frames = len as usize;
        let pcm = decode(r.bytes_upto(frames), Pcm::S8, frames);
        let end = ls.saturating_add(ll).min(len);
        samples[index - 1] = Sample {
            name: name.clone(),
            filename: path.clone(),
            wave: Arc::new(Wave::new(8, 1, pcm)),
            // Version 2's finetune, in 16ths of a semitone.
            c5: super::common::relative_c5(0, ((ft as u8) << 3) as i8),
            volume: vol.min(64),
            looped: if ll > 0 && ls < end { Loop::forward(ls, end) } else { Loop::default() },
            ..Sample::empty()
        };
    }

    // Each order its own pattern, the auto effects carried on through them.
    let mut autos = vec![StpAuto::default(); nch];
    let mut patterns = Vec::with_capacity(nord);
    let mut orders = Vec::with_capacity(nord);
    for &o in order_list.iter().take(nord) {
        let Some(Some((rows, chans, cells))) = raw.get(usize::from(o)) else { continue };
        let mut p = Pattern::new(*rows, nch);
        for row in 0..*rows {
            // A fraction of a tick a row, as a fine pattern delay on some rows.
            let mut delay = match frac & 3 {
                1 => row & 3 == 0,
                2 => row & 1 == 0,
                3 => row & 3 != 3,
                _ => false,
            };
            for c in 0..*chans {
                let b = &cells[(row * chans + c) * 4..(row * chans + c) * 4 + 4];
                let cell = &mut p.cells[row * nch + c];
                cell.instr = b[0];
                if b[1] != 0 {
                    cell.note = (25 + u16::from(b[1])).min(u16::from(NOTE_MAX)) as u8;
                }
                let written = stp_effect(cell, b[2], b[3], &mut autos[c], &mut frac);
                autos[c].fill(cell, written);
                if delay && cell.fx == Fx::None {
                    (cell.fx, cell.param) = (Fx::S3mEx, 0x61);
                    delay = false;
                }
            }
        }
        orders.push(patterns.len() as u16);
        patterns.push(Arc::new(p));
    }
    let mut m = Module {
        format: "stp".into(),
        format_name: format!("SoundTracker Pro II (version {version})"),
        tracker: "SoundTracker Pro II".into(),
        kind: Kind::Mod,
        speed: speed.clamp(1, 255) as u8,
        tempo: u16::from(stp_tempo(timer)),
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels: (0..nch).map(|c| Channel { pan: amiga_pan(c), ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Pumatracker ----
//
// A 12-character title, counts, where each of ten samples is and how
// long; the orders (each order's four channels a track of their own,
// with a transpose for its instruments and one for its notes, and a
// speed); tracks of 32 rows run-length packed, each after "patt";
// instruments, a volume script after "inst" and a pitch script after
// "insf", which neumetik-synth plays (`neumetik_synth::puma`); the
// samples. Each order is a pattern here; the ten samples are the song's
// first, the instruments' synth voices after them.

const PUMA_HEADER: usize = 80;

fn puma_order_ok(e: &[u8]) -> bool {
    (0..4).all(|c| {
        let (pat, note_t) = (e[c * 3], e[c * 3 + 2] as i8);
        pat < 128 && note_t & 1 == 0 && (-48..=48).contains(&note_t)
    }) && e[12] <= 15
        && e[13] == 0
}

pub fn probe_puma(d: &[u8]) -> bool {
    if d.len() < PUMA_HEADER || d[..12].iter().any(|&c| c != 0 && c < b' ') {
        return false;
    }
    let be16 = |i: usize| usize::from(u16::from_be_bytes([d[i], d[i + 1]]));
    let (last, npat, nins, zero) = (be16(12), be16(14), be16(16), be16(18));
    if last > 255 || npat == 0 || npat > 128 || nins == 0 || nins > 32 || zero != 0 {
        return false;
    }
    let nord = last + 1;
    let least = PUMA_HEADER + nord * 14 + npat * 8 + 4 + nins * 16 + 4;
    for i in 0..10 {
        let at = u32::from_be_bytes([d[20 + i * 4], d[21 + i * 4], d[22 + i * 4], d[23 + i * 4]]) as usize;
        let len = be16(60 + i * 2);
        if (len > 0 && at == 0) || at > 0x10_0000 || (at > 0 && at < least) {
            return false;
        }
    }
    d.len() >= PUMA_HEADER + nord * 14 && d[PUMA_HEADER..PUMA_HEADER + nord * 14].chunks_exact(14).all(puma_order_ok)
}

/// A script's commands, to its end (a jump or a stop) or the next
/// instrument's mark.
fn puma_script(r: &mut Reader, volume: bool) -> Vec<[u8; 4]> {
    let mut out = Vec::new();
    while let Some(b) = r.peek(4) {
        if b == b"inst" || b == b"insf" {
            break;
        }
        let c = [b[0], b[1], b[2], b[3]];
        r.pos += 4;
        let known = matches!(c[0], 0xA0 | 0xB0 | 0xE0) || (c[0] == 0xC0 && volume) || (c[0] == 0xD0 && !volume);
        if !known {
            break;
        }
        out.push(c);
        if matches!(c[0], 0xB0 | 0xE0) {
            break;
        }
    }
    out
}

pub fn load_puma(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let title = r.text(12)?;
    let nord = usize::from(r.u16be()?) + 1;
    let npat = usize::from(r.u16be()?);
    let nins = usize::from(r.u16be()?);
    r.skip(2)?;
    let offsets: Vec<usize> = (0..10).map(|_| r.u32be().map(|v| v as usize)).collect::<Result<_>>()?;
    let lengths: Vec<usize> = (0..10).map(|_| r.u16be().map(|v| usize::from(v) * 2)).collect::<Result<_>>()?;
    let order_data = r.bytes(nord * 14)?.to_vec();
    // Tracks: four-byte entries (note, instrument and effect, parameter,
    // rows it lasts) to 32 rows.
    let mut tracks: Vec<[[u8; 3]; 32]> = Vec::with_capacity(npat);
    for _ in 0..npat {
        if r.bytes(4)? != b"patt" {
            return crate::io::bad("a track isn't where it should be");
        }
        let mut t = [[0u8; 3]; 32];
        let mut row = 0;
        while row < 32 {
            let e = r.bytes(4)?;
            if e[0] & 1 != 0 || e[3] == 0 || usize::from(e[3]) > 32 - row {
                return crate::io::bad("a track is damaged");
            }
            t[row] = [e[0], e[1], e[2]];
            row += usize::from(e[3]);
        }
        tracks.push(t);
    }
    // A last "patt", then the instruments.
    r.skip(4)?;
    let mut sounds = Vec::with_capacity(nins);
    for _ in 0..nins {
        if r.peek(4) == Some(b"inst") {
            r.skip(4)?;
        }
        let volume = puma_script(&mut r, true);
        if r.peek(4) == Some(b"insf") {
            r.skip(4)?;
        }
        let pitch = puma_script(&mut r, false);
        sounds.push(PumaSound { volume, pitch });
    }
    let mut samples: Vec<Sample> = (0..10)
        .map(|i| {
            let len = lengths[i];
            let pcm = if len > 0 { decode(d.get(offsets[i]..).unwrap_or(&[]), Pcm::S8, len) } else { Vec::new() };
            Sample { wave: Arc::new(Wave::new(8, 1, pcm)), ..Sample::empty() }
        })
        .collect();
    let sources: Vec<u16> = (0..10).collect();
    let programs: Vec<Sample> = sounds.into_iter().map(|s| Sample { program: Some(Arc::new(Program::puma(s, sources.clone(), &samples))), ..Sample::empty() }).collect();
    samples.extend(programs);
    let instruments: Vec<Instrument> = (0..nins).map(|i| Instrument::for_sample("", (11 + i) as u16)).collect();

    let mut patterns = Vec::with_capacity(nord);
    for e in order_data.chunks_exact(14) {
        let mut p = Pattern::new(32, 4);
        for c in 0..4 {
            let (track, ins_t, note_t) = (usize::from(e[c * 3]), e[c * 3 + 1] as i8, e[c * 3 + 2] as i8);
            let Some(t) = tracks.get(track) else { continue };
            // A slide goes on over the rows after it until a note.
            let mut auto: Option<(Fx, u8)> = None;
            for (row, &[n, ins, param]) in t.iter().enumerate() {
                let cell = &mut p.cells[row * 4 + c];
                if n != 0 {
                    let note = 12 + (i32::from(n) + i32::from(note_t)) / 2;
                    cell.note = note.clamp(i32::from(NOTE_MIN), i32::from(NOTE_MAX)) as u8;
                }
                if ins & 0x1F != 0 {
                    cell.instr = ((i32::from(ins & 0x1F) + i32::from(ins_t)) & 0x1F) as u8;
                }
                // A note with no instrument plays nothing.
                if cell.instr == 0 && cell.note != NOTE_NONE {
                    cell.note = NOTE_CUT;
                }
                let mut slide = false;
                match ins >> 5 {
                    1 => (cell.fx, cell.param) = (Fx::Volume, param.min(64)),
                    2 | 3 => {
                        let fx = if ins >> 5 == 2 { Fx::PortaDown } else { Fx::PortaUp };
                        auto = (param != 0).then_some((fx, param));
                        if let Some((f, v)) = auto {
                            (cell.fx, cell.param) = (f, v);
                        }
                        slide = true;
                    }
                    _ => {}
                }
                if !slide {
                    if cell.note != NOTE_NONE {
                        auto = None;
                    } else if let Some((f, v)) = auto.filter(|_| cell.fx == Fx::None) {
                        (cell.fx, cell.param) = (f, v);
                    }
                }
            }
        }
        if e[12] != 0 {
            if let Some(cell) = (0..4).find(|&c| p.cells[c].fx == Fx::None) {
                (p.cells[cell].fx, p.cells[cell].param) = (Fx::Speed, e[12]);
            }
        }
        patterns.push(Arc::new(p));
    }
    let mut m = Module {
        title,
        format: "puma".into(),
        format_name: "Pumatracker".into(),
        tracker: "Pumatracker".into(),
        kind: Kind::Mod,
        flags: Flags { instruments: true, ..Default::default() },
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        channels: (0..4).map(|c| Channel { pan: amiga_pan(c), ..Default::default() }).collect(),
        orders: (0..nord as u16).collect(),
        patterns,
        samples,
        instruments,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}
