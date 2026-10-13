//! **UMF**, the Universal Mod Format: a tracker module that refers to
//! its samples in the samples repository rather than holding them.
//!
//! A UMF file has the song (patterns, orders, channels, instruments with
//! their keymaps and envelopes, and every sample's settings: tuning,
//! volume, loops) and, for each sample, the IDs of its sample and
//! waveform in the repository (`repo::canon`). A UMF player loads the
//! waveforms from the repository by those IDs. Saved "with samples",
//! the waveforms are in the file too, so it plays anywhere, with or
//! without a repository; a waveform in the file is checked against its
//! ID like one from the repository. A UMF with its samples is saved as
//! `.ntt` ([`extension`]).
//!
//! The layout (little-endian throughout):
//!
//! ```text
//! "UMF" 1A            magic
//! version             u16, 1
//! flags               u16: 1 has its waveforms
//! chunks              id (4 bytes), length (u32), data
//!   INFO              the song's name, words and settings
//!   CHAN              the channels
//!   ORDR              the order list
//!   PATT              the patterns, packed (lz.rs)
//!   SMPL              every sample's settings and IDs
//!   INST              the instruments
//!   (extras)          SONG, PATX... when the song uses them (umf-format.md)
//!   VISL              how the song likes to be seen (a neumetik-visual settings line)
//!   COVR              its cover: an ANSI art file, packed (lz.rs)
//!   WAVE              a waveform: its ID, then its repository file (any number)
//!   END               the end
//! ```
//!
//! Chunks a reader doesn't know are passed over, so later versions can
//! add more. Strings are a u16 length, then UTF-8.

use std::collections::HashMap;
use std::sync::Arc;

use crate::io::{Error, Reader, Result};
use crate::lz;
use crate::model::*;
use crate::canon::{audible_end, canonical_wave, instrument_id, sample_id, wave_id, Id};
use crate::wavefile::{pack_wave, unpack_wave};

const MAGIC: &[u8; 4] = b"UMF\x1A";
const VERSION: u16 = 1;

/// A UMF's extension: `umf` when its samples are in the repository,
/// `ntt` ("Neumetik Tracker type") when it carries them, so a file that
/// plays on its own (an NTT player's) is told at a glance from one that
/// needs the repository (a UMF player's). The layout is the same.
pub fn extension(bundled: bool) -> &'static str {
    if bundled {
        "ntt"
    } else {
        "umf"
    }
}

/// A sample's place in the repository: its sample's ID and its waveform's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleRef {
    pub sample: Id,
    pub wave: Id,
}

/// What the file says beside the song.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct About {
    /// The module it was made from, by its file's hash.
    pub source: Option<Id>,
    /// The repository its samples were kept in.
    pub repository: String,
    /// Its waveforms are in the file.
    pub bundled: bool,
}

/// A UMF read: the song, each sample slot's IDs, and the samples whose
/// waveforms weren't found (silent until they are).
#[derive(Clone, Debug, Default)]
pub struct Loaded {
    pub module: Module,
    pub refs: Vec<Option<SampleRef>>,
    pub instrument_ids: Vec<Option<Id>>,
    pub about: About,
    /// (1-based slot, waveform ID) of each sample not found.
    pub missing: Vec<(u16, Id)>,
}

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn s(&mut self, v: &str) {
        let b = v.as_bytes();
        let n = b.len().min(65535);
        // Cut on a character's boundary.
        let n = (0..=n).rev().find(|&i| v.is_char_boundary(i)).unwrap_or(0);
        self.u16(n as u16);
        self.0.extend_from_slice(&b[..n]);
    }
    fn id(&mut self, v: Option<Id>) {
        self.0.extend_from_slice(&v.unwrap_or_default().0);
    }
    fn lp(&mut self, l: &Loop) {
        self.u8(match l.kind {
            LoopKind::Off => 0,
            LoopKind::Forward => 1,
            LoopKind::PingPong => 2,
        });
        self.u32(l.start);
        self.u32(l.end);
    }
    fn opt(&mut self, v: Option<u8>) {
        match v {
            Some(v) => {
                self.u8(1);
                self.u8(v);
            }
            None => {
                self.u8(0);
                self.u8(0);
            }
        }
    }
    fn env(&mut self, e: &Envelope) {
        self.u8(u8::from(e.enabled) | u8::from(e.carry) << 1 | u8::from(e.filter) << 2);
        self.u8(e.points.len().min(255) as u8);
        for p in e.points.iter().take(255) {
            self.u16(p.tick);
            self.u8(p.value as u8);
        }
        let pair = |p: Option<(u8, u8)>| p.unwrap_or((0xFF, 0xFF));
        let (a, b) = pair(e.looped);
        self.u8(a);
        self.u8(b);
        let (a, b) = pair(e.sustain);
        self.u8(a);
        self.u8(b);
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn cell(&mut self, c: &Cell) {
        self.0.extend_from_slice(&[c.note, c.instr, c.vol as u8, c.volp, c.fx as u8, c.param]);
    }
    fn chunk(&mut self, id: &[u8; 4], data: &[u8]) {
        self.0.extend_from_slice(id);
        self.u32(data.len() as u32);
        self.0.extend_from_slice(data);
    }
}

/// A Musicline song but its samples' points (the module's samples hold them).
fn write_musicline(song: &MlSong) -> Vec<u8> {
    use neumetik_synth::musicline as ml;
    let mut w = W(Vec::new());
    w.u16(song.tunes.len() as u16);
    for t in &song.tunes {
        w.s(&t.title);
        w.u16(t.tempo);
        w.u8(t.speed);
        w.u8(t.groove);
        w.u16(t.volume);
        w.u8(u8::from(t.mixer));
        w.u8(t.sequences.len() as u8);
        for q in &t.sequences {
            for i in 0..256 {
                w.u16(q.get(i).copied().unwrap_or(0x0010));
            }
        }
    }
    // Only what isn't empty: parts and arpeggio lines by number.
    let empty = ml::Line::default();
    let parts: Vec<(usize, &ml::Part)> = song.parts.iter().enumerate().filter(|(_, p)| p.lines.iter().any(|l| *l != empty)).collect();
    w.u16(parts.len() as u16);
    for (n, p) in parts {
        w.u16(n as u16);
        let lines: Vec<(usize, &ml::Line)> = p.lines.iter().enumerate().filter(|(_, l)| **l != empty).collect();
        w.u8(lines.len() as u8);
        for (i, l) in lines {
            w.0.extend_from_slice(&[i as u8, l.note, l.instr]);
            for (e, a) in l.fx {
                w.0.extend_from_slice(&[e, a]);
            }
        }
    }
    let none = ml::ArpLine::default();
    let arps: Vec<(usize, &ml::Arpeggio)> = song.arpeggios.iter().enumerate().filter(|(_, a)| a.lines.iter().any(|l| *l != none)).collect();
    w.u16(arps.len() as u16);
    for (n, a) in arps {
        w.u16(n as u16);
        let lines: Vec<(usize, &ml::ArpLine)> = a.lines.iter().enumerate().filter(|(_, l)| **l != none).collect();
        w.u8(lines.len() as u8);
        for (i, l) in lines {
            w.0.extend_from_slice(&[i as u8, l.transpose as u8, l.sample, l.fx[0].0, l.fx[0].1, l.fx[1].0, l.fx[1].1]);
        }
    }
    w.u16(song.instruments.len().saturating_sub(1) as u16);
    for i in song.instruments.iter().skip(1) {
        w.0.extend_from_slice(&i.raw);
        w.u8(u8::from(i.bound));
    }
    w.u16(song.samples.len().saturating_sub(1) as u16);
    for s in song.samples.iter().skip(1) {
        w.s(&s.name);
        w.u8(s.kind);
        w.u16(s.length);
        w.u32(s.repeat);
        w.u16(s.repeat_length);
        w.u16(s.finetune as u16);
        w.u16(s.semitone as u16);
        w.u32(s.stored);
    }
    w.0
}

fn read_musicline(c: &mut Reader) -> Result<MlSong> {
    use neumetik_synth::musicline as ml;
    let mut song = ml::Song::new();
    for _ in 0..c.u16le()? {
        let title = rs(c)?;
        let (tempo, speed, groove, volume) = (c.u16le()?, c.u8()?, c.u8()?, c.u16le()?);
        let mixer = c.u8()? != 0;
        let n = c.u8()?;
        let mut sequences = Vec::with_capacity(usize::from(n));
        for _ in 0..n {
            sequences.push(c.bytes(512)?.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect());
        }
        song.tunes.push(ml::Tune { title, tempo, speed, groove, volume, mixer, sequences });
    }
    for _ in 0..c.u16le()? {
        let n = usize::from(c.u16le()?);
        let mut part = ml::Part::default();
        for _ in 0..c.u8()? {
            let b = c.bytes(13)?;
            let fx = std::array::from_fn(|k| (b[3 + k * 2], b[4 + k * 2]));
            if let Some(l) = part.lines.get_mut(usize::from(b[0])) {
                *l = ml::Line { note: b[1], instr: b[2], fx };
            }
        }
        if let Some(p) = song.parts.get_mut(n) {
            *p = part;
        }
    }
    for _ in 0..c.u16le()? {
        let n = usize::from(c.u16le()?);
        let mut arp = ml::Arpeggio::default();
        for _ in 0..c.u8()? {
            let b = c.bytes(7)?;
            if let Some(l) = arp.lines.get_mut(usize::from(b[0])) {
                *l = ml::ArpLine { transpose: b[1] as i8, sample: b[2], fx: [(b[3], b[4]), (b[5], b[6])] };
            }
        }
        if let Some(a) = song.arpeggios.get_mut(n) {
            *a = arp;
        }
    }
    for _ in 0..c.u16le()? {
        let raw = c.bytes(ml::INSTRUMENT_BYTES)?;
        let bound = c.u8()? != 0;
        song.instruments.push(ml::Instrument::new(raw, bound));
    }
    for _ in 0..c.u16le()? {
        let name = rs(c)?;
        let kind = c.u8()?;
        let length = c.u16le()?;
        let repeat = c.u32le()?;
        let repeat_length = c.u16le()?;
        let finetune = c.u16le()? as i16;
        let semitone = c.u16le()? as i16;
        let stored = c.u32le()?.min(4 << 20);
        song.samples.push(ml::Wave { name, kind, length, repeat, repeat_length, finetune, semitone, data: Vec::new(), stored });
    }
    Ok(song)
}

fn kind_byte(k: Kind) -> u8 {
    match k {
        Kind::Mod => 0,
        Kind::S3m => 1,
        Kind::Xm => 2,
        Kind::It => 3,
    }
}

/// A song saved with its samples (an NTT) where there's no repository:
/// each sample's and instrument's ID worked out here, as a repository's
/// scan would have.
pub fn write_ntt(module: &Module, about: &About) -> Vec<u8> {
    let refs: Vec<Option<SampleRef>> = module
        .samples
        .iter()
        .map(|s| {
            let canon = canonical_wave(s);
            if s.wave.is_empty() || canon.is_empty() {
                return None;
            }
            let wave = wave_id(&canon);
            Some(SampleRef { sample: sample_id(wave, s), wave })
        })
        .collect();
    let ids = |slot: u16| -> Option<Id> { refs.get(usize::from(slot).checked_sub(1)?).copied().flatten().map(|r| r.sample) };
    let instruments: Vec<Option<Id>> = if module.flags.instruments { module.instruments.iter().map(|i| Some(instrument_id(i, &ids))).collect() } else { Vec::new() };
    write(module, &refs, &instruments, about, true)
}

/// Writes a module as UMF. `refs` gives each sample slot's IDs in the
/// repository (the slots with no waveform have none); `bundle` puts the
/// waveforms in the file as well.
pub fn write(module: &Module, refs: &[Option<SampleRef>], instrument_ids: &[Option<Id>], about: &About, bundle: bool) -> Vec<u8> {
    let mut out = W(Vec::new());
    out.0.extend_from_slice(MAGIC);
    out.u16(VERSION);
    out.u16(u16::from(bundle));

    let mut info = W(Vec::new());
    for s in [&module.title, &module.artist, &module.message, &module.format, &module.format_name, &module.tracker] {
        info.s(s);
    }
    info.u8(kind_byte(module.kind));
    let f = module.flags;
    let bits = [f.linear, f.amiga_limits, f.instruments, f.old_effects, f.compat_gxx, f.fast_slides, f.vblank];
    info.u16(bits.iter().enumerate().fold(0u16, |a, (i, &b)| a | u16::from(b) << i));
    info.u8(module.speed);
    info.u16(module.tempo);
    info.u8(module.global_volume);
    info.u8(module.mix_volume);
    info.u8(module.separation);
    info.u16(module.restart);
    info.id(about.source);
    info.s(&about.repository);
    out.chunk(b"INFO", &info.0);

    let mut ch = W(Vec::new());
    ch.u16(module.channels.len() as u16);
    for c in &module.channels {
        ch.u8(c.pan);
        ch.u8(c.volume);
        ch.u8(u8::from(c.surround) | u8::from(c.muted) << 1);
        ch.s(&c.name);
    }
    out.chunk(b"CHAN", &ch.0);

    let mut ord = W(Vec::new());
    ord.u16(module.orders.len() as u16);
    for &o in &module.orders {
        ord.u16(o);
    }
    out.chunk(b"ORDR", &ord.0);

    let mut pat = W(Vec::new());
    pat.u16(module.patterns.len() as u16);
    for p in &module.patterns {
        pat.u16(p.rows as u16);
        pat.u16(p.channels as u16);
        pat.s(&p.name);
        for c in &p.cells {
            let flags = u8::from(c.note != 0) | u8::from(c.instr != 0) << 1 | u8::from(c.vol != Vol::None) << 2 | u8::from(c.fx != Fx::None) << 3;
            pat.u8(flags);
            if c.note != 0 {
                pat.u8(c.note);
            }
            if c.instr != 0 {
                pat.u8(c.instr);
            }
            if c.vol != Vol::None {
                pat.u8(c.vol as u8);
                pat.u8(c.volp);
            }
            if c.fx != Fx::None {
                pat.u8(c.fx as u8);
                pat.u8(c.param);
            }
        }
    }
    out.chunk(b"PATT", &lz::compress(&pat.0));

    let mut smp = W(Vec::new());
    smp.u16(module.samples.len() as u16);
    for (i, s) in module.samples.iter().enumerate() {
        let r = refs.get(i).copied().flatten();
        let frames = if r.is_some() { audible_end(s) } else { 0 };
        smp.s(&s.name);
        smp.s(&s.filename);
        smp.id(r.map(|r| r.sample));
        smp.id(r.map(|r| r.wave));
        smp.u32(frames as u32);
        smp.u8(s.wave.bits);
        smp.u8(s.wave.channels.max(1));
        smp.u32(s.c5);
        smp.u8(s.volume);
        smp.u8(s.global_volume);
        smp.opt(s.pan);
        smp.lp(&s.looped.clamped(frames as u32));
        smp.lp(&s.sustain.clamped(frames as u32));
        let v = s.vibrato;
        for b in [v.kind, v.speed, v.depth, v.sweep] {
            smp.u8(b);
        }
    }
    out.chunk(b"SMPL", &smp.0);

    let mut ins = W(Vec::new());
    ins.u16(module.instruments.len() as u16);
    for (i, x) in module.instruments.iter().enumerate() {
        ins.id(instrument_ids.get(i).copied().flatten());
        ins.s(&x.name);
        ins.s(&x.filename);
        for n in 0..120 {
            let (note, s) = x.keymap.get(n).copied().unwrap_or((n as u8, 0));
            ins.u8(note);
            ins.u16(s);
        }
        ins.env(&x.volume_env);
        ins.env(&x.pan_env);
        ins.env(&x.pitch_env);
        ins.u32(x.fadeout);
        ins.u8(x.global_volume);
        ins.opt(x.pan);
        for b in [x.nna as u8, x.dct, x.dca, x.pitch_pan_separation as u8, x.pitch_pan_center, x.random_volume, x.random_pan] {
            ins.u8(b);
        }
        ins.opt(x.cutoff);
        ins.opt(x.resonance);
    }
    out.chunk(b"INST", &ins.0);
    write_extras(&mut out, module);

    if bundle {
        let mut done: Vec<Id> = Vec::new();
        for (i, s) in module.samples.iter().enumerate() {
            let Some(r) = refs.get(i).copied().flatten() else { continue };
            if done.contains(&r.wave) {
                continue;
            }
            done.push(r.wave);
            let mut w = W(r.wave.0.to_vec());
            w.0.extend_from_slice(&pack_wave(&canonical_wave(s)));
            out.chunk(b"WAVE", &w.0);
        }
    }
    out.chunk(b"END ", &[]);
    out.0
}

const NONE8: u8 = 0xFF;
const NONE16: u16 = 0xFFFF;

fn target_bytes(t: Option<Target>) -> [u8; 3] {
    match t {
        None => [0, 0, 0],
        Some(Target::GlobalVolume) => [1, 0, 0],
        Some(Target::Tempo) => [2, 0, 0],
        Some(Target::ChannelVolume(c)) => [3, c, 0],
        Some(Target::ChannelPan(c)) => [4, c, 0],
        Some(Target::Effect { slot, param }) => [5, slot, param],
    }
}

fn target_of(b: [u8; 3]) -> Option<Target> {
    match b[0] {
        1 => Some(Target::GlobalVolume),
        2 => Some(Target::Tempo),
        3 => Some(Target::ChannelVolume(b[1])),
        4 => Some(Target::ChannelPan(b[1])),
        5 => Some(Target::Effect { slot: b[1], param: b[2] }),
        _ => None,
    }
}

/// The chunks version 1's first readers didn't have (and pass over):
/// written only when the song uses what they hold.
///
/// ```text
/// SONG   tempo mode, rows a beat and a measure, swing, the order lists, bookmarks, muted matrix slots
/// PATX   each pattern's own time and swing, and its automation
/// CHNX   each channel's effect slot
/// SMPX   each sample's credit, slices and synth
/// SYNV   the FM instruments and synth sounds that play in samples' places
/// PUMA   the Pumatracker instruments that play in samples' places
/// SELS   Face The Music's SEL scripts
/// MLED   a Musicline Editor song (its tunes, parts, arpeggios, instruments; its samples' settings)
/// PTRK   a ProTrekkr song (its patterns, instruments' synths and splits, tracks, effects, TB-303s; not its samples' points, which are the module's)
/// TUNE   a synth or chip song played by its own replayer (AHX, HivelyTracker...): its format and its file
/// INSX   each instrument's envelope release nodes, MIDI output, effect slot and phrase
/// EFCT   the effect slots
/// MACR   the MIDI macros
/// ```
fn write_extras(out: &mut W, m: &Module) {
    let seqs = m.all_sequences();
    if m.tempo_mode != TempoMode::Classic || m.time != TimeSig::default() || m.swing != 0 || seqs.len() > 1 || !m.bookmarks.is_empty() || !m.slot_mutes.is_empty() {
        let mut w = W(Vec::new());
        w.u8(m.tempo_mode as u8);
        w.u16(m.time.rows_per_beat);
        w.u16(m.time.rows_per_measure);
        w.u8(m.swing);
        w.u16(m.sequence.min(seqs.len().saturating_sub(1)) as u16);
        w.u16(seqs.len() as u16);
        for q in &seqs {
            w.s(&q.name);
            w.u16(q.restart);
            w.u16(q.orders.len() as u16);
            for &o in &q.orders {
                w.u16(o);
            }
        }
        w.u16(m.bookmarks.len() as u16);
        for b in &m.bookmarks {
            w.s(&b.name);
            w.u16(b.order.unwrap_or(NONE16));
            w.u16(b.pattern.unwrap_or(NONE16));
            w.u16(b.row);
        }
        w.u16(m.slot_mutes.len() as u16);
        for &(o, c) in &m.slot_mutes {
            w.u16(o);
            w.u16(c);
        }
        out.chunk(b"SONG", &w.0);
    }
    let pats: Vec<(usize, &Arc<Pattern>)> = m.patterns.iter().enumerate().filter(|(_, p)| p.time.is_some() || p.swing.is_some() || !p.automation.is_empty()).collect();
    if !pats.is_empty() {
        let mut w = W(Vec::new());
        w.u16(pats.len() as u16);
        for (i, p) in pats {
            w.u16(i as u16);
            w.u8(u8::from(p.time.is_some()) | u8::from(p.swing.is_some()) << 1);
            if let Some(t) = p.time {
                w.u16(t.rows_per_beat);
                w.u16(t.rows_per_measure);
            }
            if let Some(sw) = p.swing {
                w.u8(sw);
            }
            w.u16(p.automation.len() as u16);
            for lane in &p.automation {
                w.0.extend_from_slice(&target_bytes(lane.target));
                w.u16(lane.points.len() as u16);
                for &(row, v) in &lane.points {
                    w.u16(row);
                    w.f32(v);
                }
            }
        }
        out.chunk(b"PATX", &w.0);
    }
    if m.channels.iter().any(|c| c.effect.is_some()) {
        let mut w = W(Vec::new());
        w.u16(m.channels.len() as u16);
        for c in &m.channels {
            w.u8(c.effect.unwrap_or(NONE8));
        }
        out.chunk(b"CHNX", &w.0);
    }
    if m.samples.iter().any(|s| !s.credit.is_empty() || !s.slices.is_empty() || s.synth.is_some()) {
        let mut w = W(Vec::new());
        w.u16(m.samples.len() as u16);
        for s in &m.samples {
            w.s(&s.credit);
            w.u32(s.slices.len() as u32);
            for &x in &s.slices {
                w.u32(x);
            }
            match s.synth {
                Some(y) => {
                    w.u8(1);
                    for b in [y.wave, y.width, y.ratio, y.depth, y.beat, y.soften] {
                        w.u8(b);
                    }
                }
                None => w.u8(0),
            }
        }
        out.chunk(b"SMPX", &w.0);
    }
    let med = |s: &Sample| s.program.as_ref().is_some_and(|p| matches!(p.sound, Sound::Med(_) | Sound::Jam));
    if m.samples.iter().any(|s| s.fm.is_some() || med(s)) {
        let mut w = W(Vec::new());
        let voiced: Vec<(usize, &Sample)> = m.samples.iter().enumerate().filter(|(_, s)| s.fm.is_some() || med(s)).collect();
        w.u16(voiced.len() as u16);
        for (i, s) in voiced {
            w.u16(i as u16);
            if let Some(p) = s.fm {
                w.u8(1);
                w.0.extend_from_slice(&p.to_s3m());
            } else if let Some(Sound::Med(y)) = s.program.as_ref().map(|p| &p.sound) {
                w.u8(2);
                w.u8(u8::from(y.hybrid));
                w.u8(y.vol_speed);
                w.u8(y.wave_speed);
                for t in [&y.volumes, &y.waves] {
                    w.u16(t.len() as u16);
                    w.0.extend_from_slice(t);
                }
                w.u16(y.waveforms.len() as u16);
                for f in &y.waveforms {
                    w.u16(f.len() as u16);
                    w.0.extend(f.iter().map(|&x| x as u8));
                }
            } else {
                // A JamCracker AM wave: the sample's bytes are all it needs.
                w.u8(3);
            }
        }
        out.chunk(b"SYNV", &w.0);
    }
    let puma: Vec<(usize, &Program, &PumaSound)> = m
        .samples
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let p = s.program.as_deref()?;
            match &p.sound {
                Sound::Puma(y) => Some((i, p, y)),
                Sound::Med(_) | Sound::Jam => None,
            }
        })
        .collect();
    if !puma.is_empty() {
        let mut w = W(Vec::new());
        w.u16(puma.len() as u16);
        for (i, p, y) in puma {
            w.u16(i as u16);
            w.u16(p.sources.len() as u16);
            for &s in &p.sources {
                w.u16(s);
            }
            for script in [&y.volume, &y.pitch] {
                w.u16(script.len() as u16);
                for c in script.iter() {
                    w.0.extend_from_slice(c);
                }
            }
        }
        out.chunk(b"PUMA", &w.0);
    }
    if let Some(sel) = m.sel.as_deref().filter(|s| !s.is_empty()) {
        let mut w = W(Vec::new());
        w.u16(sel.rows_per_measure);
        let defined: Vec<(usize, &Vec<SelLine>)> = sel.scripts.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|s| (i, s))).collect();
        w.u8(defined.len() as u8);
        for (i, lines) in defined {
            w.u8(i as u8);
            w.u16(lines.len().min(0x200) as u16);
            for l in lines.iter().take(0x200) {
                w.u8(l.op);
                w.0.extend_from_slice(&l.data);
            }
        }
        out.chunk(b"SELS", &w.0);
    }
    if let Some(song) = m.ptk.as_deref() {
        out.chunk(b"PTRK", &lz::compress(&song.to_bytes()));
    }
    if let Some(song) = m.musicline.as_deref() {
        out.chunk(b"MLED", &write_musicline(song));
    }
    if let Some(t) = &m.tune {
        let id = t.0.format().as_bytes();
        let mut w = W(Vec::new());
        w.u8(id.len() as u8);
        w.0.extend_from_slice(id);
        w.0.extend_from_slice(&lz::compress(t.0.file()));
        out.chunk(b"TUNE", &w.0);
    }
    if m.instruments.iter().any(|i| i.midi.is_some() || i.effect.is_some() || i.phrase.is_some() || [&i.volume_env, &i.pan_env, &i.pitch_env].iter().any(|e| e.release.is_some())) {
        let mut w = W(Vec::new());
        w.u16(m.instruments.len() as u16);
        for i in &m.instruments {
            for e in [&i.volume_env, &i.pan_env, &i.pitch_env] {
                w.u8(e.release.unwrap_or(NONE8));
            }
            match i.midi {
                Some(o) => {
                    w.u8(1);
                    w.u8(o.channel);
                    w.u8(o.program.unwrap_or(NONE8));
                    w.u16(o.bank.unwrap_or(NONE16));
                }
                None => w.u8(0),
            }
            w.u8(i.effect.unwrap_or(NONE8));
            match &i.phrase {
                Some(p) => {
                    w.u8(1);
                    w.u8(u8::from(p.looped));
                    w.u8(p.base);
                    w.u16(p.cells.len() as u16);
                    for c in &p.cells {
                        w.cell(c);
                    }
                }
                None => w.u8(0),
            }
        }
        out.chunk(b"INSX", &w.0);
    }
    if !m.effects.is_empty() {
        let mut w = W(Vec::new());
        w.u16(m.effects.len() as u16);
        for e in &m.effects {
            w.u8(e.kind as u8);
            w.s(&e.name);
            w.f32(e.mix);
            w.u8(u8::from(e.bypass));
            w.u8(e.output.unwrap_or(NONE8));
            w.u8(e.params.len().min(255) as u8);
            for &p in e.params.iter().take(255) {
                w.f32(p);
            }
        }
        out.chunk(b"EFCT", &w.0);
    }
    if let Some(mac) = &m.macros {
        let mut w = W(Vec::new());
        for i in 0..16 {
            w.s(mac.parametered.get(i).map_or("", |s| s.as_str()));
        }
        for i in 0..128 {
            w.s(mac.fixed.get(i).map_or("", |s| s.as_str()));
        }
        out.chunk(b"MACR", &w.0);
    }
    if !m.visual.is_empty() {
        let mut w = W(Vec::new());
        w.s(&m.visual);
        out.chunk(b"VISL", &w.0);
    }
    // The cover: the song's own, or one painted for it, so every UMF has one
    // (without the painter, neumetik-visual, only a song's own is kept).
    #[cfg(not(feature = "visual"))]
    if let Some(cover) = &m.cover {
        out.chunk(b"COVR", &lz::compress(cover));
    }
}

/// The most a cover can unpack to: a picture far past 80 by 25 (a long
/// ANSI, or an ANSImation).
const COVER_MOST: usize = 4 << 20;

fn rcell(r: &mut Reader) -> Result<Cell> {
    let b = r.bytes(6)?;
    Ok(Cell { note: b[0], instr: b[1], vol: Vol::from_u8(b[2]), volp: b[3], fx: Fx::from_u8(b[4]), param: b[5] })
}

fn some8(v: u8) -> Option<u8> {
    (v != NONE8).then_some(v)
}

fn some16(v: u16) -> Option<u16> {
    (v != NONE16).then_some(v)
}

/// Reads one of the chunks `write_extras` writes into the module (read
/// after the song's own chunks, which come first).
/// A pattern's extras as PATX has them: its index, time, swing and automation.
type PatternExtras = (usize, Option<TimeSig>, Option<u8>, Vec<Lane>);

fn read_extra(id: &[u8; 4], c: &mut Reader, m: &mut Module, patx: &mut Vec<PatternExtras>) -> Result<()> {
    match id {
        b"SONG" => {
            m.tempo_mode = TempoMode::from_u8(c.u8()?);
            m.time = TimeSig { rows_per_beat: c.u16le()?.max(1), rows_per_measure: c.u16le()?.max(1) };
            m.swing = c.u8()?.min(100);
            let active = usize::from(c.u16le()?);
            let n = c.u16le()?;
            let mut seqs = Vec::new();
            for _ in 0..n {
                let name = rs(c)?;
                let restart = c.u16le()?;
                let k = c.u16le()?;
                let orders = (0..k).map(|_| c.u16le()).collect::<Result<Vec<u16>>>()?;
                seqs.push(Sequence { name, orders, restart });
            }
            if seqs.len() > 1 {
                m.sequence = active.min(seqs.len() - 1);
                m.sequences = seqs;
            }
            let n = c.u16le()?;
            for _ in 0..n {
                let name = rs(c)?;
                let order = some16(c.u16le()?);
                let pattern = some16(c.u16le()?);
                let row = c.u16le()?;
                m.bookmarks.push(Bookmark { name, order, pattern, row });
            }
            let n = c.u16le()?;
            for _ in 0..n {
                m.slot_mutes.push((c.u16le()?, c.u16le()?));
            }
        }
        b"PATX" => {
            let n = c.u16le()?;
            for _ in 0..n {
                let i = usize::from(c.u16le()?);
                let f = c.u8()?;
                let time = if f & 1 != 0 { Some(TimeSig { rows_per_beat: c.u16le()?.max(1), rows_per_measure: c.u16le()?.max(1) }) } else { None };
                let swing = if f & 2 != 0 { Some(c.u8()?.min(100)) } else { None };
                let lanes = c.u16le()?;
                let mut automation = Vec::new();
                for _ in 0..lanes {
                    let t = c.bytes(3)?;
                    let target = target_of([t[0], t[1], t[2]]);
                    let k = c.u16le()?;
                    let mut points = Vec::with_capacity(usize::from(k));
                    for _ in 0..k {
                        points.push((c.u16le()?, c.f32le()?.clamp(0.0, 1.0)));
                    }
                    automation.push(Lane { target, points });
                }
                patx.push((i, time, swing, automation));
            }
        }
        b"CHNX" => {
            let n = usize::from(c.u16le()?);
            for i in 0..n {
                let e = some8(c.u8()?);
                if let Some(ch) = m.channels.get_mut(i) {
                    ch.effect = e;
                }
            }
        }
        b"SMPX" => {
            let n = usize::from(c.u16le()?);
            for i in 0..n {
                let credit = rs(c)?;
                let k = c.u32le()? as usize;
                if k > c.left() / 4 {
                    return Err(Error::Bad("its sample slices are damaged".into()));
                }
                let slices = (0..k).map(|_| c.u32le()).collect::<Result<Vec<u32>>>()?;
                let synth = if c.u8()? != 0 {
                    let b = c.bytes(6)?;
                    Some(Synth { wave: b[0], width: b[1], ratio: b[2], depth: b[3], beat: b[4], soften: b[5] })
                } else {
                    None
                };
                if let Some(s) = m.samples.get_mut(i) {
                    s.credit = credit;
                    s.slices = slices;
                    s.synth = synth;
                }
            }
        }
        b"INSX" => {
            let n = usize::from(c.u16le()?);
            for i in 0..n {
                let rel = [some8(c.u8()?), some8(c.u8()?), some8(c.u8()?)];
                let midi = if c.u8()? != 0 { Some(MidiOut { channel: c.u8()? & 15, program: some8(c.u8()?).map(|p| p & 127), bank: some16(c.u16le()?) }) } else { None };
                let effect = some8(c.u8()?);
                let phrase = if c.u8()? != 0 {
                    let looped = c.u8()? != 0;
                    let base = c.u8()?;
                    let k = c.u16le()?;
                    let cells = (0..k).map(|_| rcell(c)).collect::<Result<Vec<Cell>>>()?;
                    Some(Phrase { cells, looped, base })
                } else {
                    None
                };
                if let Some(x) = m.instruments.get_mut(i) {
                    for (e, r) in [&mut x.volume_env, &mut x.pan_env, &mut x.pitch_env].into_iter().zip(rel) {
                        e.release = r.filter(|&r| usize::from(r) < e.points.len());
                    }
                    x.midi = midi;
                    x.effect = effect;
                    x.phrase = phrase;
                }
            }
        }
        b"SYNV" => {
            let n = c.u16le()?;
            for _ in 0..n {
                let i = usize::from(c.u16le()?);
                let (fm, program) = match c.u8()? {
                    1 => (Some(FmPatch::from_s3m(&c.bytes(11)?.try_into().expect("eleven bytes"))), None),
                    2 => {
                        let hybrid = c.u8()? != 0;
                        let (vol_speed, wave_speed) = (c.u8()?, c.u8()?);
                        let mut table = || -> Result<Vec<u8>> {
                            let k = usize::from(c.u16le()?);
                            Ok(c.bytes(k)?.to_vec())
                        };
                        let volumes = table()?;
                        let waves = table()?;
                        let k = usize::from(c.u16le()?);
                        let mut waveforms = Vec::with_capacity(k.min(64));
                        for _ in 0..k {
                            let len = usize::from(c.u16le()?);
                            waveforms.push(c.bytes(len)?.iter().map(|&b| b as i8).collect());
                        }
                        // A hybrid's sample is put in once the waveforms are read.
                        let sound = SynthSound { waveforms, volumes, waves, vol_speed, wave_speed, hybrid };
                        (None, Some(Arc::new(Program::new(sound, &Arc::default()))))
                    }
                    // Its waves are made once the sample is read.
                    3 => (None, Some(Arc::new(Program::jam(&Wave::default())))),
                    _ => return Err(Error::Bad("its synth voices are damaged".into())),
                };
                if let Some(s) = m.samples.get_mut(i) {
                    s.fm = fm;
                    s.program = program;
                }
            }
        }
        b"PUMA" => {
            let n = c.u16le()?;
            for _ in 0..n {
                let i = usize::from(c.u16le()?);
                let k = usize::from(c.u16le()?);
                let sources: Vec<u16> = (0..k).map(|_| c.u16le()).collect::<Result<_>>()?;
                let mut script = || -> Result<Vec<[u8; 4]>> {
                    let k = usize::from(c.u16le()?);
                    Ok(c.bytes(k * 4)?.chunks_exact(4).map(|b| [b[0], b[1], b[2], b[3]]).collect())
                };
                let volume = script()?;
                let pitch = script()?;
                // Its samples are put in once they're read.
                let program = Program::puma(PumaSound { volume, pitch }, sources, &[]);
                if let Some(s) = m.samples.get_mut(i) {
                    s.program = Some(Arc::new(program));
                }
            }
        }
        b"MLED" => m.musicline = Some(Arc::new(read_musicline(c)?)),
        b"TUNE" => {
            let n = usize::from(c.u8()?);
            let id = String::from_utf8_lossy(c.bytes(n)?).into_owned();
            let file = lz::decompress(c.bytes(c.left())?, 64 << 20).map_err(|_| Error::Bad("its synth song is damaged".into()))?;
            let tune = neumetik_synth::replay::open(&id, &file).ok_or_else(|| Error::Bad("its synth song is damaged".into()))?;
            m.tune = Some(Tune(tune));
        }
        b"SELS" => {
            let mut sel = SelProgram::new(c.u16le()?);
            let n = c.u8()?;
            for _ in 0..n {
                let i = usize::from(c.u8()?);
                let k = usize::from(c.u16le()?).min(0x200);
                let lines = c.bytes(k * 4)?.chunks_exact(4).map(|b| SelLine { op: b[0], data: [b[1], b[2], b[3]] }).collect();
                if i < 64 {
                    sel.scripts[i] = Some(lines);
                }
            }
            m.sel = Some(Arc::new(sel));
        }
        b"EFCT" => {
            let n = c.u16le()?;
            for _ in 0..n {
                let kind = EffectKind::from_u8(c.u8()?);
                let name = rs(c)?;
                let mix = c.f32le()?.clamp(0.0, 1.0);
                let bypass = c.u8()? != 0;
                let output = some8(c.u8()?);
                let k = c.u8()?;
                let params = (0..k).map(|_| c.f32le().map(|v| v.clamp(0.0, 1.0))).collect::<Result<Vec<f32>>>()?;
                m.effects.push(EffectSlot { kind, name, params, mix, bypass, output });
            }
        }
        b"VISL" => m.visual = rs(c)?,
        b"COVR" => {
            let body = c.bytes(c.left())?;
            m.cover = lz::decompress(body, COVER_MOST).ok().filter(|b| !b.is_empty()).map(Arc::new);
        }
        b"MACR" => {
            let parametered = (0..16).map(|_| rs(c)).collect::<Result<Vec<String>>>()?;
            let fixed = (0..128).map(|_| rs(c)).collect::<Result<Vec<String>>>()?;
            m.macros = Some(MidiMacros { parametered, fixed });
        }
        _ => {}
    }
    Ok(())
}

/// A sample slot as the SMPL chunk has it.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotEntry {
    pub name: String,
    pub filename: String,
    pub refs: Option<SampleRef>,
    pub frames: u32,
    pub bits: u8,
    pub channels: u8,
    pub c5: u32,
    pub volume: u8,
    pub global_volume: u8,
    pub pan: Option<u8>,
    pub looped: Loop,
    pub sustain: Loop,
    pub vibrato: [u8; 4],
}

fn read_slots(body: &[u8]) -> Result<Vec<SlotEntry>> {
    let mut c = Reader::new(body);
    let n = c.u16le()?;
    let mut v = Vec::with_capacity(usize::from(n));
    for _ in 0..n {
        let name = rs(&mut c)?;
        let filename = rs(&mut c)?;
        let sid = rid(&mut c)?;
        let wid = rid(&mut c)?;
        v.push(SlotEntry {
            name,
            filename,
            refs: sid.zip(wid).map(|(sample, wave)| SampleRef { sample, wave }),
            frames: c.u32le()?,
            bits: c.u8()?,
            channels: c.u8()?,
            c5: c.u32le()?,
            volume: c.u8()?,
            global_volume: c.u8()?,
            pan: ropt(&mut c)?,
            looped: rloop(&mut c)?,
            sustain: rloop(&mut c)?,
            vibrato: [c.u8()?, c.u8()?, c.u8()?, c.u8()?],
        });
    }
    Ok(v)
}

fn write_slots(v: &[SlotEntry]) -> Vec<u8> {
    let mut w = W(Vec::new());
    w.u16(v.len() as u16);
    for e in v {
        w.s(&e.name);
        w.s(&e.filename);
        w.id(e.refs.map(|r| r.sample));
        w.id(e.refs.map(|r| r.wave));
        w.u32(e.frames);
        w.u8(e.bits);
        w.u8(e.channels);
        w.u32(e.c5);
        w.u8(e.volume);
        w.u8(e.global_volume);
        w.opt(e.pan);
        w.lp(&e.looped);
        w.lp(&e.sustain);
        for b in e.vibrato {
            w.u8(b);
        }
    }
    w.0
}

/// The file's chunks, in order: (id, body).
/// A file's version, flags and chunks.
type Chunks<'a> = (u16, u16, Vec<([u8; 4], &'a [u8])>);

fn chunks(data: &[u8]) -> Result<Chunks<'_>> {
    if !probe(data) {
        return Err(Error::Unknown);
    }
    let mut r = Reader::new(data);
    r.skip(4)?;
    let version = r.u16le()?;
    let flags = r.u16le()?;
    let mut v = Vec::new();
    while r.left() >= 8 {
        let id = r.tag()?;
        let len = r.u32le()? as usize;
        let body = r.bytes(len)?;
        if &id == b"END " {
            break;
        }
        v.push((id, body));
    }
    Ok((version, flags, v))
}

/// The samples a UMF file plays, by slot, without reading the rest.
pub fn slots(data: &[u8]) -> Result<Vec<SlotEntry>> {
    let (_, _, cs) = chunks(data)?;
    cs.iter().find(|(id, _)| id == b"SMPL").map_or(Ok(Vec::new()), |(_, b)| read_slots(b))
}

/// A UMF file with its sample slots changed by `change` (which says
/// whether it changed one); `waves` gives a waveform's packed file for
/// one a file that carries its waveforms needs and hasn't. Returns the
/// new file, or None when nothing changed.
pub fn rewrite_slots(data: &[u8], change: &mut dyn FnMut(&mut SlotEntry) -> bool, waves: &mut dyn FnMut(&Id) -> Option<Vec<u8>>) -> Result<Option<Vec<u8>>> {
    let (version, flags, cs) = chunks(data)?;
    let Some(body) = cs.iter().find(|(id, _)| id == b"SMPL").map(|(_, b)| *b) else { return Ok(None) };
    let mut v = read_slots(body)?;
    let mut changed = false;
    for e in &mut v {
        changed |= change(e);
    }
    if !changed {
        return Ok(None);
    }
    let mut out = W(Vec::new());
    out.0.extend_from_slice(MAGIC);
    out.u16(version);
    out.u16(flags);
    let mut have: Vec<Id> = Vec::new();
    for (id, b) in &cs {
        if id == b"SMPL" {
            out.chunk(b"SMPL", &write_slots(&v));
        } else {
            if id == b"WAVE" && b.len() > 32 {
                have.push(Id(b[..32].try_into().expect("32 bytes")));
            }
            out.chunk(id, b);
        }
    }
    // A file that carries its waveforms carries the new ones too.
    if flags & 1 != 0 {
        for e in &v {
            if let Some(r) = e.refs {
                if !have.contains(&r.wave) {
                    if let Some(packed) = waves(&r.wave) {
                        let mut w = W(r.wave.0.to_vec());
                        w.0.extend_from_slice(&packed);
                        out.chunk(b"WAVE", &w.0);
                        have.push(r.wave);
                    }
                }
            }
        }
    }
    out.chunk(b"END ", &[]);
    Ok(Some(out.0))
}

pub fn probe(data: &[u8]) -> bool {
    data.len() >= 8 && &data[..4] == MAGIC
}

fn rs(r: &mut Reader) -> Result<String> {
    let n = usize::from(r.u16le()?);
    Ok(String::from_utf8_lossy(r.bytes(n)?).into_owned())
}

fn rid(r: &mut Reader) -> Result<Option<Id>> {
    let b = r.bytes(32)?;
    let id = Id(b.try_into().expect("32 bytes"));
    Ok((!id.is_zero()).then_some(id))
}

fn rloop(r: &mut Reader) -> Result<Loop> {
    let kind = LoopKind::from_u8(r.u8()?);
    Ok(Loop { kind, start: r.u32le()?, end: r.u32le()? })
}

fn ropt(r: &mut Reader) -> Result<Option<u8>> {
    let (on, v) = (r.u8()?, r.u8()?);
    Ok((on != 0).then_some(v))
}

fn renv(r: &mut Reader) -> Result<Envelope> {
    let flags = r.u8()?;
    let n = r.u8()?;
    let mut points = Vec::with_capacity(usize::from(n));
    for _ in 0..n {
        points.push(EnvPoint { tick: r.u16le()?, value: r.i8()? });
    }
    let pair = |a: u8, b: u8| (a != 0xFF && b != 0xFF && a < n && b < n).then_some((a, b));
    let looped = pair(r.u8()?, r.u8()?);
    let sustain = pair(r.u8()?, r.u8()?);
    Ok(Envelope { enabled: flags & 1 != 0, carry: flags & 2 != 0, filter: flags & 4 != 0, points, looped, sustain, release: None })
}

/// Reads a UMF file. Each sample's waveform comes from the file when it
/// has it, else from `find` (the repository), and must match its ID.
pub fn read(data: &[u8], find: &mut dyn FnMut(&Id) -> Option<Wave>) -> Result<Loaded> {
    if !probe(data) {
        return Err(Error::Unknown);
    }
    let mut r = Reader::new(data);
    r.skip(4)?;
    let version = r.u16le()?;
    if version > VERSION {
        return Err(Error::Unsupported("it was saved by a newer Neumetik".into()));
    }
    let flags = r.u16le()?;
    let mut m = Module { format: "umf".into(), ..Default::default() };
    let mut about = About { bundled: flags & 1 != 0, ..Default::default() };
    let mut refs: Vec<Option<SampleRef>> = Vec::new();
    let mut frames_of: Vec<(u32, u8, u8)> = Vec::new();
    let mut instrument_ids = Vec::new();
    let mut waves: HashMap<Id, &[u8]> = HashMap::new();
    let mut extras: Vec<([u8; 4], &[u8])> = Vec::new();
    while r.left() >= 8 {
        let id = r.tag()?;
        let len = r.u32le()? as usize;
        let body = r.bytes(len)?;
        let mut c = Reader::new(body);
        match &id {
            b"INFO" => {
                m.title = rs(&mut c)?;
                m.artist = rs(&mut c)?;
                m.message = rs(&mut c)?;
                let original = rs(&mut c)?;
                m.format_name = rs(&mut c)?;
                m.tracker = rs(&mut c)?;
                // It's UMF now; where it came from stays in its words.
                if original != "umf" {
                    m.format_name = if m.format_name.is_empty() { format!("UMF, from {}", original.to_uppercase()) } else { format!("UMF, from {}", m.format_name) };
                }
                m.kind = match c.u8()? {
                    1 => Kind::S3m,
                    2 => Kind::Xm,
                    3 => Kind::It,
                    _ => Kind::Mod,
                };
                let f = c.u16le()?;
                let b = |i: u16| f & (1 << i) != 0;
                m.flags = Flags { linear: b(0), amiga_limits: b(1), instruments: b(2), old_effects: b(3), compat_gxx: b(4), fast_slides: b(5), vblank: b(6) };
                m.speed = c.u8()?;
                m.tempo = c.u16le()?;
                m.global_volume = c.u8()?;
                m.mix_volume = c.u8()?;
                m.separation = c.u8()?;
                m.restart = c.u16le()?;
                about.source = rid(&mut c)?;
                about.repository = rs(&mut c)?;
            }
            b"CHAN" => {
                let n = c.u16le()?;
                for _ in 0..n {
                    let pan = c.u8()?;
                    let volume = c.u8()?;
                    let f = c.u8()?;
                    let name = rs(&mut c)?;
                    m.channels.push(Channel { pan, volume, surround: f & 1 != 0, muted: f & 2 != 0, name, effect: None });
                }
            }
            b"ORDR" => {
                let n = c.u16le()?;
                for _ in 0..n {
                    m.orders.push(c.u16le()?);
                }
            }
            b"PATT" => {
                let raw = lz::decompress(body, 256 << 20).map_err(|_| Error::Bad("its patterns are damaged".into()))?;
                let mut p = Reader::new(&raw);
                let n = p.u16le()?;
                for _ in 0..n {
                    let rows = usize::from(p.u16le()?);
                    let chans = usize::from(p.u16le()?);
                    let name = rs(&mut p)?;
                    let mut pat = Pattern::new(rows, chans);
                    pat.name = name;
                    for cell in pat.cells.iter_mut() {
                        let f = p.u8()?;
                        if f & 1 != 0 {
                            cell.note = p.u8()?;
                        }
                        if f & 2 != 0 {
                            cell.instr = p.u8()?;
                        }
                        if f & 4 != 0 {
                            cell.vol = Vol::from_u8(p.u8()?);
                            cell.volp = p.u8()?;
                        }
                        if f & 8 != 0 {
                            cell.fx = Fx::from_u8(p.u8()?);
                            cell.param = p.u8()?;
                        }
                    }
                    m.patterns.push(Arc::new(pat));
                }
            }
            b"SMPL" => {
                let n = c.u16le()?;
                for _ in 0..n {
                    let mut s = Sample::empty();
                    s.name = rs(&mut c)?;
                    s.filename = rs(&mut c)?;
                    let sid = rid(&mut c)?;
                    let wid = rid(&mut c)?;
                    let frames = c.u32le()?;
                    let bits = c.u8()?;
                    let chans = c.u8()?;
                    s.c5 = c.u32le()?;
                    s.volume = c.u8()?;
                    s.global_volume = c.u8()?;
                    s.pan = ropt(&mut c)?;
                    s.looped = rloop(&mut c)?;
                    s.sustain = rloop(&mut c)?;
                    s.vibrato = AutoVibrato { kind: c.u8()?, speed: c.u8()?, depth: c.u8()?, sweep: c.u8()? };
                    refs.push(match (sid, wid) {
                        (Some(sample), Some(wave)) => Some(SampleRef { sample, wave }),
                        _ => None,
                    });
                    frames_of.push((frames, bits, chans));
                    m.samples.push(s);
                }
            }
            b"INST" => {
                let n = c.u16le()?;
                for _ in 0..n {
                    let iid = rid(&mut c)?;
                    let mut x = Instrument { name: rs(&mut c)?, filename: rs(&mut c)?, ..Default::default() };
                    for _ in 0..120 {
                        let note = c.u8()?;
                        let s = c.u16le()?;
                        x.keymap.push((note.min(119), s));
                    }
                    x.volume_env = renv(&mut c)?;
                    x.pan_env = renv(&mut c)?;
                    x.pitch_env = renv(&mut c)?;
                    x.fadeout = c.u32le()?;
                    x.global_volume = c.u8()?;
                    x.pan = ropt(&mut c)?;
                    x.nna = Nna::from_u8(c.u8()?);
                    x.dct = c.u8()?;
                    x.dca = c.u8()?;
                    x.pitch_pan_separation = c.i8()?;
                    x.pitch_pan_center = c.u8()?;
                    x.random_volume = c.u8()?;
                    x.random_pan = c.u8()?;
                    x.cutoff = ropt(&mut c)?;
                    x.resonance = ropt(&mut c)?;
                    instrument_ids.push(iid);
                    m.instruments.push(x);
                }
            }
            b"WAVE" => {
                if body.len() > 32 {
                    let id = Id(body[..32].try_into().expect("32 bytes"));
                    waves.insert(id, &body[32..]);
                }
            }
            b"END " => break,
            // The extras, once the song they add to is read.
            _ => extras.push((id, body)),
        }
    }
    let mut patx = Vec::new();
    let mut ptk = None;
    for (id, body) in extras {
        if &id == b"PTRK" {
            // Put together once the samples it plays are in.
            ptk = Some(body);
            continue;
        }
        read_extra(&id, &mut Reader::new(body), &mut m, &mut patx)?;
    }
    for (i, time, swing, automation) in patx {
        if let Some(p) = m.patterns.get_mut(i) {
            let p = Arc::make_mut(p);
            p.time = time;
            p.swing = swing;
            p.automation = automation;
        }
    }
    // The waveforms: the file's own first, then the repository's.
    let mut missing = Vec::new();
    let mut cache: HashMap<Id, Arc<Wave>> = HashMap::new();
    for (i, s) in m.samples.iter_mut().enumerate() {
        let Some(r) = refs[i] else { continue };
        let (frames, bits, chans) = frames_of[i];
        let found = cache.get(&r.wave).cloned().or_else(|| {
            let w = waves.get(&r.wave).and_then(|b| unpack_wave(b, Some(r.wave)).ok()).or_else(|| find(&r.wave))?;
            let w = Arc::new(w);
            cache.insert(r.wave, w.clone());
            Some(w)
        });
        match found.filter(|w| w.frames() == frames as usize) {
            Some(w) => s.wave = w,
            None => {
                s.wave = Arc::new(Wave { bits, channels: chans.max(1), pcm: Vec::new() });
                missing.push(((i + 1) as u16, r.wave));
            }
        }
    }
    for s in &mut m.samples {
        if let Some(p) = s.program.as_mut().filter(|p| p.hybrid() && !p.waves.is_empty()) {
            Arc::make_mut(p).waves[0] = s.wave.clone();
        }
    }
    for s in &mut m.samples {
        if let Some(p) = s.program.as_mut().filter(|p| p.sound == Sound::Jam) {
            *p = Arc::new(Program::jam(&s.wave));
        }
    }
    let all = m.samples.clone();
    for s in &mut m.samples {
        if let Some(p) = s.program.as_mut().filter(|p| !p.sources.is_empty()) {
            Arc::make_mut(p).attach(&all);
        }
    }
    // A Musicline song's samples are the module's, in order.
    if let Some(song) = m.musicline.as_mut() {
        let song = Arc::make_mut(song);
        for (w, smp) in song.samples.iter_mut().skip(1).zip(&all) {
            let pts: Vec<i8> = smp.wave.pcm.iter().map(|&x| (x >> 8) as i8).collect();
            w.attach(&pts);
        }
    }
    // A ProTrekkr song's splits are the module's samples, in order.
    if let Some(body) = ptk {
        let mut points = |n: usize| -> (Arc<Vec<i16>>, Option<Arc<Vec<i16>>>) {
            let Some(w) = all.get(n).map(|s| &s.wave) else { return (Arc::new(Vec::new()), None) };
            if w.channels == 2 {
                let (l, r): (Vec<i16>, Vec<i16>) = w.pcm.chunks_exact(2).map(|p| (p[0], p[1])).unzip();
                (Arc::new(l), Some(Arc::new(r)))
            } else {
                (Arc::new(w.pcm.clone()), None)
            }
        };
        let raw = lz::decompress(body, 256 << 20).map_err(|_| Error::Bad("its ProTrekkr song is damaged".into()))?;
        match PtkSong::from_bytes(&raw, &mut points) {
            Some(song) => m.ptk = Some(Arc::new(song)),
            None => return Err(Error::Bad("its ProTrekkr song is damaged".into())),
        }
    }
    m.tidy();
    Ok(Loaded { module: m, refs, instrument_ids, about, missing })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::{sample_id, wave_id};

    fn song() -> Module {
        let mut m = Module::new(4);
        m.title = "Test song ♪".into();
        m.message = "line one\nline two".into();
        let wave = Wave::new(16, 1, (0..1000).map(|i| (i * 13 % 2000 - 1000) as i16).chain([0, 0, 0]).collect());
        m.samples.push(Sample { name: "lead".into(), wave: Arc::new(wave), looped: Loop::forward(100, 1000), c5: 22050, volume: 48, ..Sample::empty() });
        m.samples.push(Sample { name: "just a name".into(), ..Sample::empty() });
        m.flags.instruments = true;
        let mut ins = Instrument::for_sample("lead", 1);
        ins.volume_env = Envelope { enabled: true, points: vec![EnvPoint { tick: 0, value: 64 }, EnvPoint { tick: 20, value: 0 }], sustain: Some((0, 0)), ..Default::default() };
        m.instruments.push(ins);
        let mut p = Pattern::new(64, 4);
        *p.cell_mut(0, 0).unwrap() = Cell { note: 61, instr: 1, vol: Vol::Volume, volp: 40, fx: Fx::Vibrato, param: 0x44 };
        *p.cell_mut(16, 3).unwrap() = Cell { note: NOTE_OFF, ..Default::default() };
        m.patterns = vec![Arc::new(p)];
        m
    }

    fn refs(m: &Module) -> Vec<Option<SampleRef>> {
        m.samples
            .iter()
            .map(|s| {
                (!s.wave.is_empty()).then(|| {
                    let w = wave_id(&canonical_wave(s));
                    SampleRef { sample: sample_id(w, s), wave: w }
                })
            })
            .collect()
    }

    #[test]
    fn an_ntt_written_without_a_repository_reads_back_with_its_samples() {
        let mut m = Module::new(2);
        let saw: Vec<i16> = (0..2000).map(|i| ((i % 100) * 600 - 30000) as i16).collect();
        m.samples = vec![Sample { name: "saw".into(), wave: Arc::new(Wave::new(16, 1, saw)), ..Sample::empty() }, Sample::empty()];
        let back = read(&write_ntt(&m, &About::default()), &mut |_| None).expect("reads back");
        assert!(back.missing.is_empty());
        assert_eq!(back.module.samples[0].wave.pcm, m.samples[0].wave.pcm);
        assert!(back.module.samples[1].wave.is_empty());
    }


    #[test]
    fn a_song_round_trips_with_its_samples_in_the_repository() {
        let m = song();
        let r = refs(&m);
        let bytes = write(&m, &r, &[None], &About { repository: "abc".into(), ..Default::default() }, false);
        let canon = canonical_wave(&m.samples[0]);
        let mut asked = 0;
        let back = read(&bytes, &mut |id| {
            asked += 1;
            (*id == r[0].unwrap().wave).then(|| canon.clone())
        })
        .unwrap();
        assert_eq!(asked, 1);
        assert!(back.missing.is_empty());
        assert_eq!(back.module.patterns, m.patterns);
        assert_eq!(back.module.title, m.title);
        assert_eq!(back.module.instruments, m.instruments);
        assert_eq!(back.module.samples[0].wave.pcm, canon.pcm);
        assert_eq!(back.module.samples[0].looped, m.samples[0].looped);
        assert_eq!(back.module.samples[1].name, "just a name");
        assert_eq!(back.about.repository, "abc");
    }

    #[test]
    fn bundled_it_plays_with_no_repository_and_missing_samples_are_named() {
        let m = song();
        let r = refs(&m);
        let bundled = write(&m, &r, &[], &About::default(), true);
        let back = read(&bundled, &mut |_| None).unwrap();
        assert!(back.about.bundled && back.missing.is_empty());
        assert_eq!(back.module.samples[0].wave.frames(), 1000);
        let bare = write(&m, &r, &[], &About::default(), false);
        let back = read(&bare, &mut |_| None).unwrap();
        assert_eq!(back.missing, vec![(1, r[0].unwrap().wave)]);
        assert!(bare.len() < bundled.len());
    }

    #[test]
    fn fm_instruments_and_synth_sounds_round_trip() {
        let mut m = song();
        let patch = FmPatch::from_s3m(&[0x21, 0x31, 0x4F, 0x00, 0xF2, 0xF2, 0x60, 0x72, 1, 2, 0x0B]);
        m.samples[1].fm = Some(patch);
        let sound = SynthSound { waveforms: vec![vec![], vec![1, -1, 100, -100]], volumes: vec![64, 0xF2, 2], waves: vec![0, 1, 0xFF], vol_speed: 2, wave_speed: 3, hybrid: true };
        m.samples[0].program = Some(Arc::new(Program::new(sound.clone(), &m.samples[0].wave)));
        let r = refs(&m);
        let bytes = write(&m, &r, &[], &About::default(), true);
        let back = read(&bytes, &mut |_| None).unwrap().module;
        assert_eq!(back.samples[1].fm, Some(patch));
        let p = back.samples[0].program.as_ref().unwrap();
        assert_eq!(p.sound, Sound::Med(sound));
        // The hybrid's waveform 0 is its sample, as read.
        assert!(Arc::ptr_eq(&p.waves[0], &back.samples[0].wave));
        assert_eq!(p.waves[1].pcm, vec![256, -256, 25600, -25600]);
    }

    #[test]
    fn pumatracker_instruments_round_trip_over_their_samples() {
        let mut m = song();
        let sound = PumaSound { volume: vec![[0xC0, 0, 0, 0], [0xA0, 64, 0, 8], [0xE0, 0, 0, 0]], pitch: vec![[0xD0, 24, 0, 1], [0xB0, 0, 0, 0]] };
        let looped = m.samples[1].looped;
        m.samples[0].program = Some(Arc::new(Program::puma(sound.clone(), vec![1], &m.samples)));
        let r = refs(&m);
        let bytes = write(&m, &r, &[], &About::default(), true);
        let back = read(&bytes, &mut |_| None).unwrap().module;
        let p = back.samples[0].program.as_ref().unwrap();
        assert_eq!(p.sound, Sound::Puma(sound));
        // Its first waveform is sample 2's, as read; the built-in cycles follow.
        assert!(Arc::ptr_eq(&p.waves[0], &back.samples[1].wave));
        assert_eq!(p.loops[0], looped);
        assert_eq!(p.waves.len(), 1 + 42);
        assert_eq!(p.waves[1].frames(), 32);
    }

    #[test]
    fn what_umf_added_round_trips_and_older_readers_pass_it_over() {
        let mut m = song();
        m.tempo_mode = TempoMode::Modern;
        m.time = TimeSig { rows_per_beat: 3, rows_per_measure: 12 };
        m.swing = 30;
        m.sequences = vec![Sequence { name: "A".into(), orders: vec![0], restart: 0 }, Sequence { name: "B".into(), orders: vec![0, 0], restart: 1 }];
        m.bookmarks = vec![Bookmark { name: "chorus".into(), order: None, pattern: Some(0), row: 16 }];
        m.slot_mutes = vec![(0, 2)];
        let mut fx = EffectSlot::new(EffectKind::Delay);
        fx.output = Some(1);
        m.effects = vec![fx, EffectSlot::new(EffectKind::Reverb)];
        m.channels[1].effect = Some(0);
        m.macros = Some(MidiMacros::default());
        m.samples[0].credit = "me".into();
        m.samples[0].slices = vec![0, 500];
        m.samples[1].synth = Some(Synth { wave: 5, ratio: 3, depth: 20, ..Default::default() });
        let i = &mut m.instruments[0];
        i.midi = Some(MidiOut { channel: 9, program: Some(3), bank: None });
        i.effect = Some(1);
        i.phrase = Some(Phrase { cells: vec![Cell { note: 61, ..Default::default() }], looped: true, base: 61 });
        i.volume_env.release = Some(1);
        let p = Arc::make_mut(&mut m.patterns[0]);
        p.time = Some(TimeSig { rows_per_beat: 6, rows_per_measure: 24 });
        p.swing = Some(10);
        p.automation = vec![Lane { target: Some(Target::Effect { slot: 0, param: 2 }), points: vec![(0, 0.0), (63, 1.0)] }];
        let r = refs(&m);
        let canon = canonical_wave(&m.samples[0]);
        let bytes = write(&m, &r, &[], &About::default(), false);
        let back = read(&bytes, &mut |_| Some(canon.clone())).unwrap().module;
        assert_eq!((back.tempo_mode, back.time, back.swing), (m.tempo_mode, m.time, m.swing));
        assert_eq!(back.all_sequences(), m.all_sequences());
        assert_eq!((back.bookmarks.clone(), back.slot_mutes.clone()), (m.bookmarks.clone(), m.slot_mutes.clone()));
        assert_eq!(back.effects, m.effects);
        assert_eq!(back.channels[1].effect, Some(0));
        assert_eq!(back.macros, m.macros);
        assert_eq!((back.samples[0].credit.as_str(), back.samples[0].slices.clone(), back.samples[1].synth), ("me", vec![0, 500], m.samples[1].synth));
        assert_eq!(back.instruments, m.instruments);
        assert_eq!(back.patterns[0].time, m.patterns[0].time);
        assert_eq!(back.patterns[0].automation, m.patterns[0].automation);
        // A plain song writes none of them.
        let plain = write(&song(), &refs(&song()), &[None], &About::default(), false);
        assert!(!plain.windows(4).any(|w| w == b"SONG" || w == b"EFCT"));
    }
}
