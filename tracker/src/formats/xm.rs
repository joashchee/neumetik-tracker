//! FastTracker 2's Extended Module (.xm), and MilkyTracker's, OpenMPT's,
//! MadTracker's and the rest that write it. A header, patterns packed by
//! a bit mask a cell, then instruments: a keymap, volume and panning
//! envelopes, auto-vibrato, and their samples, delta-encoded (or, in
//! OggMod's .oxm, Ogg Vorbis).

use std::sync::Arc;

use super::common::{mod_effect, relative_c5};
use crate::io::{bad, decode, Pcm, Reader, Result};
use crate::model::{AutoVibrato, Cell, Channel, EnvPoint, Envelope, Flags, Instrument, Kind, Loop, LoopKind, Module, Pattern, Sample, Vol, Wave, NOTE_MAX, NOTE_OFF};

pub fn probe(data: &[u8]) -> bool {
    data.len() >= 80 && data[..17].eq_ignore_ascii_case(b"Extended Module: ")
}

/// An XM volume column byte as UMF's.
pub fn volume_column(v: u8) -> (Vol, u8) {
    let x = v & 0x0F;
    match v {
        0x10..=0x50 => (Vol::Volume, v - 0x10),
        0x60..=0x6F => (Vol::VolSlideDown, x),
        0x70..=0x7F => (Vol::VolSlideUp, x),
        0x80..=0x8F => (Vol::FineVolDown, x),
        0x90..=0x9F => (Vol::FineVolUp, x),
        0xA0..=0xAF => (Vol::VibratoSpeed, x),
        0xB0..=0xBF => (Vol::VibratoDepth, x),
        0xC0..=0xCF => (Vol::Panning, (u16::from(x) * 64 / 15) as u8),
        0xD0..=0xDF => (Vol::PanSlideLeft, x),
        0xE0..=0xEF => (Vol::PanSlideRight, x),
        0xF0..=0xFF => (Vol::TonePorta, x),
        _ => (Vol::None, 0),
    }
}

/// Twelve envelope points of (tick, value), `count` used, and their
/// flags and loop points; `pan` values are 0 to 64 around 32.
pub fn envelope(points: &[(u16, u16)], count: u8, flags: u8, sustain: u8, ls: u8, le: u8, pan: bool) -> Envelope {
    let count = usize::from(count).min(points.len());
    let pts: Vec<EnvPoint> = points[..count]
        .iter()
        .map(|&(t, v)| EnvPoint { tick: t, value: if pan { (v.min(64) as i16 - 32) as i8 } else { v.min(64) as i8 } })
        .collect();
    let n = pts.len() as u8;
    Envelope {
        enabled: flags & 1 != 0 && n > 0,
        looped: (flags & 4 != 0 && ls < n && le < n && ls <= le).then_some((ls, le)),
        sustain: (flags & 2 != 0 && sustain < n).then_some((sustain, sustain)),
        points: pts,
        carry: false,
        filter: false,
        release: None,
    }
}

pub fn load(data: &[u8]) -> Result<Module> {
    let mut r = Reader::new(data);
    r.seek(17)?;
    let title = r.text(20)?;
    r.skip(1)?;
    let tracker = r.text(20)?;
    let version = r.u16le()?;
    let hsize = r.u32le()? as usize;
    let songlen = usize::from(r.u16le()?);
    let restart = r.u16le()?;
    let nch = usize::from(r.u16le()?);
    let npat = usize::from(r.u16le()?);
    let nins = usize::from(r.u16le()?);
    let flags = r.u16le()?;
    let speed = r.u16le()?;
    let bpm = r.u16le()?;
    let orders_raw = r.bytes(256)?.to_vec();
    if nch == 0 || nch > 128 {
        return bad(format!("it says it has {nch} channels"));
    }
    if version < 0x0104 {
        // 1.02 and 1.03 put the instruments before the patterns; rare enough to leave.
        return Err(crate::io::Error::Unsupported(format!("XM version {}.{:02} files aren't read yet", version >> 8, version & 0xFF)));
    }
    r.seek(60 + hsize)?;

    let mut patterns = Vec::with_capacity(npat);
    for _ in 0..npat {
        let start = r.pos;
        let plen = r.u32le()? as usize;
        let _packing = r.u8()?;
        let rows = usize::from(r.u16le()?);
        let size = usize::from(r.u16le()?);
        r.seek(start + plen)?;
        let rows = if rows == 0 { 64 } else { rows.min(1024) };
        let mut pat = Pattern::new(rows, nch);
        let packed = r.bytes(size)?;
        let mut pr = Reader::new(packed);
        'cells: for i in 0..rows * nch {
            if pr.eof() {
                break;
            }
            let mut c = Cell::default();
            let first = pr.u8()?;
            let (mask, note) = if first & 0x80 != 0 { (first, None) } else { (0x1F, Some(first)) };
            let mut get = |bit: u8, given: Option<u8>| -> Option<u8> {
                if let Some(g) = given {
                    return Some(g);
                }
                if mask & bit != 0 {
                    pr.u8().ok()
                } else {
                    Some(0)
                }
            };
            let Some(n) = get(1, note) else { break 'cells };
            let Some(ins) = get(2, None) else { break 'cells };
            let Some(vol) = get(4, None) else { break 'cells };
            let Some(fx) = get(8, None) else { break 'cells };
            let Some(param) = get(16, None) else { break 'cells };
            c.note = match n {
                97 => NOTE_OFF,
                1..=96 => (n + 12).min(NOTE_MAX),
                _ => 0,
            };
            c.instr = ins;
            (c.vol, c.volp) = volume_column(vol);
            (c.fx, c.param) = mod_effect(fx, param, true);
            pat.cells[i] = c;
        }
        patterns.push(Arc::new(pat));
    }

    let mut instruments = Vec::with_capacity(nins);
    let mut samples: Vec<Sample> = Vec::new();
    let mut vorbis = false;
    for _ in 0..nins {
        let start = r.pos;
        let isize = r.u32le()? as usize;
        let name = r.text(22)?;
        let _kind = r.u8()?;
        let nsmp = usize::from(r.u16le()?);
        let mut inst = Instrument { name, global_volume: 128, ..Default::default() };
        if nsmp == 0 {
            r.seek(start + isize.max(29))?;
            inst.keymap = (0..120).map(|n| (n as u8, 0)).collect();
            instruments.push(inst);
            continue;
        }
        let _shsize = r.u32le()?;
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
        r.seek(start + isize)?;
        inst.volume_env = envelope(&vol_pts, nvol.min(12), vtype, vsus, vls, vle, false);
        inst.pan_env = envelope(&pan_pts, npan.min(12), ptype, psus, pls, ple, true);
        inst.fadeout = u32::from(fadeout) * 2;

        let first = samples.len();
        let mut heads = Vec::with_capacity(nsmp);
        for _ in 0..nsmp {
            let sstart = r.pos;
            let len = r.u32le()? as usize;
            let ls = r.u32le()?;
            let ll = r.u32le()?;
            let volume = r.u8()?;
            let finetune = r.i8()?;
            let stype = r.u8()?;
            let pan = r.u8()?;
            let rel = r.i8()?;
            let packed = r.u8()?;
            let sname = r.text(22)?;
            r.seek(sstart + 40)?;
            heads.push((len, ls, ll, volume, finetune, stype, pan, rel, packed, sname));
        }
        for (len, ls, ll, volume, finetune, stype, pan, rel, packed, sname) in heads {
            let b16 = stype & 0x10 != 0;
            let stereo = stype & 0x20 != 0;
            let bytes = r.bytes_upto(len);
            // OggMod's: the sample's length, then the Ogg Vorbis stream it was
            // squeezed into, one channel even for a stereo sample (left, then right).
            let ogg = (bytes.len() >= 8 && &bytes[4..8] == b"OggS").then(|| crate::vorbis::decode_ogg(&bytes[4..])).flatten().filter(|v| v.channels == 1);
            vorbis |= ogg.is_some();
            let (pcm, ch) = if let Some(v) = ogg {
                let pcm: Vec<i16> = if b16 { v.pcm } else { v.pcm.iter().map(|&x| x & !0xFF).collect() };
                if stereo {
                    let half = pcm.len() / 2;
                    ((0..half).flat_map(|i| [pcm[i], pcm[half + i]]).collect(), 2)
                } else {
                    (pcm, 1)
                }
            } else if packed == 0xAD && !b16 {
                (super::s3m::adpcm4(bytes, len.saturating_sub(16) * 2), 1)
            } else {
                let fmt = if b16 { Pcm::D16le } else { Pcm::D8 };
                let points = len / fmt.bytes();
                let pcm = decode(bytes, fmt, points);
                if stereo && pcm.len() >= 2 {
                    // OpenMPT's stereo XM: left then right, each delta-coded on its own.
                    let half = pcm.len() / 2;
                    let right = decode(&bytes[half * fmt.bytes()..], fmt, half);
                    let mut out = Vec::with_capacity(half * 2);
                    for i in 0..half.min(right.len()) {
                        out.push(pcm[i]);
                        out.push(right[i]);
                    }
                    (out, 2)
                } else {
                    (pcm, 1)
                }
            };
            let div = if b16 { 2 } else { 1 } * ch as u32;
            let kind = match stype & 3 {
                1 => LoopKind::Forward,
                2 | 3 => LoopKind::PingPong,
                _ => LoopKind::Off,
            };
            let looped = Loop { kind, start: ls / div, end: ls.saturating_add(ll) / div };
            samples.push(Sample {
                name: sname,
                wave: Arc::new(Wave::new(if b16 { 16 } else { 8 }, ch, pcm)),
                c5: relative_c5(rel, finetune),
                volume: volume.min(64),
                global_volume: 64,
                pan: Some(pan),
                looped,
                vibrato: vib,
                ..Sample::empty()
            });
        }
        inst.keymap = (0..120)
            .map(|n| {
                // XM's keymap covers its 96 notes, which sit an octave up here.
                let xm = n as i32 - 12;
                let s = if (0..96).contains(&xm) { keymap[xm as usize] as usize } else { 0 };
                let global = if (0..96).contains(&xm) && s < nsmp { (first + s + 1) as u16 } else { 0 };
                (n as u8, global)
            })
            .collect();
        instruments.push(inst);
    }

    let orders = orders_raw[..songlen.min(256)].iter().map(|&o| u16::from(o)).collect();
    let mut m = Module {
        title,
        format: if vorbis { "oxm" } else { "xm" }.into(),
        format_name: if vorbis { "OggMod XM" } else { "FastTracker 2" }.into(),
        tracker: if tracker.is_empty() { "FastTracker 2".into() } else { tracker },
        kind: Kind::Xm,
        flags: Flags { linear: flags & 1 != 0, instruments: true, ..Default::default() },
        speed: speed.clamp(1, 255) as u8,
        tempo: bpm.clamp(32, 999),
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        restart,
        channels: (0..nch).map(|_| Channel::default()).collect(),
        orders,
        patterns,
        samples,
        instruments,
        ..Default::default()
    };
    m.tidy();
    Ok(m)
}
