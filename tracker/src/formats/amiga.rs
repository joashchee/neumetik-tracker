//! The Amiga's other trackers: Oktalyzer (.okt, eight channels by
//! splitting four), OctaMED (MMD0 to MMD3, and MED Packer's MMDC) and the MED before it (MED2
//! to MED4: MED 1.12 to 3.x), DigiBooster 1 (.digi) and
//! DigiBooster Pro (.dbm), SoundTracker 2.6 and Ice Tracker (MTN, IT10),
//! SoundFX (.sfx) and Asylum, the Crusader games' format (.amf).

use std::collections::HashMap;
use std::sync::Arc;

use super::common::{amiga_pan, finetune_c5, mod_effect, period_to_note, plausible_text};
use crate::io::{bad, decode, Pcm, Reader, Result};
use crate::model::*;

// ---- Oktalyzer ----

pub fn probe_okt(d: &[u8]) -> bool {
    d.len() >= 16 && &d[..8] == b"OKTASONG"
}

pub fn load_okt(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 8);
    let mut split = [false; 4];
    let mut heads: Vec<(String, usize, u32, u32, u8)> = Vec::new();
    let mut speed = 6;
    let mut slen = 0usize;
    let mut plen = 0usize;
    let mut ord = Vec::new();
    let mut bodies: Vec<&[u8]> = Vec::new();
    let mut sbods: Vec<&[u8]> = Vec::new();
    while r.left() >= 8 {
        let id = r.tag()?;
        let len = r.u32be()? as usize;
        let body = r.bytes_upto(len);
        let mut c = Reader::new(body);
        match &id {
            b"CMOD" => {
                for s in split.iter_mut() {
                    *s = c.u16be().unwrap_or(0) != 0;
                }
            }
            b"SAMP" => {
                while c.left() >= 32 {
                    let name = c.text(20)?;
                    let n = c.u32be()? as usize;
                    let ls = u32::from(c.u16be()?) * 2;
                    let ll = u32::from(c.u16be()?) * 2;
                    c.skip(1)?;
                    let vol = c.u8()?;
                    c.skip(2)?;
                    heads.push((name, n, ls, ll, vol));
                }
            }
            b"SPEE" => speed = c.u16be().unwrap_or(6).clamp(1, 255) as u8,
            b"SLEN" => slen = usize::from(c.u16be().unwrap_or(0)),
            b"PLEN" => plen = usize::from(c.u16be().unwrap_or(0)),
            b"PATT" => ord = body.to_vec(),
            b"PBOD" => bodies.push(body),
            b"SBOD" => sbods.push(body),
            _ => {}
        }
    }
    // Each of the four channels may be split in two.
    let mut chans = Vec::new();
    for (i, &s) in split.iter().enumerate() {
        chans.push(amiga_pan(i));
        if s {
            chans.push(amiga_pan(i));
        }
    }
    let nch = chans.len();
    let mut patterns = Vec::new();
    for body in bodies.iter().take(slen.max(bodies.len())) {
        let mut c = Reader::new(body);
        let rows = usize::from(c.u16be()?).clamp(1, 256);
        let mut p = Pattern::new(rows, nch);
        for i in 0..rows * nch {
            let Ok(b) = c.bytes(4) else { break };
            let mut cell = Cell::default();
            if b[0] > 0 {
                cell.note = b[0].saturating_add(48).min(NOTE_MAX);
                cell.instr = b[1].saturating_add(1);
            }
            let v = b[3];
            (cell.fx, cell.param) = match b[2] {
                1 => (Fx::PortaDown, v),
                2 => (Fx::PortaUp, v),
                10..=12 => (Fx::Arpeggio, v),
                13 => (Fx::NoteSlideDown, 0x10 | (v & 15)),
                17 | 30 => (Fx::NoteSlideUp, 0x10 | (v & 15)),
                21 => (Fx::NoteSlideDown, 0x10 | (v & 15)),
                25 => (Fx::PositionJump, v),
                27 => (Fx::KeyOff, 0),
                28 => (Fx::Speed, v.max(1)),
                31 => match v {
                    0..=0x40 => (Fx::Volume, v),
                    0x41..=0x50 => (Fx::VolSlide, v - 0x40),
                    0x51..=0x60 => (Fx::VolSlide, (v - 0x50) << 4),
                    0x61..=0x70 => (Fx::FineVolSlideDown, v - 0x60),
                    0x71..=0x80 => (Fx::FineVolSlideUp, v - 0x70),
                    _ => (Fx::None, 0),
                },
                _ => (Fx::None, 0),
            };
            p.cells[i] = cell;
        }
        patterns.push(Arc::new(p));
    }
    let mut data = sbods.into_iter();
    let samples = heads
        .into_iter()
        .map(|(name, n, ls, ll, vol)| {
            let pcm = if n > 0 { decode(data.next().unwrap_or(&[]), Pcm::S8, n) } else { Vec::new() };
            Sample { name, wave: Arc::new(Wave::new(8, 1, pcm)), volume: vol.min(64), looped: if ll > 2 { Loop::forward(ls, ls + ll) } else { Loop::default() }, ..Sample::empty() }
        })
        .collect();
    let mut m = Module {
        format: "okt".into(),
        format_name: "Oktalyzer".into(),
        tracker: "Oktalyzer".into(),
        kind: Kind::Mod,
        flags: Flags { amiga_limits: false, ..Default::default() },
        speed,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: chans.into_iter().map(|pan| Channel { pan, ..Default::default() }).collect(),
        orders: ord.iter().take(plen.max(1)).map(|&o| u16::from(o)).collect(),
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- OctaMED ----

pub fn probe_med(d: &[u8]) -> bool {
    d.len() >= 52 && (&d[..4] == b"MMD0" || &d[..4] == b"MMD1" || &d[..4] == b"MMD2" || &d[..4] == b"MMD3" || &d[..4] == b"MMDC")
}

/// How a MED song keeps time: its flags for OctaMED's 5-to-8-channel
/// mode, mixing (MED Soundstudio's 1-64 channels), and BPM mode (with its
/// lines a beat).
#[derive(Clone, Copy, Default)]
struct MedTiming {
    eight: bool,
    mixing: bool,
    bpm: Option<u16>,
}

/// MED's tempo as beats a minute (ticks a second are two fifths of it).
/// OctaMED's own players (proplayer and pro8player) time their ticks by
/// the PAL Amiga's CIA timer or, in 8-channel mode, by the mixing
/// buffer's length, and MED Soundstudio's mixing player by the same timer
/// (OpenMPT's Load_med.cpp, BSD, agrees). The ticks a line (the speed)
/// don't change it.
fn med_tempo(tempo: u16, t: MedTiming) -> u16 {
    const CIA: f64 = 709_379.0;
    let bpm = match t.bpm {
        // OctaMED 5 plays BPM tempos under 7 so.
        Some(_) if !t.eight && !t.mixing && tempo < 7 => 111.5,
        Some(lines) if !t.eight => f64::from(tempo) * f64::from(lines) / 4.0,
        // 8-channel mode: a tick is a mixing buffer (1, the shortest, to
        // 10; its high-quality sizes, in 4-byte steps) at Paula's period 124.
        _ if t.eight => {
            let size = [101.0, 110.0, 119.0, 128.0, 137.0, 146.0, 156.0, 165.0, 174.0, 183.0][usize::from(tempo.clamp(1, 10)) - 1];
            2.5 * 3_546_895.0 / (size * 4.0 * 124.0)
        }
        // The mixing player's timer, divided as the CIA's is below,
        // overflows under 8 and plays at this.
        _ if t.mixing && tempo < 8 => 157.86,
        // 1 to 10 stand for SoundTracker's speeds: the timer from a table.
        _ if !t.mixing && (1..=10).contains(&tempo) => 2.5 * CIA / f64::from([2417u16, 4833, 7250, 9666, 12083, 14500, 16916, 19332, 21436, 24163][usize::from(tempo) - 1]),
        _ => 2.5 * CIA / f64::from(470_000 / u32::from(tempo.max(1))),
    };
    (bpm.round() as u16).clamp(32, 999)
}

fn med_effect(cmd: u8, p: u8, vol_hex: bool, timing: MedTiming) -> (Fx, u8) {
    match cmd {
        0x00 => (if p != 0 { Fx::Arpeggio } else { Fx::None }, p),
        0x01 => (Fx::PortaUp, p),
        0x02 => (Fx::PortaDown, p),
        0x03 => (Fx::TonePorta, p),
        0x04 | 0x14 => (Fx::Vibrato, p),
        0x05 => (Fx::TonePortaVol, p),
        0x06 => (Fx::VibratoVol, p),
        0x07 => (Fx::Tremolo, p),
        0x09 => (Fx::Speed, (p & 0x1F).max(1)),
        0x0A | 0x0D => (Fx::VolSlide, p),
        0x0B => (Fx::PositionJump, p),
        0x0C => {
            let v = if vol_hex { p & 0x7F } else { (p >> 4) * 10 + (p & 15) };
            (Fx::Volume, v.min(64))
        }
        0x0F => match p {
            0 => (Fx::PatternBreak, 0),
            // OctaMED plays 1 and 2 at this tempo, whatever its mode.
            1 | 2 => (Fx::Tempo, 0x70),
            3..=0xF0 => (Fx::Tempo, med_tempo(u16::from(p), timing).min(255) as u8),
            0xF1 => (Fx::Retrig, 3),
            0xF2 => (Fx::ModEx, 0xD3),
            0xF3 => (Fx::Retrig, 2),
            0xFE => (Fx::PositionJump, 0xFF),
            0xFF => (Fx::KeyOff, 0),
            _ => (Fx::None, 0),
        },
        0x11 => (Fx::ModEx, 0x10 | (p & 15)),
        0x12 => (Fx::ModEx, 0x20 | (p & 15)),
        0x15 => (Fx::SetFinetune, p & 15),
        0x16 => (Fx::ModEx, 0x60 | (p & 15)),
        0x18 => (Fx::ModEx, 0xC0 | (p & 15)),
        0x19 => (Fx::Offset, p),
        0x1A => (Fx::ModEx, 0xA0 | (p & 15)),
        0x1B => (Fx::ModEx, 0xB0 | (p & 15)),
        0x1D => (Fx::PatternBreak, p),
        0x1E => (Fx::ModEx, 0xE0 | (p & 15)),
        0x1F => (Fx::ModEx, 0xD0 | (p >> 4)),
        _ => (Fx::None, 0),
    }
}

/// An OctaMED synth or hybrid sound (the MMD format's SynthInstr) at
/// `at`: its sequences and waveforms, and a hybrid's sample with its loop.
fn med_synth(d: &[u8], at: usize, hybrid: bool) -> Option<(SynthSound, Option<(Wave, Loop)>)> {
    let mut h = Reader::at(d, at + 10);
    let (rep, replen) = (u32::from(h.u16be().ok()?), u32::from(h.u16be().ok()?));
    let vol_len = usize::from(h.u16be().ok()?).min(128);
    let wave_len = usize::from(h.u16be().ok()?).min(128);
    let vol_speed = h.u8().ok()?;
    let wave_speed = h.u8().ok()?;
    let forms = usize::from(h.u16be().ok()?).min(64);
    let volumes = h.bytes(128).ok()?[..vol_len].to_vec();
    let waves = h.bytes(128).ok()?[..wave_len].to_vec();
    let mut waveforms = Vec::with_capacity(forms);
    let mut sample = None;
    for i in 0..forms {
        let off = h.u32be().ok()? as usize;
        let mut w = Reader::at(d, at.checked_add(off)?);
        if i == 0 && hybrid {
            // A sample's own header: its length, its type, its points.
            let len = (w.u32be().ok()? as usize).min(1 << 20);
            let _kind = w.i16be().ok()?;
            let pcm = decode(w.bytes_upto(len), Pcm::S8, len);
            let frames = pcm.len() as u32;
            let looped = if replen > 1 { Loop::forward(rep * 2, (rep + replen) * 2).clamped(frames) } else { Loop::default() };
            sample = Some((Wave::new(8, 1, pcm), looped));
            waveforms.push(Vec::new());
        } else {
            let words = usize::from(w.u16be().ok()?).min(64);
            waveforms.push(w.bytes(words * 2).ok()?.iter().map(|&b| b as i8).collect());
        }
    }
    (!waveforms.is_empty()).then_some((SynthSound { waveforms, volumes, waves, vol_speed, wave_speed, hybrid }, sample))
}

pub fn load_med(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let id = r.tag()?;
    // MMDC: MED Packer's, an MMD0 whose blocks are packed.
    let packed = id[3] == b'C';
    let version = if packed { 0 } else { id[3] - b'0' };
    r.seek(8)?;
    let song = r.u32be()? as usize;
    r.seek(16)?;
    let blockarr = r.u32be()? as usize;
    r.seek(24)?;
    let smplarr = r.u32be()? as usize;
    r.seek(32)?;
    let expdata = r.u32be()? as usize;
    let mut s = Reader::at(d, song);
    let mut ext = Vec::new();
    for _ in 0..63 {
        let rep = u32::from(s.u16be()?) * 2;
        let replen = u32::from(s.u16be()?) * 2;
        s.skip(2)?;
        let svol = s.u8()?;
        let strans = s.i8()?;
        ext.push((rep, replen, svol, strans));
    }
    let numblocks = usize::from(s.u16be()?);
    let songlen = usize::from(s.u16be()?);
    let mut block_order: Vec<u16> = Vec::new();
    let mut track_pans: Vec<i8> = Vec::new();
    if version <= 1 {
        let seq = s.bytes(256)?;
        block_order = seq[..songlen.min(256)].iter().map(|&b| u16::from(b)).collect();
    } else {
        let pseq_table = s.u32be()? as usize;
        let section_table = s.u32be()? as usize;
        let _trackvols = s.u32be()?;
        let numtracks = usize::from(s.u16be()?);
        let numpseqs = usize::from(s.u16be()?);
        let trackpans = s.u32be()? as usize;
        if trackpans > 0 {
            track_pans = d.get(trackpans..trackpans + numtracks.min(64)).map(|b| b.iter().map(|&v| v as i8).collect()).unwrap_or_default();
        }
        s.skip(4 + 2 + 2 + 1 + 1 + 2 + 1 + 223)?;
        let mut sec = Reader::at(d, section_table);
        for _ in 0..songlen {
            let Ok(pi) = sec.u16be() else { break };
            if usize::from(pi) >= numpseqs {
                continue;
            }
            let Ok(ptr) = Reader::at(d, pseq_table + usize::from(pi) * 4).u32be() else { continue };
            let mut ps = Reader::at(d, ptr as usize + 40);
            let Ok(len) = ps.u16be() else { continue };
            for _ in 0..len {
                let Ok(b) = ps.u16be() else { break };
                if b < 0x8000 {
                    block_order.push(b);
                }
            }
        }
    }
    let deftempo = s.u16be()?;
    let transpose = s.i8()?;
    let flags = s.u8()?;
    let flags2 = s.u8()?;
    let tempo2 = s.u8()?;
    s.skip(16)?;
    let _mastervol = s.u8()?;
    let numsamples = usize::from(s.u8()?);
    let vol_hex = flags & 0x10 != 0;
    let timing = MedTiming { eight: flags & 0x40 != 0, mixing: flags2 & 0x80 != 0, bpm: (flags2 & 0x20 != 0).then_some(u16::from(flags2 & 0x1F) + 1) };
    let (speed, tempo) = (tempo2.clamp(1, 32), med_tempo(deftempo, timing));

    // Instrument names, in the expansion.
    let mut names: Vec<String> = Vec::new();
    let mut finetunes: Vec<i8> = Vec::new();
    let mut song_name = String::new();
    let mut annotation = String::new();
    let mut colors: Vec<[u8; 3]> = Vec::new();
    if expdata > 0 {
        let mut e = Reader::at(d, expdata + 4);
        let exp_smp = e.u32be().unwrap_or(0) as usize;
        let s_entries = usize::from(e.u16be().unwrap_or(0));
        let s_size = usize::from(e.u16be().unwrap_or(0));
        let annotxt = e.u32be().unwrap_or(0) as usize;
        let annolen = e.u32be().unwrap_or(0) as usize;
        let iinfo = e.u32be().unwrap_or(0) as usize;
        let i_entries = usize::from(e.u16be().unwrap_or(0));
        let i_size = usize::from(e.u16be().unwrap_or(0));
        let _jumpmask = e.u32be().unwrap_or(0);
        let rgbtable = e.u32be().unwrap_or(0) as usize;
        let _ = e.skip(4 + 4);
        // The annotation (OctaMED's notepad), and the screen's eight colors
        // as OctaMED kept them with the song (0RGB words): clues for its cover.
        if annotxt > 0 && annolen > 1 {
            if let Some(b) = d.get(annotxt..annotxt + annolen.min(65_536)) {
                annotation = crate::io::message(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]);
            }
        }
        if rgbtable > 0 {
            if let Some(b) = d.get(rgbtable..rgbtable + 16) {
                let words: Vec<u16> = b.chunks(2).map(|w| u16::from_be_bytes([w[0], w[1]])).collect();
                if words.iter().all(|&w| w < 0x1000) && words.iter().any(|&w| w != 0) {
                    colors = words.iter().map(|&w| [((w >> 8) & 15) as u8 * 17, ((w >> 4) & 15) as u8 * 17, (w & 15) as u8 * 17]).collect();
                }
            }
        }
        let songname = e.u32be().unwrap_or(0) as usize;
        let songnamelen = e.u32be().unwrap_or(0) as usize;
        if iinfo > 0 && i_size >= 40 {
            for i in 0..i_entries.min(64) {
                names.push(d.get(iinfo + i * i_size..iinfo + i * i_size + 40).map(crate::io::text).unwrap_or_default());
            }
        }
        if exp_smp > 0 && s_size >= 4 {
            for i in 0..s_entries.min(64) {
                finetunes.push(d.get(exp_smp + i * s_size + 3).map_or(0, |&v| v as i8));
            }
        }
        if songname > 0 && songnamelen > 0 {
            song_name = d.get(songname..songname + songnamelen.min(256)).map(crate::io::text).unwrap_or_default();
        }
    }

    // Samples, and synth and hybrid sounds (a hybrid's waveform 0 is a sample).
    let mut samples = Vec::new();
    let mut sa = Reader::at(d, smplarr);
    for i in 0..numsamples.min(63) {
        let ptr = sa.u32be().unwrap_or(0) as usize;
        let mut smp = Sample { name: names.get(i).cloned().unwrap_or_default(), volume: ext[i].2.min(64), ..Sample::empty() };
        if let Some(&ft) = finetunes.get(i) {
            smp.c5 = finetune_c5(ft as u8);
        }
        if ptr > 0 {
            let mut h = Reader::at(d, ptr);
            let len = h.u32be().unwrap_or(0) as usize;
            let kind = h.i16be().unwrap_or(-1);
            if kind >= 0 {
                let b16 = kind & 0x10 != 0;
                let stereo = kind & 0x20 != 0;
                let bytes = h.bytes_upto(len);
                let points = len / if b16 { 2 } else { 1 };
                let mut pcm = decode(bytes, if b16 { Pcm::S16be } else { Pcm::S8 }, points);
                // A multi-octave instrument: its first octave.
                let ch = if stereo {
                    pcm = crate::io::interleave(&pcm);
                    2
                } else {
                    1
                };
                let (rep, replen) = (ext[i].0, ext[i].1);
                let div = if b16 { 2 } else { 1 };
                smp.wave = Arc::new(Wave::new(if b16 { 16 } else { 8 }, ch, pcm));
                if replen > 2 {
                    smp.looped = Loop::forward(rep / div, (rep + replen) / div);
                }
            } else if kind == -1 || kind == -2 {
                if let Some((sound, sample)) = med_synth(d, ptr, kind == -2) {
                    if let Some((wave, looped)) = sample {
                        smp.wave = Arc::new(wave);
                        smp.looped = looped;
                    }
                    smp.program = Some(Arc::new(Program::new(sound, &smp.wave)));
                }
            }
        }
        samples.push(smp);
    }

    // Blocks.
    let mut patterns = Vec::new();
    let mut nch = 4;
    let mut ba = Reader::at(d, blockarr);
    let mut blocks = Vec::new();
    for _ in 0..numblocks {
        let ptr = ba.u32be().unwrap_or(0) as usize;
        let unpacked: Vec<u8>;
        let mut b = Reader::at(d, ptr);
        let (tracks, lines) = if version == 0 { (usize::from(b.u8()?), usize::from(b.u8()?) + 1) } else { (usize::from(b.u16be()?), usize::from(b.u16be()?) + 1) };
        if version > 0 {
            b.skip(4)?;
        }
        let tracks = tracks.clamp(1, 64);
        nch = nch.max(tracks);
        if packed {
            // Runs of zeroes (a byte of 128 up, 256 less it), or bytes as
            // they are (a byte under 128, one more than it).
            let size = tracks * lines * 3;
            let mut out = vec![0u8; size];
            let mut j = 0;
            while j < size {
                let Ok(n) = b.u8() else { break };
                if n & 0x80 != 0 {
                    j += 256 - usize::from(n);
                } else {
                    let n = (usize::from(n) + 1).min(size - j);
                    let Ok(x) = b.bytes(n) else { break };
                    out[j..j + n].copy_from_slice(x);
                    j += n;
                }
            }
            unpacked = out;
            b = Reader::new(&unpacked);
        }
        let mut cells = vec![Cell::default(); lines.min(3200) * tracks];
        for (i, c) in cells.iter_mut().enumerate() {
            let (note, instr, cmd, p) = if version == 0 {
                let Ok(x) = b.bytes(3) else { break };
                (x[0] & 0x3F, x[1] >> 4 | (x[0] & 0x80) >> 3 | (x[0] & 0x40) >> 1, x[1] & 15, x[2])
            } else {
                let Ok(x) = b.bytes(4) else { break };
                (x[0] & 0x7F, x[1] & 0x3F, x[2] & 0x1F, x[3])
            };
            if note > 0 {
                let strans = if instr > 0 { ext.get(usize::from(instr) - 1).map_or(0, |e| e.3) } else { 0 };
                c.note = (i16::from(note) + 48 + i16::from(transpose) + i16::from(strans)).clamp(1, i16::from(NOTE_MAX)) as u8;
            }
            c.instr = instr;
            (c.fx, c.param) = med_effect(cmd, p, vol_hex, timing);
            let _ = i;
        }
        blocks.push((lines.min(3200), tracks, cells));
    }
    for (lines, tracks, cells) in blocks {
        let mut p = Pattern::new(lines, nch);
        for row in 0..lines {
            for t in 0..tracks {
                p.cells[row * nch + t] = cells[row * tracks + t];
            }
        }
        patterns.push(Arc::new(p));
    }
    if patterns.is_empty() {
        return bad("it has no blocks");
    }
    let pan = |i: usize| -> u8 {
        match track_pans.get(i) {
            Some(&v) if v != 0 => (128 + i32::from(v.clamp(-16, 16)) * 8).clamp(0, 255) as u8,
            _ => amiga_pan(i),
        }
    };
    let mut m = Module {
        title: song_name,
        format: "med".into(),
        format_name: if packed { "OctaMED (MMDC)".into() } else { format!("OctaMED (MMD{version})") },
        tracker: if packed { "MED Packer".into() } else if version >= 2 { "OctaMED Soundstudio".into() } else { "OctaMED".into() },
        kind: Kind::Mod,
        // Volume slides on every tick, unless it asks for SoundTracker's.
        flags: Flags { fast_slides: flags & 0x20 == 0, ..Default::default() },
        speed,
        tempo,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: (0..nch).map(|i| Channel { pan: pan(i), ..Default::default() }).collect(),
        orders: block_order,
        patterns,
        samples,
        message: annotation,
        palette: colors,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- MED 1.12 to 3.x (MED2, MED3, MED4) ----
//
// The MED before OctaMED's MMD: "MED" and a version byte. MED2 (MED
// 1.12): 32 instruments' names (40 bytes), volumes, loops, then the
// orders, tempo, flags, and 4-channel blocks of 64 rows in MOD's cells.
// MED3 (MED 2.00): names ended by a zero, the rest behind bit masks
// saying which instruments have them, and blocks packed by nibbles
// behind masks of the rows (and of the channels in each) that have
// notes and effects. MED4 (MED 2.10 to 3.x): up to 64 instruments
// behind a mask, each its flags, name, loop, volume and transpose, and
// blocks of up to 16 channels and 256 rows packed the same way, then
// the instruments: samples, synth and hybrid sounds ("MSH"), and an
// IFF-like tail (MEDV, ANNO). A song file (no "instruments attached"
// flag) keeps its instruments beside it, each a file named as the
// instrument. Described by libxmp's med2, med3 and med4 loaders (MIT).

pub fn probe_med_old(d: &[u8]) -> bool {
    d.len() >= 64 && &d[..3] == b"MED" && matches!(d[3], 2..=4)
}

/// The MED effects these versions have, as MMD's (the rest are dropped).
fn med_old_effect(cmd: u8, p: u8, version: u8, vol_hex: bool) -> (Fx, u8) {
    match cmd {
        0x0..=0x4 | 0xC | 0xD | 0xF => med_effect(cmd, p, vol_hex, MedTiming::default()),
        0xE if version == 2 => (Fx::VolSlide, p),
        0x9 if version == 4 && (1..=0x20).contains(&p) => (Fx::Speed, p),
        _ => (Fx::None, 0),
    }
}

/// An instrument kept beside a song: a sample file, or raw 8-bit data.
fn med_external(companion: super::Companion, name: &str) -> Option<Sample> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let file = name.rsplit([':', '/']).next().unwrap_or(name);
    let data = companion(file)?;
    if let Ok(l) = crate::samplefile::read(&data, file) {
        return l.samples.into_iter().next();
    }
    Some(Sample { wave: Arc::new(Wave::new(8, 1, decode(&data, Pcm::S8, data.len()))), ..Sample::empty() })
}

/// MED's nibbles, high first.
struct Nibbles<'a> {
    d: &'a [u8],
    at: usize,
}

impl Nibbles<'_> {
    fn get(&mut self, n: u32) -> u32 {
        let mut v = 0;
        for _ in 0..n {
            let b = self.d.get(self.at / 2).copied().unwrap_or(0);
            v = v << 4 | u32::from(if self.at % 2 == 0 { b >> 4 } else { b & 15 });
            self.at += 1;
        }
        v
    }
}

/// A packed block's notes and effects: `masks` the rows' line and effect
/// masks (32 rows each), `chn` its channels. Each cell (note, instrument,
/// effect, parameter).
fn med_unpack(packed: &[u8], masks: &[(u32, u32)], rows: usize, chn: usize, aligned: bool) -> Vec<[u8; 4]> {
    let mut out = vec![[0u8; 4]; rows * chn];
    let mut nb = Nibbles { d: packed, at: 0 };
    // MED3 reads a channel mask of chn/4 nibbles; MED4 one aligned to 4, 8, 12 or 16 bits.
    let mask_nibbles = if aligned { (chn.max(1) as u32).div_ceil(4) } else { (chn / 4) as u32 };
    let mask_bits = mask_nibbles * 4;
    for row in 0..rows {
        let (lm, fm) = masks[row / 32];
        let bit = 31 - (row % 32) as u32;
        if lm >> bit & 1 != 0 {
            let cm = nb.get(mask_nibbles);
            for ch in 0..chn {
                if (ch as u32) < mask_bits && cm >> (mask_bits - 1 - ch as u32) & 1 != 0 {
                    let x = nb.get(3);
                    let c = &mut out[row * chn + ch];
                    c[0] = (x >> 4) as u8;
                    c[1] = (x & 15) as u8;
                }
            }
        }
        if fm >> bit & 1 != 0 {
            let cm = nb.get(mask_nibbles);
            for ch in 0..chn {
                if (ch as u32) < mask_bits && cm >> (mask_bits - 1 - ch as u32) & 1 != 0 {
                    let x = nb.get(3);
                    let c = &mut out[row * chn + ch];
                    c[2] = (x >> 8) as u8;
                    c[3] = x as u8;
                }
            }
        }
    }
    out
}

/// MED4's synth or hybrid sound ("MSH"), its tables as long as they say.
fn med4_synth(d: &[u8], at: usize, hybrid: bool, rep: u32, replen: u32) -> Option<(SynthSound, Option<(Wave, Loop)>)> {
    let mut h = Reader::at(d, at);
    if h.tag().ok()? != *b"MSH\0" {
        return None;
    }
    h.skip(2 + 2 + 2 + 2 + 2).ok()?;
    let vol_len = usize::from(h.u16be().ok()?).min(128);
    let wave_len = usize::from(h.u16be().ok()?).min(128);
    let vol_speed = h.u8().ok()?;
    let wave_speed = h.u8().ok()?;
    let forms = usize::from(h.u16be().ok()?);
    if forms == 0 || forms > 64 {
        return None;
    }
    let volumes = h.bytes(vol_len).ok()?.to_vec();
    let waves = h.bytes(wave_len).ok()?.to_vec();
    let mut waveforms = Vec::with_capacity(forms);
    let mut sample = None;
    for i in 0..forms {
        let off = h.u32be().ok()? as usize;
        let mut w = Reader::at(d, at.checked_add(off)?);
        if i == 0 && hybrid {
            let len = (w.u32be().ok()? as usize).min(1 << 20);
            let _kind = w.i16be().ok()?;
            let pcm = decode(w.bytes_upto(len), Pcm::S8, len);
            let frames = pcm.len() as u32;
            let looped = if replen > 2 { Loop::forward(rep, rep + replen).clamped(frames) } else { Loop::default() };
            sample = Some((Wave::new(8, 1, pcm), looped));
            waveforms.push(Vec::new());
        } else {
            let words = usize::from(w.u16be().ok()?).min(64);
            waveforms.push(w.bytes(words * 2).ok()?.iter().map(|&b| b as i8).collect());
        }
    }
    Some((SynthSound { waveforms, volumes, waves, vol_speed, wave_speed, hybrid }, sample))
}

pub fn load_med_old(d: &[u8], companion: super::Companion) -> Result<Module> {
    let version = d[3];
    let mut r = Reader::at(d, 4);
    // Each instrument: name, volume, loop start and length (bytes), transpose.
    let mut inst: Vec<(String, u8, u32, u32, i8)> = Vec::new();
    let (npat, orders, tempo, transpose, flags, speed): (usize, Vec<u16>, u16, i8, u8, u8);
    match version {
        2 => {
            r.skip(40)?;
            for _ in 0..31 {
                inst.push((r.text(40)?, 0, 0, 0, 0));
            }
            r.skip(1)?;
            for i in inst.iter_mut() {
                i.1 = r.u8()?;
            }
            r.skip(2)?;
            for i in inst.iter_mut() {
                i.2 = u32::from(r.u16be()?);
            }
            r.skip(2)?;
            for i in inst.iter_mut() {
                i.3 = u32::from(r.u16be()?);
            }
            npat = usize::from(r.u16be()?);
            let raw = r.bytes(100)?.to_vec();
            let len = usize::from(r.u16be()?).min(100);
            orders = raw[..len].iter().map(|&o| u16::from(o)).collect();
            tempo = r.u16be()?;
            flags = r.u16be()? as u8;
            let _sliding = r.u16be()?;
            r.skip(4 + 16)?;
            transpose = 0;
            speed = 6;
        }
        3 => {
            for _ in 0..32 {
                let rest = &d[r.pos..];
                let n = rest.iter().take(40).position(|&b| b == 0).unwrap_or(40.min(rest.len()));
                let name = crate::io::text(&rest[..n]);
                r.skip((n + 1).min(rest.len()))?;
                inst.push((name, 0, 0, 0, 0));
            }
            let masked = |r: &mut Reader, f: &mut dyn FnMut(&mut Reader, usize) -> Result<()>| -> Result<()> {
                let mask = r.u32be()?;
                for i in 0..32 {
                    if mask << i & 0x8000_0000 != 0 {
                        f(r, i)?;
                    }
                }
                Ok(())
            };
            masked(&mut r, &mut |r, i| {
                inst[i].1 = r.u8()?;
                Ok(())
            })?;
            masked(&mut r, &mut |r, i| {
                inst[i].2 = u32::from(r.u16be()?);
                Ok(())
            })?;
            masked(&mut r, &mut |r, i| {
                inst[i].3 = u32::from(r.u16be()?);
                Ok(())
            })?;
            npat = usize::from(r.u16be()?);
            let len = usize::from(r.u16be()?);
            orders = r.bytes(len)?.iter().map(|&o| u16::from(o)).collect();
            tempo = r.u16be()?;
            transpose = r.i8()?;
            flags = r.u8()?;
            let _sliding = r.u16be()?;
            r.skip(4 + 16)?;
            masked(&mut r, &mut |r, _| r.skip(1))?;
            masked(&mut r, &mut |r, _| r.skip(1))?;
            speed = 6;
        }
        _ => {
            let m0 = r.u8()?;
            let mut mask: u64 = 0;
            let mut size = 0;
            for i in 0..8 {
                if m0 << i & 0x80 != 0 {
                    mask = mask << 8 | u64::from(r.u8()?);
                    size += 1;
                }
            }
            if size > 0 {
                mask <<= 8 * (8 - size);
            }
            for i in 0..64 {
                if mask == 0 {
                    break;
                }
                let on = mask & (1 << 63) != 0;
                mask <<= 1;
                while inst.len() <= i {
                    inst.push((String::new(), 64, 0, 0, 0));
                }
                if !on {
                    continue;
                }
                let c = r.u8()?;
                let n = usize::from(r.u8()?);
                inst[i].0 = crate::io::text(r.bytes(n)?);
                let mut len = 0;
                if c & 1 == 0 {
                    inst[i].2 = u32::from(r.u16be()?) * 2;
                }
                if c & 2 == 0 {
                    len = u32::from(r.u16be()?) * 2;
                }
                if c & 4 == 0 {
                    r.skip(1)?;
                }
                if c & 8 == 0 {
                    r.skip(1)?;
                }
                if c & 0x30 == 0 {
                    inst[i].1 = r.u8()?;
                }
                if c & 0x40 == 0 {
                    inst[i].4 = r.i8()?;
                }
                inst[i].3 = len;
            }
            npat = usize::from(r.u16be()?);
            let len = usize::from(r.u16be()?);
            orders = r.bytes(len)?.iter().map(|&o| u16::from(o)).collect();
            tempo = r.u16be()?;
            transpose = r.i8()?;
            flags = r.u8()?;
            let spd = r.u16be()?;
            speed = if d.windows(8).any(|w| w == b"MEDV\0\0\0\x04") { spd.clamp(1, 32) as u8 } else if flags & 0x20 != 0 { 5 } else { 6 };
            r.skip(20 + 16 + 1)?;
        }
    }
    if npat == 0 || npat > 256 {
        return bad("it says it has no blocks");
    }
    let vol_hex = version == 4 && flags & 0x10 != 0;
    let (speed, tempo_bpm) = (speed.max(1), med_tempo(tempo, MedTiming::default()));

    // Blocks.
    let mut blocks: Vec<(usize, usize, Vec<[u8; 4]>)> = Vec::new();
    for _ in 0..npat {
        match version {
            2 => {
                r.skip(4)?;
                let mut cells = Vec::with_capacity(256);
                for _ in 0..256 {
                    let period = r.u16be()?;
                    let x = r.u8()?;
                    let p = r.u8()?;
                    // The note as MMD's (1 is C-1 there, 48 below ours).
                    let note = period_to_note(period).map_or(0, |n| n.saturating_sub(48));
                    cells.push([note, x >> 4, x & 15, p]);
                }
                blocks.push((64, 4, cells));
            }
            3 => {
                let _tracks = r.u8()?;
                let b = r.u8()?;
                let convsz = usize::from(r.u16be()?);
                let mut m = [0u32; 4];
                for (i, (zero, full)) in [(0x10, 0x01), (0x20, 0x02), (0x40, 0x04), (0x80, 0x08)].into_iter().enumerate() {
                    m[i] = if b & zero != 0 {
                        0
                    } else if b & full != 0 {
                        u32::MAX
                    } else {
                        r.u32be()?
                    };
                }
                let packed = r.bytes_upto(convsz);
                let mut cells = med_unpack(packed, &[(m[0], m[2]), (m[1], m[3])], 64, 4, false);
                for c in cells.iter_mut() {
                    if c[1] > 0 {
                        c[1] += 1;
                    }
                }
                blocks.push((64, 4, cells));
            }
            _ => {
                let size = usize::from(r.u8()?);
                let start = r.pos;
                let chn = usize::from(r.u8()?);
                let rows = usize::from(r.u8()?) + 1;
                let plen = usize::from(r.u16be()?);
                if chn == 0 || chn > 16 {
                    return bad("a block has too many channels");
                }
                let mut ctl = [0u8; 4];
                for (j, c) in ctl.iter_mut().enumerate() {
                    if rows > j * 64 {
                        *c = r.u8()?;
                    }
                }
                let mut masks = Vec::new();
                for y in 0..8 {
                    if rows <= y * 32 {
                        break;
                    }
                    let c = ctl[y / 2];
                    let s = 4 * (y % 2);
                    let lm = if c & (0x80 >> s) != 0 {
                        u32::MAX
                    } else if c & (0x40 >> s) != 0 {
                        0
                    } else {
                        r.u32be()?
                    };
                    let fm = if c & (0x20 >> s) != 0 {
                        u32::MAX
                    } else if c & (0x10 >> s) != 0 {
                        0
                    } else {
                        r.u32be()?
                    };
                    masks.push((lm, fm));
                }
                r.seek(start + size)?;
                let packed = r.bytes_upto(plen);
                blocks.push((rows, chn, med_unpack(packed, &masks, rows, chn, true)));
                r.seek((start + size + plen).min(d.len()))?;
            }
        }
    }

    // Instruments: in the file (a module), or beside it (a song).
    let attached = version >= 3 && flags & 0x08 != 0;
    let mut samples: Vec<Sample> = inst
        .iter()
        .map(|(name, vol, ls, ll, _)| Sample {
            name: name.clone(),
            volume: (*vol).min(64),
            looped: if *ll > 2 { Loop::forward(*ls, ls + ll) } else { Loop::default() },
            ..Sample::empty()
        })
        .collect();
    if attached {
        let (count, mut mask) = if version == 3 {
            (32, u64::from(r.u32be()?) << 32)
        } else {
            let hi = r.u32be()?;
            let lo = r.u32be()?;
            (64, (u64::from(hi) << 32 | u64::from(lo)) << 1)
        };
        for i in 0..count {
            if mask == 0 {
                break;
            }
            let on = mask & (1 << 63) != 0;
            mask <<= 1;
            if !on {
                continue;
            }
            let start = r.pos;
            let Ok(len) = r.u32be() else { break };
            let Ok(kind) = r.i16be() else { break };
            let len = len as usize;
            while samples.len() <= i {
                samples.push(Sample::empty());
            }
            let s = &mut samples[i];
            match kind {
                0 => s.wave = Arc::new(Wave::new(8, 1, decode(r.bytes_upto(len), Pcm::S8, len))),
                -1 | -2 if version == 4 => {
                    let (rep, replen) = (s.looped.start, s.looped.end - s.looped.start);
                    if let Some((sound, sample)) = med4_synth(d, r.pos, kind == -2, rep, replen) {
                        if let Some((wave, looped)) = sample {
                            s.wave = Arc::new(wave);
                            s.looped = looped;
                        }
                        s.program = Some(Arc::new(Program::new(sound, &s.wave)));
                    }
                    r.seek((start + 6 + len).min(d.len()))?;
                }
                _ => r.seek((start + 6 + len).min(d.len()))?,
            }
        }
    } else {
        for s in samples.iter_mut() {
            if let Some(x) = med_external(companion, &s.name) {
                s.wave = x.wave;
                if !s.looped.on() {
                    s.looped = x.looped;
                }
            }
        }
    }
    // MED4's IFF-like tail: the annotation.
    let mut message = String::new();
    if version == 4 {
        let mut t = r.clone();
        while t.left() >= 8 {
            let (Ok(id), Ok(n)) = (t.tag(), t.u32be()) else { break };
            let Ok(body) = t.bytes((n as usize).min(t.left())) else { break };
            if &id == b"ANNO" {
                message = crate::io::message(body);
            }
        }
    }

    let nch = blocks.iter().map(|b| b.1).max().unwrap_or(4).max(4);
    let patterns = blocks
        .into_iter()
        .map(|(rows, chn, cells)| {
            let mut p = Pattern::new(rows, nch);
            for row in 0..rows {
                for ch in 0..chn {
                    let [note, ins, cmd, prm] = cells[row * chn + ch];
                    let c = &mut p.cells[row * nch + ch];
                    if note > 0 {
                        let t = if ins > 0 { inst.get(usize::from(ins) - 1).map_or(0, |i| i.4) } else { 0 };
                        c.note = (i16::from(note) + 48 + i16::from(transpose) + i16::from(t)).clamp(1, i16::from(NOTE_MAX)) as u8;
                    }
                    c.instr = ins;
                    (c.fx, c.param) = med_old_effect(cmd, prm, version, vol_hex);
                }
            }
            Arc::new(p)
        })
        .collect();
    let mut m = Module {
        message,
        format: "med".into(),
        format_name: format!("MED (MED{version})"),
        tracker: match version {
            2 => "MED 1.12".into(),
            3 => "MED 2.00".into(),
            _ => "MED 2.10 to 3.x".into(),
        },
        kind: Kind::Mod,
        speed,
        tempo: tempo_bpm,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: (0..nch).map(|i| Channel { pan: amiga_pan(i), ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- DigiBooster 1 ----

pub fn probe_digi(d: &[u8]) -> bool {
    d.len() >= 1572 && &d[..20] == b"DIGI Booster module\0"
}

pub fn load_digi(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 24);
    let _version = r.u8()?;
    let nch = usize::from(r.u8()?).clamp(1, 8);
    let packed = r.u8()? != 0;
    r.seek(46)?;
    let npat = usize::from(r.u8()?) + 1;
    let nord = usize::from(r.u8()?) + 1;
    let orders: Vec<u16> = r.bytes(128)?[..nord.min(128)].iter().map(|&o| u16::from(o)).collect();
    let mut lens = [0usize; 31];
    let mut ls = [0u32; 31];
    let mut ll = [0u32; 31];
    for v in &mut lens {
        *v = r.u32be()? as usize;
    }
    for v in &mut ls {
        *v = r.u32be()?;
    }
    for v in &mut ll {
        *v = r.u32be()?;
    }
    let vols = r.bytes(31)?.to_vec();
    let fts = r.bytes(31)?.to_vec();
    let title = r.text(32)?;
    let mut names = Vec::new();
    for _ in 0..31 {
        names.push(r.text(30)?);
    }
    let cell = |b: &[u8]| -> Cell {
        let period = u16::from(b[0] & 0x0F) << 8 | u16::from(b[1]);
        let (fx, param) = mod_effect(b[2] & 0x0F, b[3], false);
        Cell { note: period_to_note(period).unwrap_or(0), instr: (b[0] & 0xF0) | (b[2] >> 4), fx, param, ..Default::default() }
    };
    let mut patterns = Vec::new();
    for _ in 0..npat {
        let mut p = Pattern::new(64, nch);
        if packed {
            let size = usize::from(r.u16be()?);
            let body = r.bytes(size)?;
            let mut b = Reader::new(body);
            let mask = b.bytes(64)?.to_vec();
            for row in 0..64 {
                for c in 0..nch {
                    if mask[row] & (0x80 >> c) != 0 {
                        let Ok(x) = b.bytes(4) else { break };
                        p.cells[row * nch + c] = cell(x);
                    }
                }
            }
        } else {
            // Stored a channel at a time.
            let body = r.bytes(64 * nch * 4)?;
            for c in 0..nch {
                for row in 0..64 {
                    let at = (c * 64 + row) * 4;
                    p.cells[row * nch + c] = cell(&body[at..at + 4]);
                }
            }
        }
        patterns.push(Arc::new(p));
    }
    let mut samples = Vec::new();
    for i in 0..31 {
        let pcm = decode(r.bytes_upto(lens[i]), Pcm::S8, lens[i]);
        samples.push(Sample {
            name: names[i].clone(),
            wave: Arc::new(Wave::new(8, 1, pcm)),
            c5: finetune_c5(fts[i]),
            volume: vols[i].min(64),
            looped: if ll[i] > 2 { Loop::forward(ls[i], ls[i] + ll[i]) } else { Loop::default() },
            ..Sample::empty()
        });
    }
    let mut m = Module {
        title,
        format: "digi".into(),
        format_name: "DigiBooster".into(),
        tracker: "DigiBooster 1".into(),
        kind: Kind::Mod,
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: (0..nch).map(|i| Channel { pan: amiga_pan(i), ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- DigiBooster Pro ----

pub fn probe_dbm(d: &[u8]) -> bool {
    d.len() >= 16 && &d[..4] == b"DBM0"
}

pub fn load_dbm(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 8);
    let mut m = Module { format: "dbm".into(), format_name: "DigiBooster Pro".into(), tracker: format!("DigiBooster Pro {}.{:02x}", d[4], d[5]), kind: Kind::Xm, ..Default::default() };
    let (mut ninst, mut nsmp, mut npat, mut nch) = (0usize, 0usize, 0usize, 4usize);
    let mut insts: Vec<(String, u16, u8, u32, u32, u32, i16, u16)> = Vec::new();
    let mut wave_list: Vec<Option<Arc<Wave>>> = Vec::new();
    let mut envs: Vec<(u16, bool, Envelope)> = Vec::new();
    while r.left() >= 8 {
        let id = r.tag()?;
        let len = r.u32be()? as usize;
        let body = r.bytes_upto(len);
        let mut c = Reader::new(body);
        match &id {
            b"NAME" => m.title = crate::io::text(body),
            b"INFO" => {
                ninst = usize::from(c.u16be()?);
                nsmp = usize::from(c.u16be()?);
                let _songs = c.u16be()?;
                npat = usize::from(c.u16be()?);
                nch = usize::from(c.u16be()?).clamp(1, 128);
            }
            b"SONG" => {
                if m.orders.is_empty() {
                    c.skip(44)?;
                    let n = usize::from(c.u16be()?);
                    for _ in 0..n {
                        m.orders.push(c.u16be()?);
                    }
                }
            }
            b"INST" => {
                for _ in 0..ninst {
                    let name = c.text(30)?;
                    let smp = c.u16be()?;
                    let vol = c.u16be()?.min(64) as u8;
                    let rate = c.u32be()?;
                    let ls = c.u32be()?;
                    let ll = c.u32be()?;
                    let pan = c.i16be()?;
                    let flags = c.u16be()?;
                    insts.push((name, smp, vol, rate, ls, ll, pan, flags));
                }
            }
            b"PATT" => {
                for _ in 0..npat {
                    let rows = usize::from(c.u16be()?).clamp(1, 1024);
                    let size = c.u32be()? as usize;
                    let data = c.bytes(size)?;
                    let mut p = Reader::new(data);
                    let mut pat = Pattern::new(rows, nch);
                    let mut row = 0;
                    while row < rows && !p.eof() {
                        let ch = p.u8()?;
                        if ch == 0 {
                            row += 1;
                            continue;
                        }
                        let mask = p.u8()?;
                        let mut cell = Cell::default();
                        if mask & 1 != 0 {
                            let n = p.u8()?;
                            cell.note = if n == 0x1F { NOTE_OFF } else if n > 0 { ((n >> 4) * 12 + (n & 15) + 1 + 12).min(NOTE_MAX) } else { 0 };
                        }
                        if mask & 2 != 0 {
                            cell.instr = p.u8()?;
                        }
                        let mut fx = [(0u8, 0u8); 2];
                        if mask & 4 != 0 {
                            fx[0].0 = p.u8()?;
                        }
                        if mask & 8 != 0 {
                            fx[0].1 = p.u8()?;
                        }
                        if mask & 16 != 0 {
                            fx[1].0 = p.u8()?;
                        }
                        if mask & 32 != 0 {
                            fx[1].1 = p.u8()?;
                        }
                        for (cmd, param) in fx {
                            let (f, v) = mod_effect(cmd, param, true);
                            if f == Fx::Volume && cell.vol == Vol::None {
                                (cell.vol, cell.volp) = (Vol::Volume, v);
                            } else if f != Fx::None && cell.fx == Fx::None {
                                (cell.fx, cell.param) = (f, v);
                            }
                        }
                        if let Some(s) = pat.cell_mut(row, usize::from(ch) - 1) {
                            *s = cell;
                        }
                    }
                    m.patterns.push(Arc::new(pat));
                }
            }
            b"SMPL" => {
                // Files cut short lose the samples at the end, not the song.
                for _ in 0..nsmp {
                    let (Ok(flags), Ok(frames)) = (c.u32be(), c.u32be()) else { break };
                    let frames = frames as usize;
                    let w = match flags & 7 {
                        1 => Some(Wave::new(8, 1, decode(c.bytes_upto(frames), Pcm::S8, frames))),
                        2 => Some(Wave::new(16, 1, decode(c.bytes_upto(frames * 2), Pcm::S16be, frames))),
                        4 => {
                            let b = c.bytes_upto(frames * 4);
                            Some(Wave::new(16, 1, b.chunks_exact(4).map(|x| i16::from_be_bytes([x[0], x[1]])).collect()))
                        }
                        _ => None,
                    };
                    wave_list.push(w.map(Arc::new));
                }
            }
            b"VENV" | b"PENV" => {
                let n = usize::from(c.u16be().unwrap_or(0));
                for _ in 0..n {
                    let Ok(ins) = c.u16be() else { break };
                    let flags = c.u8()?;
                    let count = c.u8()?.min(32);
                    let sus_a = c.u8()?;
                    let ls = c.u8()?;
                    let le = c.u8()?;
                    let _sus_b = c.u8()?;
                    let mut points = Vec::new();
                    for i in 0..32 {
                        let t = c.u16be()?;
                        let v = c.i16be()?;
                        if i < count {
                            points.push(EnvPoint { tick: t, value: if &id == b"PENV" { (v.clamp(-128, 128) / 4) as i8 } else { v.clamp(0, 64) as i8 } });
                        }
                    }
                    let env = Envelope {
                        enabled: flags & 1 != 0 && count > 0,
                        sustain: (flags & 2 != 0 && sus_a < count).then_some((sus_a, sus_a)),
                        looped: (flags & 4 != 0 && ls <= le && le < count).then_some((ls, le)),
                        points,
                        ..Default::default()
                    };
                    envs.push((ins, &id == b"PENV", env));
                }
            }
            _ => {}
        }
    }
    // DBM's instruments each play one sample; they're instruments here,
    // for their envelopes.
    for (i, (name, smp, vol, rate, ls, ll, pan, flags)) in insts.into_iter().enumerate() {
        let wave = wave_list.get(usize::from(smp).wrapping_sub(1)).cloned().flatten().unwrap_or_default();
        let kind = if flags & 2 != 0 { LoopKind::PingPong } else if flags & 1 != 0 { LoopKind::Forward } else { LoopKind::Off };
        m.samples.push(Sample {
            name: name.clone(),
            wave,
            c5: (f64::from(rate.max(1000)) * 2.0f64.powf(0.0)) as u32,
            volume: vol,
            pan: Some((i32::from(pan.clamp(-128, 128)) + 128).min(255) as u8),
            looped: Loop { kind, start: ls, end: ls + ll },
            ..Sample::empty()
        });
        let mut ins = Instrument::for_sample(&name, (i + 1) as u16);
        for (n, e) in envs.iter().filter(|e| usize::from(e.0) == i + 1).map(|e| (e.1, e.2.clone())) {
            if n {
                ins.pan_env = e;
            } else {
                ins.volume_env = e;
            }
        }
        m.instruments.push(ins);
    }
    m.flags = Flags { linear: true, instruments: true, ..Default::default() };
    m.channels = (0..nch).map(|_| Channel::default()).collect();
    m.speed = 6;
    m.tempo = 125;
    m.global_volume = 128;
    m.mix_volume = 32;
    m.separation = 128;
    m.tidy();
    Ok(m)
}

// ---- SoundTracker 2.6 and Ice Tracker ----

pub fn probe_mtn(d: &[u8]) -> bool {
    d.len() >= 1468 && (&d[1464..1468] == b"MTN\0" || &d[1464..1468] == b"IT10")
}

pub fn load_mtn(d: &[u8]) -> Result<Module> {
    let mut r = Reader::new(d);
    let title = r.text(20)?;
    let mut heads = Vec::new();
    for _ in 0..31 {
        let name = r.text(22)?;
        let len = usize::from(r.u16be()?) * 2;
        let ft = r.u8()?;
        let vol = r.u8()?;
        let ls = u32::from(r.u16be()?) * 2;
        let ll = u32::from(r.u16be()?) * 2;
        heads.push((name, len, ft, vol, ls, ll));
    }
    let songlen = usize::from(r.u8()?).clamp(1, 128);
    let ntracks = usize::from(r.u8()?);
    let table = r.bytes(512)?.to_vec();
    r.seek(1468)?;
    let tracks = r.bytes(ntracks * 256)?;
    // A pattern for each different set of four tracks, and the swing
    // it's entered with: F is always speed, its high nibble taken when
    // set, and both nibbles set (and different) alternate the two a row
    // each until the next F. 5 to 9 aren't SoundTracker 2.6's, and its E
    // only sets the filter.
    let mut seen: HashMap<([u8; 4], u8), (u16, u8)> = HashMap::new();
    let mut patterns = Vec::new();
    let mut orders = Vec::new();
    let mut swing = 0u8;
    for o in 0..songlen {
        let key = [table[o * 4], table[o * 4 + 1], table[o * 4 + 2], table[o * 4 + 3]];
        let (idx, after) = *seen.entry((key, swing)).or_insert_with(|| {
            let mut p = Pattern::new(64, 4);
            let mut s = swing;
            for row in 0..64 {
                for (c, &t) in key.iter().enumerate() {
                    let t = usize::from(t);
                    if t >= ntracks {
                        continue;
                    }
                    // MOD's cells: a period, a sample, an effect.
                    let b = &tracks[t * 256 + row * 4..t * 256 + row * 4 + 4];
                    let period = u16::from(b[0] & 0x0F) << 8 | u16::from(b[1]);
                    let (cmd, arg) = (b[2] & 15, b[3]);
                    let (fx, param) = match cmd {
                        5..=9 => (Fx::None, 0),
                        0xE if arg >= 0x10 => (Fx::None, 0),
                        0xF if arg != 0 => {
                            let (hi, lo) = (arg >> 4, arg & 15);
                            s = if hi != 0 && lo != 0 && hi != lo { arg } else { 0 };
                            (Fx::Speed, if hi != 0 { hi } else { lo })
                        }
                        _ => mod_effect(cmd, arg, false),
                    };
                    p.cells[row * 4 + c] = Cell { note: period_to_note(period).unwrap_or(0), instr: (b[0] & 0xF0) | (b[2] >> 4), fx, param, ..Default::default() };
                }
                if s != 0 {
                    s = s.rotate_right(4);
                    let cells = &mut p.cells[row * 4..row * 4 + 4];
                    if let Some(c) = cells.iter().position(|c| c.fx == Fx::Speed).or_else(|| cells.iter().position(|c| c.fx == Fx::None)) {
                        (cells[c].fx, cells[c].param) = (Fx::Speed, s & 15);
                    }
                }
            }
            patterns.push(Arc::new(p));
            ((patterns.len() - 1) as u16, s)
        });
        swing = after;
        orders.push(idx);
    }
    let samples = heads
        .into_iter()
        .map(|(name, len, ft, vol, ls, ll)| Sample {
            name,
            wave: Arc::new(Wave::new(8, 1, decode(r.bytes_upto(len), Pcm::S8, len))),
            c5: finetune_c5(ft),
            volume: vol.min(64),
            looped: if ll > 2 { Loop::forward(ls, ls + ll) } else { Loop::default() },
            ..Sample::empty()
        })
        .collect();
    let ice = &d[1464..1468] == b"IT10";
    let mut m = Module {
        title,
        format: "mtn".into(),
        format_name: if ice { "Ice Tracker".into() } else { "SoundTracker 2.6".into() },
        tracker: if ice { "Ice Tracker".into() } else { "SoundTracker 2.6".into() },
        kind: Kind::Mod,
        flags: Flags { amiga_limits: true, ..Default::default() },
        speed: 6,
        tempo: 125,
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: (0..4).map(|i| Channel { pan: amiga_pan(i), ..Default::default() }).collect(),
        orders,
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- SoundFX ----

pub fn probe_sfx(d: &[u8]) -> bool {
    (d.len() >= 64 && &d[60..64] == b"SONG") || (d.len() >= 128 && &d[124..128] == b"SO31")
}

pub fn load_sfx(d: &[u8]) -> Result<Module> {
    let count = if d.len() >= 128 && &d[124..128] == b"SO31" { 31 } else { 15 };
    let mut r = Reader::new(d);
    let mut lens = Vec::new();
    for _ in 0..count {
        lens.push(r.u32be()? as usize);
    }
    r.skip(4)?;
    let delay = r.u16be()?;
    r.skip(14)?;
    let mut heads = Vec::new();
    for i in 0..count {
        let name = r.text(22)?;
        let _len = r.u16be()?;
        let ft = r.u8()?;
        let vol = r.u8()?;
        let ls = u32::from(r.u16be()?);
        let ll = u32::from(r.u16be()?) * 2;
        heads.push((name, lens[i], ft, vol, ls, ll));
    }
    let songlen = usize::from(r.u8()?).clamp(1, 128);
    let _restart = r.u8()?;
    let ord = r.bytes(128)?.to_vec();
    r.skip(if count == 31 { 4 } else { 0 })?;
    let npat = usize::from(ord[..songlen].iter().copied().max().unwrap_or(0)) + 1;
    let mut patterns = Vec::new();
    for _ in 0..npat {
        let body = r.bytes(1024)?;
        let mut p = Pattern::new(64, 4);
        for (i, b) in body.chunks_exact(4).enumerate() {
            let period = u16::from_be_bytes([b[0], b[1]]);
            let mut c = Cell::default();
            if period == 0xFFFD || period == 0xFFFE {
                c.note = NOTE_CUT;
            } else if period < 0x1000 {
                c.note = period_to_note(period & 0x0FFF).unwrap_or(0);
                c.instr = (b[0] & 0x10) | (b[2] >> 4);
                let v = b[3];
                (c.fx, c.param) = match b[2] & 15 {
                    1 => (Fx::Arpeggio, v),
                    2 if v >> 4 != 0 => (Fx::PortaDown, v >> 4),
                    2 => (Fx::PortaUp, v & 15),
                    5 => (Fx::FineVolSlideUp, v),
                    6 => (Fx::FineVolSlideDown, v),
                    7 => (Fx::PortaUp, v),
                    8 => (Fx::PortaDown, v),
                    _ => (Fx::None, 0),
                };
            }
            p.cells[i] = c;
        }
        patterns.push(Arc::new(p));
    }
    let samples = heads
        .into_iter()
        .map(|(name, len, ft, vol, ls, ll)| Sample {
            name,
            wave: Arc::new(Wave::new(8, 1, decode(r.bytes_upto(len), Pcm::S8, len))),
            c5: finetune_c5(ft),
            volume: vol.min(64),
            looped: if ll > 2 { Loop::forward(ls, ls + ll) } else { Loop::default() },
            ..Sample::empty()
        })
        .collect();
    let mut m = Module {
        format: "sfx".into(),
        format_name: "SoundFX".into(),
        tracker: if count == 31 { "SoundFX 2".into() } else { "SoundFX".into() },
        kind: Kind::Mod,
        flags: Flags { amiga_limits: true, ..Default::default() },
        speed: 6,
        // The CIA delay as a tempo.
        tempo: if delay > 0 { (14565u32 * 122 / u32::from(delay)).clamp(32, 999) as u16 } else { 125 },
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: (0..4).map(|i| Channel { pan: amiga_pan(i), ..Default::default() }).collect(),
        orders: ord[..songlen].iter().map(|&o| u16::from(o)).collect(),
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

// ---- Asylum ----

pub fn probe_asylum(d: &[u8]) -> bool {
    d.len() >= 294 + 64 * 37 && d.starts_with(b"ASYLUM Music Format V1.0\0")
}

pub fn load_asylum(d: &[u8]) -> Result<Module> {
    let mut r = Reader::at(d, 32);
    let speed = r.u8()?;
    let tempo = r.u8()?;
    let nsmp = usize::from(r.u8()?);
    let npat = usize::from(r.u8()?);
    let nord = usize::from(r.u8()?);
    let restart = r.u8()?;
    let ord = r.bytes(256)?.to_vec();
    let mut heads = Vec::new();
    for _ in 0..64 {
        let name = r.text(22)?;
        let ft = r.u8()?;
        let vol = r.u8()?;
        let transpose = r.i8()?;
        let len = r.u32le()? as usize;
        let ls = r.u32le()?;
        let ll = r.u32le()?;
        heads.push((name, ft, vol, transpose, len, ls, ll));
    }
    let mut patterns = Vec::new();
    for _ in 0..npat {
        let body = r.bytes(64 * 8 * 4)?;
        let mut p = Pattern::new(64, 8);
        for (i, b) in body.chunks_exact(4).enumerate() {
            let (fx, param) = mod_effect(b[2], b[3], false);
            p.cells[i] = Cell { note: if b[0] > 0 && b[0] < 0x60 { (b[0] + 12).min(NOTE_MAX) } else { 0 }, instr: b[1], fx, param, ..Default::default() };
        }
        patterns.push(Arc::new(p));
    }
    let samples = heads
        .into_iter()
        .take(nsmp.max(1))
        .map(|(name, ft, vol, transpose, len, ls, ll)| Sample {
            name,
            wave: Arc::new(Wave::new(8, 1, decode(r.bytes_upto(len), Pcm::S8, len))),
            c5: (f64::from(finetune_c5(ft)) * 2f64.powf(f64::from(transpose) / 12.0)) as u32,
            volume: vol.min(64),
            looped: if ll > 2 { Loop::forward(ls, ls + ll) } else { Loop::default() },
            ..Sample::empty()
        })
        .collect();
    let mut m = Module {
        format: "amf".into(),
        format_name: "Asylum".into(),
        tracker: "Asylum".into(),
        kind: Kind::Mod,
        speed: speed.max(1),
        tempo: u16::from(tempo.max(32)),
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        restart: u16::from(restart),
        channels: (0..8).map(|i| Channel { pan: amiga_pan(i), ..Default::default() }).collect(),
        orders: ord[..nord.min(256)].iter().map(|&o| u16::from(o)).collect(),
        patterns,
        samples,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}

/// Whether the 20 bytes at the start look like a title (for formats
/// with no mark).
pub fn titled(d: &[u8]) -> bool {
    d.len() >= 20 && plausible_text(&d[..20])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_med_synth_sound_is_read() {
        // A SynthInstr at 4: its sequences and two waveforms after it.
        let at = 4;
        let mut d = vec![0u8; at + 278 + 64 * 4];
        d[at + 4..at + 6].copy_from_slice(&(-1i16).to_be_bytes());
        d[at + 14..at + 16].copy_from_slice(&3u16.to_be_bytes());
        d[at + 16..at + 18].copy_from_slice(&2u16.to_be_bytes());
        (d[at + 18], d[at + 19]) = (1, 2);
        d[at + 20..at + 22].copy_from_slice(&2u16.to_be_bytes());
        d[at + 22..at + 25].copy_from_slice(&[64, 0xF2, 1]);
        d[at + 150..at + 152].copy_from_slice(&[1, 0xFF]);
        for w in 0..2u32 {
            let off = d.len() - at;
            d[at + 278 + w as usize * 4..][..4].copy_from_slice(&(off as u32).to_be_bytes());
            d.extend_from_slice(&2u16.to_be_bytes());
            d.extend_from_slice(&[w as u8, 10, 0x80, 0x7F]);
        }
        let (sound, sample) = med_synth(&d, at, false).unwrap();
        assert!(sample.is_none());
        assert_eq!((sound.volumes, sound.waves, sound.vol_speed, sound.wave_speed), (vec![64, 0xF2, 1], vec![1, 0xFF], 1, 2));
        assert_eq!(sound.waveforms, vec![vec![0, 10, -128, 127], vec![1, 10, -128, 127]]);
        // Damage is nothing read, not a panic.
        assert!(med_synth(&d[..300], at, false).is_none());
    }
}
