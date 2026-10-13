//! **ProTrekkr's and NoiseTrekker's replayer**: sixteen tracks, each
//! with up to sixteen notes a row, its own filter (24 kinds), flanger,
//! distortion, compressor, three-band EQ, LFOs on filter, volume and
//! panning, sends to a chorus and a reverb; instruments that play a
//! sample (split across the keyboard, looped) and a two-oscillator synth
//! with a sub-oscillator, two envelopes and two LFOs modulating eight
//! targets, its own filter and distortion ([`voice`]); two TB-303s, each
//! a step sequencer and a resonant low-pass ([`tb303`]). Everything runs
//! at 44.1 kHz, a frame at a time, as the replayer does; a row lasts the
//! tempo's frames, its effects worked six (or seven) times over it.
//!
//! Written from ProTrekkr's replayer (Franck Charlet's, BSD-licensed,
//! after Juan Antonio Arguelles Rius's NoiseTrekker), read for what each
//! part does and the order it does it in, not copied; its stock cycles
//! and tables are made here from their formulas. Its slips are kept
//! where songs hear them (see `docs/synth-methods.md`).

mod dsp;
mod player;
mod tb303;
mod voice;

use std::sync::Arc;

pub use player::Machine;

/// 2^x as the replayer works it out: the exponent from the float's bits,
/// the fraction's curve a parabola (its "fast" pow, which every pitch,
/// filter curve and compressor in it goes through).
pub(crate) fn pow2(i: f32) -> f32 {
    let y = i - i.floor();
    let y = (y - y * y) * 0.339_71;
    let x = (i + 127.0 - y) * (1 << 23) as f32;
    f32::from_bits(x as i32 as u32)
}

/// log2(x) the same way: the float's bits read as a number.
pub(crate) fn log2(i: f32) -> f32 {
    let x = i.to_bits() as i32 as f32 * (1.0 / (1 << 23) as f32) - 127.0;
    let y = x - x.floor();
    x + (y - y * y) * 0.346_607
}

/// a^b, the replayer's way.
pub(crate) fn pow(a: f32, b: f32) -> f32 {
    pow2(b * log2(a))
}

/// Tracks, and notes (and sub-channels) a track can have.
pub const TRACKS: usize = 16;
pub const POLYPHONY: usize = 16;
/// The replayer's rate.
pub const RATE: u32 = 44_100;
pub const INSTRUMENTS: usize = 128;
pub const SPLITS: usize = 16;
/// A pattern's most rows, and the sequence's most positions.
pub const MAX_ROWS: usize = 128;
pub const POSITIONS: usize = 256;
/// A note off; no note; no instrument; no volume or panning.
pub const NOTE_OFF: u8 = 120;
pub const NO_NOTE: u8 = 121;
pub const NO_INSTR: u8 = 255;
pub const NO_VALUE: u8 = 255;
/// The note a synth plays at its C (and a sample at its own rate, by
/// default): ProTrekkr's C-4.
pub const BASE_NOTE: u8 = 48;

/// A track's row: its notes (with their instruments), volume, panning
/// and up to four effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub notes: [u8; POLYPHONY],
    pub instrs: [u8; POLYPHONY],
    /// 0 to 64; F0 to FF cut the note on that sixth of the row.
    pub volume: u8,
    /// 0 to 128.
    pub panning: u8,
    pub fx: [(u8, u8); 4],
}

impl Default for Cell {
    fn default() -> Cell {
        Cell { notes: [NO_NOTE; POLYPHONY], instrs: [NO_INSTR; POLYPHONY], volume: NO_VALUE, panning: NO_VALUE, fx: [(0, 0); 4] }
    }
}

/// A pattern: `rows` rows of every track's cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub rows: usize,
    pub cells: Vec<Cell>,
}

impl Pattern {
    pub fn new(rows: usize) -> Pattern {
        Pattern { rows, cells: vec![Cell::default(); rows * TRACKS] }
    }

    pub fn cell(&self, row: usize, track: usize) -> Option<&Cell> {
        self.cells.get(row * TRACKS + track)
    }
}

/// An ADSR as the file has it: times (in a 512th, scaled by the note's
/// cycle) and a sustain level (of 128).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Adsr {
    pub attack: i32,
    pub decay: i32,
    pub sustain: u8,
    pub release: i32,
}

/// What an LFO or envelope does to each of the synth's targets (64 is
/// nothing; under it, the other way).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mods {
    pub osc1_pw: u8,
    pub osc2_pw: u8,
    pub osc1_pitch: u8,
    pub osc2_pitch: u8,
    pub osc1_volume: u8,
    pub osc2_volume: u8,
    pub cutoff: u8,
    pub resonance: u8,
    pub disto: u8,
}

impl Default for Mods {
    fn default() -> Mods {
        Mods { osc1_pw: 64, osc2_pw: 64, osc1_pitch: 64, osc2_pitch: 64, osc1_volume: 64, osc2_volume: 64, cutoff: 64, resonance: 64, disto: 64 }
    }
}

/// An instrument's synth, as the file has it. Waveforms: 0 sine, 1 saw,
/// 2 pulse, 3 white noise, 4 none, 5 the instrument's sample, 6 triangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SynthParams {
    pub osc1_wave: u8,
    pub osc2_wave: u8,
    /// How the oscillators combine: 0 add, 1 subtract, 2 multiply, 3 divide.
    pub combine: u8,
    /// Pulse widths, 0 to 512 (256 none).
    pub osc1_pw: i32,
    pub osc2_pw: i32,
    pub osc2_detune: u8,
    pub osc2_finetune: u8,
    pub vcf_cutoff: u8,
    pub vcf_resonance: u8,
    /// 0 low-pass, 1 high-pass, 2 none, 3 four-pole low-pass, 4 four-pole band-pass.
    pub vcf_type: u8,
    pub env1: Adsr,
    pub env2: Adsr,
    pub lfo1_period: i32,
    pub lfo2_period: i32,
    pub lfo1: Mods,
    pub lfo2: Mods,
    pub env1_mods: Mods,
    pub env2_mods: Mods,
    pub osc3_volume: u8,
    pub osc3_switch: u8,
    pub glide: u8,
    pub volume: u8,
    pub disto: u8,
    pub lfo1_adsr: Adsr,
    pub lfo2_adsr: Adsr,
    /// Oscillator 2 restarts 1 (and the sub) as it comes round.
    pub sync: u8,
    /// The sub-oscillator's interval under oscillator 1, 0 to 12 semitones (12 an octave).
    pub osc3_interval: u8,
}

impl Default for SynthParams {
    /// A new instrument's synth (ProTrekkr's defaults).
    fn default() -> SynthParams {
        let full = Mods { osc1_volume: 127, osc2_volume: 127, ..Mods::default() };
        SynthParams {
            osc1_wave: 1,
            osc2_wave: 2,
            combine: 0,
            osc1_pw: 256,
            osc2_pw: 256,
            osc2_detune: 65,
            osc2_finetune: 0,
            vcf_cutoff: 64,
            vcf_resonance: 64,
            vcf_type: 0,
            env1: Adsr { attack: 0, decay: 2560, sustain: 20, release: 16384 },
            env2: Adsr { attack: 0, decay: 2560, sustain: 20, release: 16384 },
            lfo1_period: 16,
            lfo2_period: 16,
            lfo1: Mods::default(),
            lfo2: Mods::default(),
            env1_mods: full,
            env2_mods: full,
            osc3_volume: 128,
            osc3_switch: 0,
            glide: 0,
            volume: 128,
            disto: 64,
            lfo1_adsr: Adsr { attack: 2560, decay: 2560, sustain: 16, release: 16384 },
            lfo2_adsr: Adsr { attack: 2560, decay: 2560, sustain: 16, release: 16384 },
            sync: 0,
            osc3_interval: 12,
        }
    }
}

/// A sample in one of an instrument's splits: from its base note up to
/// the next split's.
#[derive(Clone, Debug, PartialEq)]
pub struct Split {
    pub name: String,
    pub base_note: u8,
    pub loop_start: u32,
    pub loop_end: u32,
    /// 0 none, 1 forward, 2 ping-pong.
    pub loop_type: u8,
    /// In 128ths of a semitone.
    pub finetune: i8,
    pub amplify: f32,
    /// How fast the track's filter closes while it plays.
    pub decay: f32,
    pub left: Arc<Vec<i16>>,
    pub right: Option<Arc<Vec<i16>>>,
}

impl Split {
    pub fn len(&self) -> usize {
        self.left.len()
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_empty()
    }
}

/// An instrument: its splits, and its synth (`synth_prg` 0 off, 1 on
/// its own sample, 2 and up on instrument `synth_prg - 2`'s).
#[derive(Clone, Debug, PartialEq)]
pub struct Instrument {
    pub name: String,
    pub synth_prg: u8,
    pub synth: SynthParams,
    pub volume: f32,
    /// The sample is stretched to last `beat_lines` rows.
    pub beat_sync: bool,
    pub beat_lines: u16,
    pub splits: Vec<Option<Split>>,
}

impl Default for Instrument {
    fn default() -> Instrument {
        Instrument { name: String::new(), synth_prg: 0, synth: SynthParams::default(), volume: 1.0, beat_sync: false, beat_lines: 16, splits: vec![None; SPLITS] }
    }
}

/// A track's LFO, on its filter's cutoff, its volume and its panning.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lfo {
    pub on: bool,
    pub rate: f32,
    pub filter: f32,
    pub volume: f32,
    pub panning: f32,
    pub scale: f32,
}

/// A track's flanger.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flanger {
    pub on: bool,
    pub amount: f32,
    pub dephase: f32,
    pub rate: f32,
    pub ampl: f32,
    pub feedback: f32,
    pub delay: i32,
}

/// A track's settings as the song starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Track {
    /// The filter's cutoff (0 to 127) and how fast it gets there.
    pub cutoff: f32,
    pub inertia: f32,
    /// 0 left to 1 right.
    pub pan: f32,
    /// The filter's kind (4 none) and resonance.
    pub filter: i32,
    pub resonance: i32,
    /// The distortion's threshold and clamp, and whether it's on.
    pub dist_threshold: f32,
    pub dist_clamp: f32,
    pub distortion: bool,
    pub reverb_send: f32,
    /// Of 127.
    pub chorus_send: i32,
    pub polyphony: u8,
    pub notes: u8,
    pub effects: u8,
    pub volume: f32,
    pub surround: bool,
    /// Low, mid and high gains.
    pub eq: [f32; 3],
    /// 0 to 4: an average over 0, 4, 8, 16 or 32 frames.
    pub denoise: u8,
    pub lfo: Lfo,
    pub flanger: Flanger,
    pub muted: bool,
    pub compress: bool,
    pub comp_threshold: f32,
    pub comp_ratio: f32,
}

impl Default for Track {
    fn default() -> Track {
        Track {
            cutoff: 126.0,
            inertia: 0.003_906_2,
            pan: 0.5,
            filter: 4,
            resonance: 64,
            dist_threshold: 32767.0,
            dist_clamp: 32767.0,
            distortion: false,
            reverb_send: 0.0,
            chorus_send: 0,
            polyphony: 1,
            notes: 1,
            effects: 1,
            volume: 1.0,
            surround: false,
            eq: [1.0; 3],
            denoise: 0,
            lfo: Lfo { on: false, rate: 0.0001, filter: 0.0, volume: 0.0, panning: 0.0, scale: 1.0 },
            flanger: Flanger { on: false, amount: -0.8, dephase: 0.017_453_2, rate: 0.006_812_5 / 57.295_78, ampl: 0.001, feedback: -0.51, delay: 176 },
            muted: false,
            compress: false,
            comp_threshold: 100.0,
            comp_ratio: 0.0,
        }
    }
}

/// A TB-303's settings and its 32 patterns of 16 steps.
#[derive(Clone, Debug, PartialEq)]
pub struct Tb303 {
    pub selected: u8,
    pub tune: u8,
    pub cutoff: u8,
    pub resonance: u8,
    pub envmod: u8,
    pub decay: u8,
    pub accent: u8,
    /// 0 sawtooth, 1 square.
    pub waveform: u8,
    /// Rows a step lasts.
    pub scale: u8,
    pub lengths: [u8; 32],
    pub tones: [[u8; 16]; 32],
    /// A step's flags: 1 slide, 2 accent, 4 an octave up, 8 down, 16 a note (a rest without).
    pub flags: [[u8; 16]; 32],
    pub volume: f32,
}

impl Default for Tb303 {
    fn default() -> Tb303 {
        Tb303 { selected: 0, tune: 64, cutoff: 64, resonance: 64, envmod: 64, decay: 64, accent: 64, waveform: 0, scale: 1, lengths: [16; 32], tones: [[0; 16]; 32], flags: [[16; 16]; 32], volume: 0.5 }
    }
}

/// A song as the replayer plays it.
#[derive(Clone, Debug, PartialEq)]
pub struct Song {
    /// The file's version letter.
    pub version: u8,
    pub sequence: Vec<u8>,
    pub length: usize,
    pub patterns: Vec<Pattern>,
    /// Rows of each of the 128 patterns (those past `patterns` are empty).
    pub pattern_rows: Vec<u16>,
    pub instruments: Vec<Instrument>,
    pub tracks: Vec<Track>,
    /// Tracks shown and played.
    pub song_tracks: usize,
    /// Each position's tracks that play.
    pub active: Vec<[bool; TRACKS]>,
    pub bpm: i32,
    /// Rows a beat.
    pub tpb: i32,
    pub volume: f32,
    pub shuffle: i32,
    /// The reverb's on, its allpasses' spacing, its combs' decays, its
    /// allpasses' feedback, its filter, its damping and stereo spread.
    pub reverb: bool,
    pub reverb_threshold: i32,
    pub reverb_decays: Vec<f32>,
    pub reverb_feedback: f32,
    pub reverb_cutoff: f32,
    pub reverb_resonance: f32,
    pub reverb_damp: f32,
    pub reverb_stereo: u8,
    /// The chorus: each side's delay and feedback, and its filter (kind
    /// 4 none, cutoff, resonance).
    pub chorus_delay: [i32; 2],
    pub chorus_feedback: [f32; 2],
    pub chorus_filter: [i32; 3],
    /// The mix's compressor: threshold and ratio (0 to 100).
    pub comp: [f32; 2],
    pub tb303: [Tb303; 2],
}

impl Default for Song {
    fn default() -> Song {
        Song {
            version: b'X',
            sequence: vec![0; POSITIONS],
            length: 1,
            patterns: Vec::new(),
            pattern_rows: vec![64; 128],
            instruments: vec![Instrument::default(); INSTRUMENTS],
            tracks: vec![Track::default(); TRACKS],
            song_tracks: TRACKS,
            active: vec![[true; TRACKS]; POSITIONS],
            bpm: 125,
            tpb: 4,
            volume: 1.0,
            shuffle: 0,
            reverb: false,
            reverb_threshold: 32,
            reverb_decays: Vec::new(),
            reverb_feedback: 0.5,
            reverb_cutoff: 0.08,
            reverb_resonance: 0.2,
            reverb_damp: 1.0,
            reverb_stereo: 63,
            chorus_delay: [10584, 15876],
            chorus_feedback: [0.6, 0.5],
            chorus_filter: [4, 0, 64],
            comp: [100.0, 0.0],
            tb303: [Tb303::default(), Tb303::default()],
        }
    }
}

impl Song {
    /// The pattern at a position, and its rows.
    pub fn pattern_at(&self, position: usize) -> (Option<&Pattern>, usize) {
        let p = usize::from(self.sequence.get(position).copied().unwrap_or(0));
        (self.patterns.get(p), usize::from(self.pattern_rows.get(p).copied().unwrap_or(64)).clamp(1, MAX_ROWS))
    }

    /// The splits' samples in order (instrument by instrument), each with
    /// where it is: what a host keeps as its samples.
    pub fn splits(&self) -> Vec<(usize, usize, &Split)> {
        let mut out = Vec::new();
        for (i, ins) in self.instruments.iter().enumerate() {
            for (s, sp) in ins.splits.iter().enumerate() {
                if let Some(sp) = sp {
                    out.push((i, s, sp));
                }
            }
        }
        out
    }

    /// Everything but the samples' points, in a few bytes (`from_bytes`
    /// reads it back with the points handed in, in `splits()` order).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer(Vec::new());
        w.u8(1);
        w.u8(self.version);
        w.bytes(&self.sequence);
        w.u16(self.length as u16);
        w.u16(self.patterns.len() as u16);
        for p in &self.patterns {
            w.u8(p.rows as u8);
            // Only cells that aren't empty, by place.
            let empty = Cell::default();
            let cells: Vec<(usize, &Cell)> = p.cells.iter().enumerate().filter(|(_, c)| **c != empty).collect();
            w.u32(cells.len() as u32);
            for (i, c) in cells {
                w.u16(i as u16);
                w.bytes(&c.notes);
                w.bytes(&c.instrs);
                w.u8(c.volume);
                w.u8(c.panning);
                for (e, d) in c.fx {
                    w.u8(e);
                    w.u8(d);
                }
            }
        }
        for &r in &self.pattern_rows {
            w.u16(r);
        }
        for ins in &self.instruments {
            w.s(&ins.name);
            w.u8(ins.synth_prg);
            write_synth(&mut w, &ins.synth);
            w.f32(ins.volume);
            w.u8(u8::from(ins.beat_sync));
            w.u16(ins.beat_lines);
            for sp in &ins.splits {
                match sp {
                    None => w.u8(0),
                    Some(s) => {
                        w.u8(1 + u8::from(s.right.is_some()));
                        w.s(&s.name);
                        w.u8(s.base_note);
                        w.u32(s.loop_start);
                        w.u32(s.loop_end);
                        w.u8(s.loop_type);
                        w.u8(s.finetune as u8);
                        w.f32(s.amplify);
                        w.f32(s.decay);
                        w.u32(s.len() as u32);
                    }
                }
            }
        }
        for t in &self.tracks {
            for v in [t.cutoff, t.inertia, t.pan, t.dist_threshold, t.dist_clamp, t.reverb_send, t.volume, t.eq[0], t.eq[1], t.eq[2], t.comp_threshold, t.comp_ratio] {
                w.f32(v);
            }
            w.u32(t.filter as u32);
            w.u32(t.resonance as u32);
            w.u32(t.chorus_send as u32);
            for b in [t.distortion, t.surround, t.muted, t.compress] {
                w.u8(u8::from(b));
            }
            w.bytes(&[t.polyphony, t.notes, t.effects, t.denoise]);
            let l = t.lfo;
            w.u8(u8::from(l.on));
            for v in [l.rate, l.filter, l.volume, l.panning, l.scale] {
                w.f32(v);
            }
            let f = t.flanger;
            w.u8(u8::from(f.on));
            for v in [f.amount, f.dephase, f.rate, f.ampl, f.feedback] {
                w.f32(v);
            }
            w.u32(f.delay as u32);
        }
        w.u8(self.song_tracks as u8);
        for a in &self.active {
            w.u16(a.iter().enumerate().fold(0u16, |m, (i, &on)| m | (u16::from(on) << i)));
        }
        w.u32(self.bpm as u32);
        w.u32(self.tpb as u32);
        w.f32(self.volume);
        w.u32(self.shuffle as u32);
        w.u8(u8::from(self.reverb));
        w.u32(self.reverb_threshold as u32);
        w.u8(self.reverb_decays.len() as u8);
        for &d in &self.reverb_decays {
            w.f32(d);
        }
        for v in [self.reverb_feedback, self.reverb_cutoff, self.reverb_resonance, self.reverb_damp] {
            w.f32(v);
        }
        w.u8(self.reverb_stereo);
        for v in self.chorus_delay.into_iter().chain(self.chorus_filter) {
            w.u32(v as u32);
        }
        for v in self.chorus_feedback.into_iter().chain(self.comp) {
            w.f32(v);
        }
        for t in &self.tb303 {
            w.bytes(&[t.selected, t.tune, t.cutoff, t.resonance, t.envmod, t.decay, t.accent, t.waveform, t.scale]);
            w.bytes(&t.lengths);
            for p in 0..32 {
                w.bytes(&t.tones[p]);
                w.bytes(&t.flags[p]);
            }
            w.f32(t.volume);
        }
        w.0
    }

    /// A song from `to_bytes`, its samples' points from `points` (each
    /// split's left and right, in `splits()` order), or None when it's
    /// damaged.
    pub fn from_bytes(b: &[u8], points: &mut dyn FnMut(usize) -> (Arc<Vec<i16>>, Option<Arc<Vec<i16>>>)) -> Option<Song> {
        let mut r = ByteReader { b, at: 0 };
        if r.u8()? != 1 {
            return None;
        }
        let mut s = Song { version: r.u8()?, sequence: r.bytes(POSITIONS)?.to_vec(), length: usize::from(r.u16()?), ..Song::default() };
        let np = usize::from(r.u16()?);
        for _ in 0..np {
            let rows = usize::from(r.u8()?).clamp(1, MAX_ROWS);
            let mut p = Pattern::new(rows);
            for _ in 0..r.u32()? {
                let i = usize::from(r.u16()?);
                let mut c = Cell::default();
                c.notes.copy_from_slice(r.bytes(POLYPHONY)?);
                c.instrs.copy_from_slice(r.bytes(POLYPHONY)?);
                c.volume = r.u8()?;
                c.panning = r.u8()?;
                for f in &mut c.fx {
                    *f = (r.u8()?, r.u8()?);
                }
                *p.cells.get_mut(i)? = c;
            }
            s.patterns.push(p);
        }
        for p in s.pattern_rows.iter_mut() {
            *p = r.u16()?;
        }
        let mut n = 0;
        for ins in s.instruments.iter_mut() {
            ins.name = r.s()?;
            ins.synth_prg = r.u8()?;
            ins.synth = read_synth(&mut r)?;
            ins.volume = r.f32()?;
            ins.beat_sync = r.u8()? != 0;
            ins.beat_lines = r.u16()?;
            for sp in ins.splits.iter_mut() {
                let kind = r.u8()?;
                if kind == 0 {
                    continue;
                }
                let name = r.s()?;
                let (base_note, loop_start, loop_end, loop_type, finetune) = (r.u8()?, r.u32()?, r.u32()?, r.u8()?, r.u8()? as i8);
                let (amplify, decay) = (r.f32()?, r.f32()?);
                let len = r.u32()? as usize;
                let (mut left, mut right) = points(n);
                n += 1;
                if left.len() != len {
                    let mut l = left.as_ref().clone();
                    l.resize(len, 0);
                    left = Arc::new(l);
                }
                if kind == 1 {
                    right = None;
                } else if right.as_ref().is_none_or(|x| x.len() != len) {
                    let mut x = right.as_deref().cloned().unwrap_or_default();
                    x.resize(len, 0);
                    right = Some(Arc::new(x));
                }
                *sp = Some(Split { name, base_note, loop_start, loop_end, loop_type, finetune, amplify, decay, left, right });
            }
        }
        for t in s.tracks.iter_mut() {
            let mut f = [0f32; 12];
            for v in &mut f {
                *v = r.f32()?;
            }
            [t.cutoff, t.inertia, t.pan, t.dist_threshold, t.dist_clamp, t.reverb_send, t.volume, t.eq[0], t.eq[1], t.eq[2], t.comp_threshold, t.comp_ratio] = f;
            t.filter = r.u32()? as i32;
            t.resonance = r.u32()? as i32;
            t.chorus_send = r.u32()? as i32;
            t.distortion = r.u8()? != 0;
            t.surround = r.u8()? != 0;
            t.muted = r.u8()? != 0;
            t.compress = r.u8()? != 0;
            let b = r.bytes(4)?;
            (t.polyphony, t.notes, t.effects, t.denoise) = (b[0], b[1], b[2], b[3]);
            t.lfo.on = r.u8()? != 0;
            (t.lfo.rate, t.lfo.filter, t.lfo.volume, t.lfo.panning, t.lfo.scale) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?);
            t.flanger.on = r.u8()? != 0;
            (t.flanger.amount, t.flanger.dephase, t.flanger.rate, t.flanger.ampl, t.flanger.feedback) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?);
            t.flanger.delay = r.u32()? as i32;
        }
        s.song_tracks = usize::from(r.u8()?).min(TRACKS);
        for a in s.active.iter_mut() {
            let m = r.u16()?;
            for (i, on) in a.iter_mut().enumerate() {
                *on = m >> i & 1 != 0;
            }
        }
        s.bpm = r.u32()? as i32;
        s.tpb = r.u32()? as i32;
        s.volume = r.f32()?;
        s.shuffle = r.u32()? as i32;
        s.reverb = r.u8()? != 0;
        s.reverb_threshold = r.u32()? as i32;
        let nd = usize::from(r.u8()?);
        s.reverb_decays = (0..nd).map(|_| r.f32()).collect::<Option<_>>()?;
        (s.reverb_feedback, s.reverb_cutoff, s.reverb_resonance, s.reverb_damp) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?);
        s.reverb_stereo = r.u8()?;
        s.chorus_delay = [r.u32()? as i32, r.u32()? as i32];
        s.chorus_filter = [r.u32()? as i32, r.u32()? as i32, r.u32()? as i32];
        s.chorus_feedback = [r.f32()?, r.f32()?];
        s.comp = [r.f32()?, r.f32()?];
        for t in s.tb303.iter_mut() {
            let b = r.bytes(9)?;
            (t.selected, t.tune, t.cutoff, t.resonance, t.envmod, t.decay, t.accent, t.waveform, t.scale) = (b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8]);
            t.lengths.copy_from_slice(r.bytes(32)?);
            for p in 0..32 {
                t.tones[p].copy_from_slice(r.bytes(16)?);
                t.flags[p].copy_from_slice(r.bytes(16)?);
            }
            t.volume = r.f32()?;
        }
        Some(s)
    }
}

fn write_adsr(w: &mut Writer, a: &Adsr) {
    w.u32(a.attack as u32);
    w.u32(a.decay as u32);
    w.u8(a.sustain);
    w.u32(a.release as u32);
}

fn read_adsr(r: &mut ByteReader) -> Option<Adsr> {
    Some(Adsr { attack: r.u32()? as i32, decay: r.u32()? as i32, sustain: r.u8()?, release: r.u32()? as i32 })
}

fn write_mods(w: &mut Writer, m: &Mods) {
    w.bytes(&[m.osc1_pw, m.osc2_pw, m.osc1_pitch, m.osc2_pitch, m.osc1_volume, m.osc2_volume, m.cutoff, m.resonance, m.disto]);
}

fn read_mods(r: &mut ByteReader) -> Option<Mods> {
    let b = r.bytes(9)?;
    Some(Mods { osc1_pw: b[0], osc2_pw: b[1], osc1_pitch: b[2], osc2_pitch: b[3], osc1_volume: b[4], osc2_volume: b[5], cutoff: b[6], resonance: b[7], disto: b[8] })
}

fn write_synth(w: &mut Writer, p: &SynthParams) {
    w.bytes(&[p.osc1_wave, p.osc2_wave, p.combine]);
    w.u32(p.osc1_pw as u32);
    w.u32(p.osc2_pw as u32);
    w.bytes(&[p.osc2_detune, p.osc2_finetune, p.vcf_cutoff, p.vcf_resonance, p.vcf_type]);
    write_adsr(w, &p.env1);
    write_adsr(w, &p.env2);
    w.u32(p.lfo1_period as u32);
    w.u32(p.lfo2_period as u32);
    for m in [&p.lfo1, &p.lfo2, &p.env1_mods, &p.env2_mods] {
        write_mods(w, m);
    }
    w.bytes(&[p.osc3_volume, p.osc3_switch, p.glide, p.volume, p.disto]);
    write_adsr(w, &p.lfo1_adsr);
    write_adsr(w, &p.lfo2_adsr);
    w.bytes(&[p.sync, p.osc3_interval]);
}

fn read_synth(r: &mut ByteReader) -> Option<SynthParams> {
    let b = r.bytes(3)?;
    let (osc1_wave, osc2_wave, combine) = (b[0], b[1], b[2]);
    let (osc1_pw, osc2_pw) = (r.u32()? as i32, r.u32()? as i32);
    let b = r.bytes(5)?;
    let (osc2_detune, osc2_finetune, vcf_cutoff, vcf_resonance, vcf_type) = (b[0], b[1], b[2], b[3], b[4]);
    let (env1, env2) = (read_adsr(r)?, read_adsr(r)?);
    let (lfo1_period, lfo2_period) = (r.u32()? as i32, r.u32()? as i32);
    let (lfo1, lfo2, env1_mods, env2_mods) = (read_mods(r)?, read_mods(r)?, read_mods(r)?, read_mods(r)?);
    let b = r.bytes(5)?;
    let (osc3_volume, osc3_switch, glide, volume, disto) = (b[0], b[1], b[2], b[3], b[4]);
    let (lfo1_adsr, lfo2_adsr) = (read_adsr(r)?, read_adsr(r)?);
    let b = r.bytes(2)?;
    Some(SynthParams {
        osc1_wave,
        osc2_wave,
        combine,
        osc1_pw,
        osc2_pw,
        osc2_detune,
        osc2_finetune,
        vcf_cutoff,
        vcf_resonance,
        vcf_type,
        env1,
        env2,
        lfo1_period,
        lfo2_period,
        lfo1,
        lfo2,
        env1_mods,
        env2_mods,
        osc3_volume,
        osc3_switch,
        glide,
        volume,
        disto,
        lfo1_adsr,
        lfo2_adsr,
        sync: b[0],
        osc3_interval: b[1],
    })
}

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
    fn s(&mut self, s: &str) {
        let b = s.as_bytes();
        let n = b.len().min(255);
        self.u8(n as u8);
        self.0.extend_from_slice(&b[..n]);
    }
}

struct ByteReader<'a> {
    b: &'a [u8],
    at: usize,
}

impl ByteReader<'_> {
    fn bytes(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.b.get(self.at..self.at + n)?;
        self.at += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.bytes(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.bytes(4)?.try_into().ok()?))
    }
    fn s(&mut self) -> Option<String> {
        let n = usize::from(self.u8()?);
        Some(String::from_utf8_lossy(self.bytes(n)?).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fast_pow_is_exact_on_whole_powers_and_close_between() {
        assert_eq!((pow2(0.0), pow2(1.0), pow2(-2.0)), (1.0, 2.0, 0.25));
        assert!((pow2(0.5) - std::f32::consts::SQRT_2).abs() < 0.003);
        assert!((log2(8.0) - 3.0).abs() < 1e-6);
        assert!((pow(9.0, 0.5) - 3.0).abs() < 0.03);
    }

    /// One saw note on track 0 at ProTrekkr's C-4 (+12: C-5).
    fn song() -> Song {
        let mut s = Song::default();
        s.song_tracks = 1;
        let mut p = Pattern::new(16);
        p.cells[0].notes[0] = BASE_NOTE + 12;
        p.cells[0].instrs[0] = 0;
        s.patterns.push(p);
        s.pattern_rows[0] = 16;
        let ins = &mut s.instruments[0];
        ins.synth_prg = 1;
        ins.synth.osc2_wave = 4;
        ins.synth.vcf_type = 2;
        ins.synth.env1.sustain = 128;
        s
    }

    #[test]
    fn a_synth_note_plays_its_pitch_and_comes_back_from_its_bytes() {
        let s = Arc::new(song());
        let mut m = Machine::new(s.clone());
        let n = 44_100;
        let (mut l, mut r) = (vec![0f32; n], vec![0f32; n]);
        m.render_native(&mut l, &mut r);
        // A rising saw falls once a cycle: about 130.5 times a second here.
        let falls = l[4410..].windows(2).filter(|w| w[0] > 0.0 && w[1] < 0.0 && w[0] - w[1] > 0.01).count() as f64 / 0.9;
        assert!((falls - 130.5).abs() < 3.0, "{falls}");
        let back = Song::from_bytes(&s.to_bytes(), &mut |_| (Arc::new(Vec::new()), None)).expect("reads back");
        assert_eq!(&back, s.as_ref());
    }

    #[test]
    fn a_song_is_as_long_as_its_rows() {
        let s = Arc::new(song());
        assert!((Machine::length(&s, 60.0) - 16.0 * 5293.0 / 44_100.0).abs() < 0.001);
    }
}
