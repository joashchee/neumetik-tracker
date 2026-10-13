//! ProTrekkr (.ptk) and NoiseTrekker (.ntk): "TWNNSNG" (or, from about
//! 'R', "PROTREK") and a version letter, then (from '9') the rest zlib-packed. Every version's fields
//! are read as ProTrekkr's loader reads them: patterns of sixteen tracks
//! (up to sixteen notes and four effects a row), 128 instruments of up
//! to sixteen split samples and a synth each (the old versions' synth a
//! C structure, with its padding), each track's settings, the reverb,
//! chorus and compressors, two TB-303s. The song plays by its own
//! replayer, neumetik-synth's (`neumetik_synth::ptk`); the patterns show
//! it, each track's notes a channel each. Described by ProTrekkr's loader
//! (BSD), read for the format, not copied.

use std::sync::Arc;

use crate::io::{bad, Reader, Result};
use crate::model::*;
use neumetik_synth::ptk::{self as ptk, Adsr, Mods, SynthParams};

pub fn probe_ptk(d: &[u8]) -> bool {
    d.len() > 9 && (&d[..7] == b"TWNNSNG" || &d[..7] == b"PROTREK") && matches!(d[7], b'1'..=b'9' | b'A'..=b'X')
}

/// What a version's file has: each letter adds to the one before.
struct Version {
    letter: u8,
    beta: bool,
    old_ntk: bool,
    old_bug: bool,
    disto: bool,
    pack: bool,
    adsr: bool,
    comp: bool,
    packed: bool,
    portable: bool,
    poly: bool,
    multi: bool,
    mp3: bool,
    interp: bool,
    reverb: bool,
    env_mod: bool,
    new_env: bool,
    fx2: bool,
    flanger_bug: bool,
    comp_tracks: bool,
    xtra: bool,
    stereo_reverb: bool,
    reverb_res: bool,
    tb_scale: bool,
    surround: bool,
    long_midi: bool,
    var_disto: bool,
    damp: bool,
    ext_lfo: bool,
    sync: bool,
    osc3: bool,
    chorus: bool,
    denoise: bool,
}

impl Version {
    fn of(letter: u8) -> Version {
        const ORDER: &[u8] = b"3456789ABCDEFGHIJKLMNOPQRSTUVWX";
        let at = ORDER.iter().position(|&c| c == letter);
        let has = |c: u8| at.is_some_and(|a| a >= ORDER.iter().position(|&x| x == c).unwrap_or(usize::MAX));
        Version {
            letter,
            beta: letter == b'1',
            old_ntk: letter == b'1' || letter == b'2',
            old_bug: !has(b'4'),
            disto: has(b'5'),
            pack: has(b'6'),
            adsr: has(b'7'),
            comp: has(b'8'),
            packed: has(b'9'),
            portable: has(b'A'),
            poly: has(b'B'),
            multi: has(b'C'),
            mp3: has(b'D'),
            interp: has(b'E'),
            reverb: has(b'F'),
            env_mod: has(b'G'),
            new_env: has(b'H'),
            fx2: has(b'I'),
            flanger_bug: has(b'J'),
            comp_tracks: has(b'K'),
            xtra: has(b'L'),
            stereo_reverb: has(b'M'),
            reverb_res: has(b'N'),
            tb_scale: has(b'O'),
            surround: has(b'P'),
            long_midi: has(b'Q'),
            var_disto: has(b'R'),
            damp: has(b'S'),
            ext_lfo: has(b'T'),
            sync: has(b'U'),
            osc3: has(b'V'),
            chorus: has(b'W'),
            denoise: has(b'X'),
        }
    }
}

/// The most a packed song may unpack to.
const MOST: usize = 256 << 20;

pub fn load_ptk(d: &[u8]) -> Result<Module> {
    let v = Version::of(d[7]);
    let body = &d[9..];
    let data;
    let body = if v.packed {
        let packed = if v.poly { body.get(4..).unwrap_or_default() } else { body };
        data = crate::unpack::zlib(packed, MOST).ok_or_else(|| crate::io::Error::Bad("its packing is damaged".into()))?;
        &data[..]
    } else {
        body
    };
    let mut r = Reader::new(body);
    let (song, info) = read_song(&mut r, &v)?;
    Ok(module(song, info, &v))
}

/// The title, artist and style.
struct Info {
    title: String,
    artist: String,
    style: String,
}

fn read_song(r: &mut Reader, v: &Version) -> Result<(ptk::Song, Info)> {
    let mut s = ptk::Song { version: v.letter, ..ptk::Song::default() };
    let title = r.text(20)?;
    let npat = usize::from(r.u8()?);
    s.length = usize::from(r.u8()?).max(1);
    if v.interp {
        r.u8()?;
    }
    s.sequence = r.bytes(ptk::POSITIONS)?.to_vec();
    if !v.beta {
        for p in s.pattern_rows.iter_mut() {
            *p = r.u16le()?;
        }
    }
    for t in s.tracks.iter_mut() {
        t.polyphony = 1;
    }
    if v.multi {
        for t in s.tracks.iter_mut() {
            t.notes = r.u8()?.clamp(1, ptk::POLYPHONY as u8);
        }
    }
    if v.xtra {
        for t in s.tracks.iter_mut() {
            t.effects = r.u8()?.clamp(1, 4);
        }
        for t in s.tracks.iter_mut() {
            t.volume = r.f32le()?;
        }
        if v.surround {
            for t in s.tracks.iter_mut() {
                t.surround = r.u8()? != 0;
            }
        }
        for t in s.tracks.iter_mut() {
            t.eq = [r.f32le()?, r.f32le()?, r.f32le()?];
        }
        if v.denoise {
            for t in s.tracks.iter_mut() {
                t.denoise = r.u8()?.min(4);
            }
        }
    }
    // Patterns: every one's 128 rows (64 in the beta), sixteen tracks a row.
    let file_rows = if v.beta { 64 } else { ptk::MAX_ROWS };
    let mut second_fx = [false; ptk::TRACKS];
    for p in 0..npat {
        let rows = usize::from(s.pattern_rows[p]).clamp(1, ptk::MAX_ROWS);
        let mut pat = ptk::Pattern::new(rows);
        for row in 0..file_rows {
            for t in 0..ptk::TRACKS {
                let mut c = ptk::Cell::default();
                if v.multi {
                    for i in 0..ptk::POLYPHONY {
                        c.notes[i] = r.u8()?;
                        c.instrs[i] = r.u8()?;
                    }
                } else {
                    c.notes[0] = r.u8()?;
                    c.instrs[0] = r.u8()?;
                }
                c.volume = r.u8()?;
                c.panning = r.u8()?;
                c.fx[0] = (r.u8()?, r.u8()?);
                if v.fx2 {
                    c.fx[1] = (r.u8()?, r.u8()?);
                    if !v.xtra && c.fx[1] != (0, 0) {
                        second_fx[t] = true;
                    }
                }
                if v.xtra {
                    c.fx[2] = (r.u8()?, r.u8()?);
                    c.fx[3] = (r.u8()?, r.u8()?);
                }
                if row < rows {
                    pat.cells[row * ptk::TRACKS + t] = c;
                }
            }
        }
        s.patterns.push(pat);
    }
    for (t, on) in s.tracks.iter_mut().zip(second_fx) {
        if on {
            t.effects = 2;
        }
    }
    for ins in 0..ptk::INSTRUMENTS {
        let name = r.text(20)?;
        if v.long_midi {
            r.i32le()?;
        } else {
            r.u8()?;
        }
        let prg = r.u8()?;
        let synth = read_synth(r, v)?;
        if v.pack {
            let comp = r.u8()?;
            if v.mp3 && (comp == 1 || comp == 4) {
                r.u8()?;
            }
        }
        let mut splits = vec![None; ptk::SPLITS];
        for sp in splits.iter_mut() {
            if r.u8()? == 0 {
                continue;
            }
            let sname = r.text(if v.old_bug { 256 } else { 64 })?;
            let base_note = r.u8()?;
            let loop_start = r.u32le()?;
            let loop_end = r.u32le()?;
            let loop_type = r.u8()?;
            let len = r.u32le()? as usize;
            let finetune = r.i8()?;
            let amplify = r.f32le()?;
            let decay = r.f32le()?;
            if len > r.left() / 2 {
                return bad("a sample runs past the end");
            }
            let mut left = points(r.bytes(len * 2)?);
            let right = if r.u8()? == 2 {
                if len > r.left() / 2 {
                    return bad("a sample runs past the end");
                }
                Some(points(r.bytes(len * 2)?))
            } else {
                None
            };
            // The replayer zeroes each side's first point.
            if let Some(p) = left.first_mut() {
                *p = 0;
            }
            let right = right.map(|mut x| {
                if let Some(p) = x.first_mut() {
                    *p = 0;
                }
                Arc::new(x)
            });
            *sp = Some(ptk::Split { name: sname, base_note, loop_start, loop_end, loop_type, finetune, amplify, decay, left: Arc::new(left), right });
        }
        s.instruments[ins] = ptk::Instrument { name, synth_prg: prg, synth, splits, ..ptk::Instrument::default() };
    }
    let mut info = Info { title, artist: String::new(), style: String::new() };
    // What follows is settings: a file cut short keeps the defaults.
    let mut preset = 0;
    let _ = read_settings(r, v, &mut s, &mut info, &mut preset);
    if !v.reverb {
        old_reverb(&mut s, preset);
    }
    Ok((s, info))
}

fn points(b: &[u8]) -> Vec<i16> {
    b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
}

fn read_settings(r: &mut Reader, v: &Version, s: &mut ptk::Song, info: &mut Info, preset: &mut i32) -> Result<()> {
    for t in s.tracks.iter_mut() {
        t.cutoff = r.f32le()?;
        t.inertia = r.f32le()?.clamp(0.000_061_035_156_25, 0.007_812_5);
        t.pan = r.f32le()?;
        t.filter = r.i32le()?;
        t.resonance = r.i32le()?;
        t.dist_threshold = r.f32le()?;
        t.dist_clamp = r.f32le()?;
        t.reverb_send = r.f32le()?;
        t.chorus_send = r.i32le()?;
        if v.poly {
            t.polyphony = r.u8()?.clamp(1, ptk::POLYPHONY as u8);
        }
    }
    // Kept in a byte: only the low one counts.
    s.reverb = r.i32le()? & 0xFF != 0;
    s.reverb_threshold = r.i32le()?;
    s.bpm = r.i32le()?.max(1);
    s.tpb = r.i32le()?.clamp(1, 32);
    s.volume = r.f32le()?.clamp(0.01, 1.0);
    if v.comp {
        if r.u8()? != 0 {
            s.comp = [r.f32le()?.clamp(0.01, 100.0), r.f32le()?.clamp(0.01, 100.0)];
        }
        if v.comp_tracks {
            for t in s.tracks.iter_mut() {
                t.comp_threshold = r.f32le()?;
            }
            for t in s.tracks.iter_mut() {
                t.comp_ratio = r.f32le()?;
            }
            for t in s.tracks.iter_mut() {
                t.compress = r.u8()? != 0;
            }
        }
    }
    if !v.reverb {
        r.i32le()?;
    }
    s.reverb_feedback = r.f32le()?;
    if !v.reverb {
        *preset = r.i32le()?;
    }
    s.chorus_delay = [r.i32le()?, r.i32le()?];
    s.chorus_feedback = [r.f32le()?, r.f32le()?];
    s.shuffle = r.i32le()?;
    if v.reverb {
        let n = usize::from(r.u8()?).min(10);
        for _ in 0..n {
            r.i32le()?;
        }
        s.reverb_decays = (0..n).map(|_| r.f32le()).collect::<Result<_>>()?;
    }
    for a in s.active.iter_mut() {
        for on in a.iter_mut() {
            *on = r.u8()? != 0;
        }
    }
    r.skip(ptk::TRACKS * 4)?;
    for t in s.tracks.iter_mut() {
        t.lfo.on = r.u8()? == 1;
        t.lfo.rate = r.f32le()?;
        t.lfo.filter = r.f32le()?;
        if v.ext_lfo {
            t.lfo.volume = r.f32le()?;
            t.lfo.panning = r.f32le()?;
            t.lfo.scale = r.f32le()?;
        }
    }
    for t in s.tracks.iter_mut() {
        let f = &mut t.flanger;
        f.on = r.u8()? != 0;
        f.amount = r.f32le()?;
        f.dephase = r.f32le()?;
        f.rate = r.f32le()?;
        f.ampl = r.f32le()?;
        f.feedback = r.f32le()?;
        f.delay = r.i32le()?;
    }
    if !v.flanger_bug {
        // Read into the first track's (an array's name taken for its first).
        s.tracks[0].flanger.dephase = r.f32le()?;
    }
    for t in s.tracks.iter_mut() {
        t.muted = r.i32le()? != 0;
    }
    s.song_tracks = usize::from(r.u8()?).clamp(1, ptk::TRACKS);
    for t in s.tracks.iter_mut() {
        t.distortion = r.u8()? != 0;
        if !v.portable {
            r.u8()?;
        }
    }
    if v.beta {
        return Ok(());
    }
    info.artist = r.text(20)?;
    info.style = r.text(20)?;
    if !v.portable {
        r.u8()?;
    }
    for ins in s.instruments.iter_mut() {
        ins.beat_sync = r.u8()? != 0;
    }
    for ins in s.instruments.iter_mut() {
        ins.beat_lines = r.u16le()?;
    }
    s.reverb_cutoff = r.f32le()?;
    if v.reverb_res {
        s.reverb_resonance = r.f32le()?;
    }
    if v.stereo_reverb {
        s.reverb_stereo = r.u8()?;
    }
    if v.damp {
        s.reverb_damp = r.f32le()?;
    }
    if v.chorus {
        s.chorus_filter = [r.i32le()?, r.i32le()?, r.i32le()?];
    }
    for ins in s.instruments.iter_mut() {
        ins.volume = r.f32le()?;
    }
    if !v.portable {
        r.u8()?;
    }
    for tb in s.tb303.iter_mut() {
        let b = r.bytes(9)?;
        (tb.selected, tb.tune, tb.cutoff, tb.resonance, tb.envmod, tb.decay, tb.accent, tb.waveform) = (b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8]);
        tb.scale = 1;
        if v.tb_scale {
            tb.scale = r.u8()?.clamp(1, 16);
        }
        if v.portable {
            for p in 0..32 {
                tb.lengths[p] = r.u8()?;
                tb.tones[p].copy_from_slice(r.bytes(16)?);
                for i in 0..16 {
                    tb.flags[p][i] = (r.u32le()? & 0x1F) as u8;
                }
                r.skip(20)?;
            }
        } else {
            tb.lengths.copy_from_slice(r.bytes(32)?);
            for p in 0..32 {
                tb.tones[p].copy_from_slice(r.bytes(16)?);
            }
            r.skip(3)?;
            for p in 0..32 {
                for i in 0..16 {
                    tb.flags[p][i] = (r.u32le()? & 0x1F) as u8;
                }
            }
            if !v.old_ntk {
                r.skip(32 * 20)?;
            }
        }
    }
    s.tb303[0].volume = r.f32le()?;
    s.tb303[1].volume = r.f32le()?;
    Ok(())
}

/// The reverb presets the old versions chose by number (their decays a
/// 64th of these).
fn old_reverb(s: &mut ptk::Song, preset: i32) {
    let d: &[f32] = match preset {
        0 => &[20.0, 0.0, 15.0, 0.0, 9.0, 1.0, 8.0, 1.0, 4.0, 1.0],
        1 | 5 => &[7.0, -13.0, 25.0, 31.0, 20.0, 28.0, -21.0, 18.0, -13.0, 9.0],
        2 => &[1.0, 1.0, 9.0, 12.0, 22.0, 1.0, 15.0, 1.0, 7.0, 2.0],
        3 => &[22.0, 5.0, 12.0, 3.0],
        4 => &[11.0, 0.0, 31.0, 0.0],
        6 => &[7.0, -13.0, 25.0, 31.0, 20.0, 28.0, -21.0, 18.0, -13.0, 12.0],
        _ => &[],
    };
    // An unknown preset keeps the last one's (none, here).
    s.reverb_decays = d.iter().map(|x| x * 0.015_625).collect();
}

/// The synth as each version stores it.
fn read_synth(r: &mut Reader, v: &Version) -> Result<SynthParams> {
    let mut p = SynthParams { disto: 64, lfo1_adsr: Adsr { attack: 0, decay: 0, sustain: 128, release: 0x10000 }, lfo2_adsr: Adsr { attack: 0, decay: 0, sustain: 128, release: 0x10000 }, sync: 0, osc3_interval: 12, ..SynthParams::default() };
    if !v.portable {
        if v.beta {
            let b = r.bytes(108)?;
            let i = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
            p.osc1_wave = b[20];
            p.osc2_wave = b[21];
            p.osc1_pw = i(24);
            p.osc2_pw = i(28);
            (p.osc2_detune, p.osc2_finetune, p.vcf_cutoff, p.vcf_resonance, p.vcf_type) = (b[32], b[33], b[34], b[35], b[38]);
            p.env1 = Adsr { attack: i(40), decay: i(44), sustain: b[48], release: i(52) };
            p.env2 = Adsr { attack: i(56), decay: i(60), sustain: b[64], release: i(68) };
            p.lfo1_period = i(72);
            p.lfo2_period = i(76);
            let six = |o: usize| Mods { osc1_pw: b[o], osc2_pw: b[o + 1], osc1_pitch: 64, osc2_pitch: 64, osc1_volume: b[o + 2], osc2_volume: b[o + 3], cutoff: b[o + 4], resonance: b[o + 5], disto: 64 };
            p.lfo1 = six(80);
            p.lfo2 = six(86);
            p.env1_mods = six(92);
            p.env2_mods = six(98);
            p.osc3_volume = b[104];
            p.osc3_switch = b[105];
            p.glide = 0;
            p.volume = 127;
        } else {
            // The C structure as it was, its padding and all: through the
            // global volume (116 bytes), the distortion (120), the LFOs'
            // envelopes (152).
            let size = if v.disto && v.adsr {
                152
            } else if v.disto {
                120
            } else {
                116
            };
            let b = r.bytes(size)?;
            let i = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
            let eight = |o: usize, disto: u8| Mods { osc1_pw: b[o], osc2_pw: b[o + 1], osc1_pitch: b[o + 2], osc2_pitch: b[o + 3], osc1_volume: b[o + 4], osc2_volume: b[o + 5], cutoff: b[o + 6], resonance: b[o + 7], disto };
            p.osc1_wave = b[20];
            p.osc2_wave = b[21];
            // The padding byte that later became the combine mode.
            p.combine = b[22];
            p.osc1_pw = i(24);
            p.osc2_pw = i(28);
            (p.osc2_detune, p.osc2_finetune, p.vcf_cutoff, p.vcf_resonance, p.vcf_type) = (b[32], b[33], b[34], b[35], b[36]);
            p.env1 = Adsr { attack: i(40), decay: i(44), sustain: b[48], release: i(52) };
            p.env2 = Adsr { attack: i(56), decay: i(60), sustain: b[64], release: i(68) };
            p.lfo1_period = i(72);
            p.lfo2_period = i(76);
            p.lfo1 = eight(80, p.lfo1.disto);
            p.lfo2 = eight(88, p.lfo2.disto);
            p.env1_mods = eight(96, p.env1_mods.disto);
            p.env2_mods = eight(104, p.env2_mods.disto);
            (p.osc3_volume, p.osc3_switch, p.glide, p.volume) = (b[112], b[113], b[114], b[115]);
            if size >= 120 {
                p.disto = b[116];
            }
            if size >= 152 {
                p.lfo1_adsr = Adsr { attack: i(120), decay: i(124), sustain: b[128], release: i(132) };
                p.lfo2_adsr = Adsr { attack: i(136), decay: i(140), sustain: b[144], release: i(148) };
                p.disto = p.disto / 2 + 64;
                p.volume = 0x7F;
            }
        }
    } else {
        r.skip(20)?;
        p.osc1_wave = r.u8()?;
        p.osc2_wave = r.u8()?;
        p.osc1_pw = r.i32le()?;
        p.osc2_pw = r.i32le()?;
        let b = r.bytes(5)?;
        (p.osc2_detune, p.osc2_finetune, p.vcf_cutoff, p.vcf_resonance, p.vcf_type) = (b[0], b[1], b[2], b[3], b[4]);
        p.env1 = read_adsr(r)?;
        p.env2 = read_adsr(r)?;
        p.lfo1_period = r.i32le()?;
        p.lfo2_period = r.i32le()?;
        for m in [&mut p.lfo1, &mut p.lfo2, &mut p.env1_mods, &mut p.env2_mods] {
            let b = r.bytes(8)?;
            (m.osc1_pw, m.osc2_pw, m.osc1_pitch, m.osc2_pitch, m.osc1_volume, m.osc2_volume, m.cutoff, m.resonance) = (b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]);
            if v.var_disto {
                m.disto = r.u8()?;
            }
        }
        let b = r.bytes(4)?;
        (p.osc3_volume, p.osc3_switch, p.glide, p.volume) = (b[0], b[1], b[2], b[3]);
        if v.disto {
            p.disto = r.u8()?;
        }
        if !v.var_disto {
            p.disto = p.disto / 2 + 64;
        }
        if v.adsr {
            p.lfo1_adsr = read_adsr(r)?;
            p.lfo2_adsr = read_adsr(r)?;
        }
        if v.xtra {
            p.combine = r.u8()?;
        }
        if v.sync {
            p.sync = r.u8()?;
        }
        if v.osc3 {
            p.osc3_interval = r.u8()?;
        }
    }
    if !v.env_mod {
        std::mem::swap(&mut p.env1_mods.osc2_volume, &mut p.env2_mods.osc1_volume);
    }
    if !v.new_env {
        // A 64 was a full envelope in the old versions.
        for x in [&mut p.env1_mods.osc1_volume, &mut p.env1_mods.osc2_volume, &mut p.env2_mods.osc1_volume, &mut p.env2_mods.osc2_volume] {
            if *x == 64 {
                *x = 127;
            }
        }
    }
    if !v.sync {
        // Pink noise, which became the triangle, plays as white.
        for w in [&mut p.osc1_wave, &mut p.osc2_wave] {
            if *w == 6 {
                *w = 3;
            }
        }
    }
    p.lfo1_period = p.lfo1_period.min(128);
    p.lfo2_period = p.lfo2_period.min(128);
    if v.old_ntk {
        if p.glide < 1 {
            p.glide = 64;
        }
        if p.volume < 1 {
            p.volume = 64;
        }
    }
    Ok(p)
}

fn read_adsr(r: &mut Reader) -> Result<Adsr> {
    Ok(Adsr { attack: r.i32le()?, decay: r.i32le()?, sustain: r.u8()?, release: r.i32le()? })
}

/// The module that shows the song: each track's notes a channel each,
/// its volume, panning and first effect the tracker has a name for on
/// its first; every split a sample; the song itself in `ptk`.
fn module(song: ptk::Song, info: Info, v: &Version) -> Module {
    let ntrk = song.song_tracks.clamp(1, ptk::TRACKS);
    let cols: Vec<usize> = song.tracks.iter().take(ntrk).map(|t| usize::from(t.notes).clamp(1, ptk::POLYPHONY)).collect();
    let nch: usize = cols.iter().sum();
    let name = if v.old_ntk { "NoiseTrekker" } else { "ProTrekkr" };
    let mut m = Module {
        format: if v.old_ntk { "ntk".into() } else { "ptk".into() },
        format_name: name.into(),
        tracker: format!("{name} (version {})", v.letter as char),
        kind: Kind::Xm,
        speed: 6,
        tempo: song.bpm.clamp(32, 999) as u16,
        global_volume: 128,
        mix_volume: 48,
        separation: 128,
        tempo_mode: TempoMode::Modern,
        time: TimeSig { rows_per_beat: song.tpb.clamp(1, 32) as u16, rows_per_measure: (song.tpb.clamp(1, 32) * 4) as u16 },
        ..Default::default()
    };
    m.title = info.title;
    m.artist = info.artist;
    if !info.style.is_empty() && info.style != "Anything Goes" {
        m.message = info.style;
    }
    for (t, &n) in cols.iter().enumerate() {
        let tr = &song.tracks[t];
        for k in 0..n {
            let name = if n > 1 { format!("Track {} ({})", t + 1, k + 1) } else { format!("Track {}", t + 1) };
            m.channels.push(Channel { pan: (tr.pan.clamp(0.0, 1.0) * 255.0) as u8, muted: tr.muted, name, ..Default::default() });
        }
    }
    // Samples: the splits in order; instruments up to the last that has anything.
    let mut sample_of = vec![[None::<u16>; ptk::SPLITS]; ptk::INSTRUMENTS];
    for (i, s, sp) in song.splits() {
        sample_of[i][s] = Some(m.samples.len() as u16 + 1);
        m.samples.push(module_sample(sp, &song.instruments[i].name));
    }
    let last = song.instruments.iter().rposition(|ins| !ins.name.trim().is_empty() || ins.synth_prg != 0 || ins.splits.iter().any(Option::is_some)).map_or(0, |i| i + 1);
    for (i, ins) in song.instruments.iter().take(last).enumerate() {
        let keymap = (0..120u8)
            .map(|n| {
                let pn = n;
                let split = ins.splits.iter().enumerate().filter(|(_, s)| s.as_ref().is_some_and(|s| pn >= s.base_note)).map(|(k, _)| k).last().unwrap_or(0);
                (n, sample_of[i][split].unwrap_or(0))
            })
            .collect();
        m.instruments.push(Instrument { name: ins.name.clone(), keymap, global_volume: 128, ..Default::default() });
    }
    m.flags.instruments = true;
    m.flags.linear = true;
    let max_pat = song.sequence.iter().take(song.length).map(|&p| usize::from(p)).max().unwrap_or(0).max(song.patterns.len().saturating_sub(1));
    for p in 0..=max_pat {
        let rows = usize::from(song.pattern_rows.get(p).copied().unwrap_or(64)).clamp(1, ptk::MAX_ROWS);
        let mut pat = Pattern::new(rows, nch.max(1));
        if let Some(src) = song.patterns.get(p) {
            for row in 0..rows {
                let mut ch = 0;
                for (t, &n) in cols.iter().enumerate() {
                    let Some(c) = src.cell(row, t) else { continue };
                    for k in 0..n {
                        let cell = pat.cell_mut(row, ch + k).expect("in the pattern");
                        cell.note = match c.notes[k] {
                            ptk::NOTE_OFF => NOTE_OFF,
                            n if n < ptk::NOTE_OFF => n + 1,
                            _ => NOTE_NONE,
                        };
                        cell.instr = if c.instrs[k] == ptk::NO_INSTR { 0 } else { c.instrs[k].saturating_add(1) };
                        if k == 0 {
                            if c.volume <= 64 {
                                (cell.vol, cell.volp) = (Vol::Volume, c.volume);
                            } else if c.panning <= 128 {
                                (cell.vol, cell.volp) = (Vol::Panning, c.panning / 2);
                            }
                            let nfx = usize::from(song.tracks[t].effects).clamp(1, 4);
                            if let Some((fx, param)) = c.fx[..nfx].iter().find_map(|&(e, d)| shown_fx(e, d)) {
                                (cell.fx, cell.param) = (fx, param);
                            }
                        }
                    }
                    ch += n;
                }
            }
        }
        m.patterns.push(Arc::new(pat));
    }
    m.orders = song.sequence.iter().take(song.length).map(|&p| u16::from(p)).collect();
    m.ptk = Some(Arc::new(song));
    m
}

/// A ProTrekkr effect the tracker has a name for, for the screen.
fn shown_fx(e: u8, d: u8) -> Option<(Fx, u8)> {
    Some(match e {
        0x01 => (Fx::PortaUp, d),
        0x02 => (Fx::PortaDown, d),
        0x03 => (Fx::Volume, d / 4),
        0x05 => (Fx::TonePorta, d),
        0x09 => (Fx::Offset, d),
        0x0D => (Fx::PatternBreak, d),
        0x0E => (Fx::Retrig, d),
        0x0F => (Fx::Speed, d),
        0x1B => (Fx::Arpeggio, d),
        0x1C => (Fx::GlobalVolume, d / 2),
        0x1F => (Fx::PositionJump, d),
        0xF0 => (Fx::Tempo, d),
        _ => return None,
    })
}

/// A split as the tracker keeps it: its points (both sides, interleaved,
/// for a stereo one), its loop, its rate at C-5 (it plays at 44.1 kHz on
/// its base note, ProTrekkr's C-4 our note `base + 1`). Only the
/// tracker's own playing (a note jammed in the editor) reads these; the
/// song plays its own splits.
fn module_sample(sp: &ptk::Split, instrument: &str) -> Sample {
    let pcm = match &sp.right {
        Some(r) => sp.left.iter().zip(r.iter()).flat_map(|(&l, &r)| [l, r]).collect(),
        None => sp.left.as_ref().clone(),
    };
    let wave = Wave::new(16, if sp.right.is_some() { 2 } else { 1 }, pcm);
    let frames = sp.len() as u32;
    let (ls, le) = (sp.loop_start.min(frames), sp.loop_end.min(frames));
    // A ping-pong loop turns on its end point, which it plays: the loop
    // keeps it (the repository cuts a sample after its loop).
    let looped = match sp.loop_type {
        1 if le > ls => Loop::forward(ls, le),
        2 if le > ls => Loop { kind: LoopKind::PingPong, start: ls, end: (le + 1).min(frames) },
        _ => Loop::default(),
    };
    let semis = (f64::from(NOTE_MIDDLE_C) - f64::from(sp.base_note) - 1.0) + f64::from(sp.finetune) / 128.0;
    let c5 = (f64::from(ptk::RATE) * 2f64.powf(semis / 12.0)).round().clamp(1.0, 2_000_000.0) as u32;
    let name = if sp.name.trim().is_empty() { instrument.to_string() } else { sp.name.clone() };
    Sample { name, wave: Arc::new(wave), looped, c5, ..Sample::empty() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A zlib stream of stored blocks (no packing), as a file's packed body.
    fn zlib_stored(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x78, 0x01];
        let chunks: Vec<&[u8]> = data.chunks(65_535).collect();
        for (i, c) in chunks.iter().enumerate() {
            out.push(u8::from(i + 1 == chunks.len()));
            out.extend_from_slice(&(c.len() as u16).to_le_bytes());
            out.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
            out.extend_from_slice(c);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in data {
            a = (a + u32::from(x)) % 65_521;
            b = (b + a) % 65_521;
        }
        out.extend_from_slice(&(b << 16 | a).to_be_bytes());
        out
    }

    struct W(Vec<u8>);

    impl W {
        fn u8(&mut self, v: u8) {
            self.0.push(v);
        }
        fn i32(&mut self, v: i32) {
            self.0.extend_from_slice(&v.to_le_bytes());
        }
        fn f32(&mut self, v: f32) {
            self.0.extend_from_slice(&v.to_le_bytes());
        }
        fn text(&mut self, s: &str, n: usize) {
            let mut b = s.as_bytes().to_vec();
            b.resize(n, 0);
            self.0.extend_from_slice(&b);
        }
        fn adsr(&mut self, a: i32, d: i32, s: u8, r: i32) {
            self.i32(a);
            self.i32(d);
            self.u8(s);
            self.i32(r);
        }
    }

    /// A version X song of sixteen rows: a sawtooth synth note on track
    /// 0, a square-wave sample on track 1, the first TB-303 fired on
    /// track 2.
    fn file() -> Vec<u8> {
        let mut w = W(Vec::new());
        w.text("Test", 20);
        w.u8(1);
        w.u8(1);
        w.u8(0);
        w.0.extend_from_slice(&[0u8; 256]);
        for p in 0..128 {
            w.0.extend_from_slice(&(if p == 0 { 16u16 } else { 64 }).to_le_bytes());
        }
        w.0.extend_from_slice(&[1u8; 16]);
        w.0.extend_from_slice(&[1u8; 16]);
        for _ in 0..16 {
            w.f32(1.0);
        }
        w.0.extend_from_slice(&[0u8; 16]);
        for _ in 0..48 {
            w.f32(1.0);
        }
        w.0.extend_from_slice(&[0u8; 16]);
        for row in 0..128 {
            for t in 0..16 {
                let mut cell = [0u8; 42];
                for i in 0..16 {
                    cell[i * 2] = ptk::NO_NOTE;
                    cell[i * 2 + 1] = ptk::NO_INSTR;
                }
                cell[32] = ptk::NO_VALUE;
                cell[33] = ptk::NO_VALUE;
                if row == 0 && t < 2 {
                    cell[0] = 48;
                    cell[1] = t as u8;
                }
                if row == 0 && t == 2 {
                    cell[34] = 0x31;
                    cell[35] = 0xA1;
                }
                w.0.extend_from_slice(&cell);
            }
        }
        for ins in 0..128 {
            w.text(if ins == 0 { "saw" } else { "" }, 20);
            w.i32(-1);
            w.u8(if ins == 0 { 1 } else { 0 });
            // The synth: a saw on oscillator 1, nothing on 2, full volume.
            w.text("", 20);
            w.u8(1);
            w.u8(4);
            w.i32(256);
            w.i32(256);
            w.0.extend_from_slice(&[64, 0, 127, 0, 2]);
            w.adsr(0, 2560, 100, 16384);
            w.adsr(0, 2560, 100, 16384);
            w.i32(16);
            w.i32(16);
            for full in [false, false, true, true] {
                w.0.extend_from_slice(&[64, 64, 64, 64, if full { 127 } else { 64 }, if full { 127 } else { 64 }, 64, 64, 64]);
            }
            w.0.extend_from_slice(&[64, 0, 0, 128]);
            w.u8(64);
            w.adsr(2560, 2560, 16, 16384);
            w.adsr(2560, 2560, 16, 16384);
            w.u8(0);
            w.u8(0);
            w.u8(12);
            w.u8(7);
            for sp in 0..16 {
                if ins == 1 && sp == 0 {
                    w.u8(1);
                    w.text("square", 64);
                    w.u8(48);
                    w.i32(0);
                    w.i32(200);
                    w.u8(1);
                    w.i32(200);
                    w.u8(0);
                    w.f32(1.0);
                    w.f32(0.0);
                    for i in 0..200 {
                        w.0.extend_from_slice(&(if i % 100 < 50 { 12_000i16 } else { -12_000 }).to_le_bytes());
                    }
                    w.u8(1);
                } else {
                    w.u8(0);
                }
            }
        }
        for _ in 0..16 {
            for v in [126.0f32, 0.0039, 0.5] {
                w.f32(v);
            }
            w.i32(4);
            w.i32(64);
            for v in [32767.0f32, 32767.0, 0.0] {
                w.f32(v);
            }
            w.i32(0);
            w.u8(1);
        }
        w.i32(0);
        w.i32(32);
        w.i32(125);
        w.i32(4);
        w.f32(1.0);
        w.u8(0);
        for _ in 0..32 {
            w.f32(100.0);
        }
        w.0.extend_from_slice(&[0u8; 16]);
        w.f32(0.5);
        w.i32(10584);
        w.i32(15876);
        w.f32(0.6);
        w.f32(0.5);
        w.i32(0);
        w.u8(0);
        w.0.extend_from_slice(&[1u8; 256 * 16]);
        w.0.extend_from_slice(&[0u8; 64]);
        for _ in 0..16 {
            w.u8(0);
            for v in [0.0001f32, 0.0, 0.0, 0.0, 1.0] {
                w.f32(v);
            }
        }
        for _ in 0..16 {
            w.u8(0);
            for v in [-0.8f32, 0.0174532, 0.0001, 0.001, -0.51] {
                w.f32(v);
            }
            w.i32(176);
        }
        w.0.extend_from_slice(&[0u8; 64]);
        w.u8(3);
        w.0.extend_from_slice(&[0u8; 16]);
        w.text("Someone", 20);
        w.text("Anything Goes", 20);
        w.0.extend_from_slice(&[0u8; 128]);
        w.0.extend_from_slice(&[16u8, 0].repeat(128));
        w.f32(0.08);
        w.f32(0.5);
        w.u8(63);
        w.f32(1.0);
        w.i32(4);
        w.i32(0);
        w.i32(64);
        for _ in 0..128 {
            w.f32(1.0);
        }
        for _ in 0..2 {
            w.0.extend_from_slice(&[1, 0, 64, 64, 64, 64, 64, 64, 0]);
            w.u8(1);
            for p in 0..32 {
                w.u8(4);
                w.0.extend_from_slice(&[12, 0, 7, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
                for i in 0..16 {
                    w.i32(if p == 0 && i == 2 { 16 | 1 } else { 16 });
                }
                w.text("", 20);
            }
        }
        w.f32(0.5);
        w.f32(0.5);
        let mut d = b"PROTREKX\0".to_vec();
        d.extend_from_slice(&(w.0.len() as u32).to_le_bytes());
        d.extend_from_slice(&zlib_stored(&w.0));
        d
    }

    #[test]
    fn a_song_reads_shows_its_tracks_plays_and_comes_back_from_umf() {
        let d = file();
        let m = crate::formats::load(&d, "t.ptk").expect("reads");
        assert_eq!((m.format.as_str(), m.channels.len(), m.samples.len(), m.patterns[0].rows), ("ptk", 3, 1, 16));
        let p = &m.patterns[0];
        assert_eq!((p.cell(0, 0).note, p.cell(0, 0).instr, p.cell(0, 1).instr), (49, 1, 2));
        let song = m.ptk.as_deref().expect("its song");
        assert_eq!((song.bpm, song.tpb, song.instruments[0].synth_prg, song.instruments[0].synth.osc1_wave), (125, 4, 1, 1));
        assert_eq!(song.tb303[0].flags[0][2], 17);
        let audio = crate::player::render(Arc::new(m.clone()), 22_050, 1.0, 0.0);
        let rms = (audio.iter().map(|v| v * v).sum::<f32>() / audio.len() as f32).sqrt();
        assert!(rms > 0.01, "it plays: {rms}");
        use crate::canon::{canonical_wave, sample_id, wave_id};
        let refs: Vec<_> = m.samples.iter().map(|s| Some(crate::umf::SampleRef { sample: sample_id(wave_id(&canonical_wave(s)), s), wave: wave_id(&canonical_wave(s)) })).collect();
        let bytes = crate::umf::write(&m, &refs, &[], &Default::default(), true);
        let back = crate::umf::read(&bytes, &mut |_| None).expect("reads back").module;
        assert_eq!(back.ptk.as_deref(), m.ptk.as_deref());
        assert_eq!(crate::player::render(Arc::new(back), 22_050, 1.0, 0.0), audio);
    }

    #[test]
    fn its_length_is_its_rows() {
        let m = crate::formats::load(&file(), "t.ptk").expect("reads");
        // Sixteen rows of 5,292 frames (and one more each) at 125 BPM, 4 rows a beat.
        let secs = crate::player::length(Arc::new(m), 60.0);
        assert!((secs - 16.0 * 5293.0 / 44_100.0).abs() < 0.01, "{secs}");
    }
}
