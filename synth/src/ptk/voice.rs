//! ProTrekkr's synth: two oscillators and a sub-oscillator (a stock
//! cycle or the instrument's sample), each under an ADSR envelope, two
//! LFOs (a sine under its own ADSR), and the eight targets each of the
//! four modulates (pulse width, pitch and volume of either oscillator,
//! the filter's cutoff and resonance, the distortion); a soft clip; a
//! two-pole filter (low or high) or a four-pole one (low or band).
//!
//! Its envelopes and LFOs count in output frames at 44.1 kHz, their
//! lengths scaled by the note's pitch (a longer cycle, a longer stage),
//! as the replayer has them.

use super::{pow2, Adsr, Mods, SynthParams};

/// The stock cycles' length, and white noise's.
pub(crate) const CYCLE: usize = 360;
pub(crate) const NOISE: usize = 88_273;

const SIN: u8 = 0;
const SAW: u8 = 1;
const PULSE: u8 = 2;
const WHITE: u8 = 3;
const NONE: u8 = 4;
pub(crate) const WAV: u8 = 5;
const TRI: u8 = 6;

/// A stock cycle's step for a frame at its note's first semitone (C).
const ADJUST: f32 = 0.5325;

pub(crate) const ATTACK: u8 = 1;
pub(crate) const DECAY: u8 = 2;
pub(crate) const SUSTAIN: u8 = 3;
pub(crate) const RELEASE: u8 = 4;

pub(crate) const LOOP_NONE: u8 = 0;
pub(crate) const LOOP_FORWARD: u8 = 1;
pub(crate) const LOOP_PINGPONG: u8 = 2;

/// The stock cycles (360 points each), white noise (88,273 points), and
/// the sine the LFOs read in whole degrees.
pub(crate) struct Tables {
    pub sin: Vec<i16>,
    pub saw: Vec<i16>,
    pub pulse: Vec<i16>,
    pub tri: Vec<i16>,
    pub white: Vec<i16>,
    pub degrees: [f32; 360],
    /// Panning's square roots, of 1024ths.
    pub sqrt: [f32; 1025],
}

impl Tables {
    pub fn new() -> Tables {
        let mut t = Tables { sin: Vec::with_capacity(CYCLE), saw: Vec::with_capacity(CYCLE), pulse: Vec::with_capacity(CYCLE), tri: Vec::with_capacity(CYCLE), white: vec![0; NOISE], degrees: [0.0; 360], sqrt: [0.0; 1025] };
        let (mut tri, mut tri_step) = (0i32, 1i32);
        for i in 0..CYCLE {
            let x = i as f32;
            let v = x * 0.017_453_2;
            t.pulse.push(if v.sin() < 0.0 { 32767 } else { -32767 });
            // The ramp's points are an unsigned word read as a signed one:
            // up from 0 to the top, then from the bottom back to 0.
            t.saw.push((((x / 360.0) % 64.0) * 32767.0 * 2.0) as u16 as i16);
            if i % 180 == 0 {
                tri_step = -tri_step;
            }
            t.tri.push((((tri as f32 / 180.0) + 0.5) * 32767.0 * 2.0) as i32 as i16);
            tri += tri_step;
            t.sin.push((v.sin() * 32767.0) as i16);
        }
        // Two seconds of it as the replayer counts them (a float stepped
        // by a 44,100th to 2), which fills the whole table.
        let mut seed = 0x2545_F491u32;
        for p in t.white.iter_mut() {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            *p = (((seed >> 16) & 0x7FFF) as i32 - 16_384).wrapping_mul(2) as i16;
        }
        for (i, d) in t.degrees.iter_mut().enumerate() {
            *d = (i as f32 * 0.017_453_2).sin();
        }
        for (i, s) in t.sqrt.iter_mut().enumerate() {
            *s = (i as f32 / 1024.0).sqrt();
        }
        t
    }

    fn cycle(&self, wave: u8) -> Option<&[i16]> {
        match wave {
            SIN => Some(&self.sin),
            SAW => Some(&self.saw),
            PULSE => Some(&self.pulse),
            WHITE => Some(&self.white),
            TRI => Some(&self.tri),
            _ => None,
        }
    }
}

/// A place in a waveform: its whole frame and where it really is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Pos {
    pub int: i32,
    pub flt: f32,
}

impl Pos {
    pub fn at(x: i32) -> Pos {
        Pos { int: x, flt: x as f32 }
    }
}

/// Where in a waveform a place reads, held to the loop's ends or the
/// waveform's (the replayer's `Set_Carrier_Boundaries`).
pub(crate) fn bounded(pos: i32, kind: u8, backwards: bool, length: u32, loop_end: u32, loop_start: u32) -> usize {
    let mut at = pos as u32;
    let length = length.wrapping_sub(1);
    let loop_end = if loop_end >= length { length } else { loop_end };
    match kind {
        LOOP_FORWARD => {
            if !backwards {
                if at >= loop_end {
                    at = loop_start;
                }
            } else if at as i32 <= loop_start as i32 {
                at = loop_end;
            }
        }
        LOOP_PINGPONG => {
            if at >= loop_end {
                at = loop_end;
            }
            if backwards && at as i32 <= loop_start as i32 {
                at = loop_start;
            }
        }
        _ => {
            if !backwards {
                if at >= length {
                    at = length;
                }
            } else if at as i32 <= 0 {
                at = 0;
            }
        }
    }
    at as usize
}

/// The parameters as the synth works with them.
#[derive(Clone, Copy, Debug, Default)]
struct Data {
    osc1_wave: u8,
    osc2_wave: u8,
    combine: u8,
    sync: bool,
    osc3_interval: u8,
    osc1_pw: f32,
    osc2_pw: f32,
    osc2_detune: f32,
    osc2_finetune: f32,
    cutoff: f32,
    resonance: f32,
    vcf_type: u8,
    env: [Stage; 2],
    lfo_env: [Stage; 2],
    lfo_subgr_max: [i32; 2],
    lfo: [Targets; 2],
    envm: [Targets; 2],
    osc3_volume: f32,
    osc3_on: bool,
    glide: f32,
    disto: f32,
}

/// An ADSR's times (of a 512th of a cycle's length... as the replayer
/// scales them) and its sustain level.
#[derive(Clone, Copy, Debug, Default)]
struct Stage {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

impl Stage {
    fn of(a: &Adsr) -> Stage {
        Stage {
            attack: ((a.attack + 1) as f32 / 512.0).max(0.1),
            decay: (a.decay + 1) as f32 / 512.0,
            sustain: f32::from(a.sustain) * 0.007_812_5,
            release: ((a.release + 1) as f32 / 512.0).max(0.15),
        }
    }
}

/// What an LFO or an envelope does to each target, -1 to 1.
#[derive(Clone, Copy, Debug, Default)]
struct Targets {
    osc1_pw: f32,
    osc2_pw: f32,
    osc1_pitch: f32,
    osc2_pitch: f32,
    osc1_volume: f32,
    osc2_volume: f32,
    cutoff: f32,
    resonance: f32,
    disto: f32,
}

impl Targets {
    fn of(m: &Mods) -> Targets {
        let f = |v: u8| (f32::from(v) - 64.0) * 0.015_625;
        Targets {
            osc1_pw: f(m.osc1_pw),
            osc2_pw: f(m.osc2_pw),
            osc1_pitch: f(m.osc1_pitch),
            osc2_pitch: f(m.osc2_pitch),
            osc1_volume: f(m.osc1_volume),
            osc2_volume: f(m.osc2_volume),
            cutoff: f(m.cutoff),
            resonance: f(m.resonance),
            disto: f(m.disto),
        }
    }
}

/// An envelope (or an LFO's ADSR) running.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Env {
    pub stage: u8,
    counter: f32,
    value: f32,
    /// Its stages' lengths in frames, and its steps.
    attack: f32,
    decay: f32,
    release: f32,
    a: f32,
    d: f32,
    r: f32,
}

impl Env {
    /// A frame on. True when its release has ended.
    fn run(&mut self, sustain: f32) -> bool {
        match self.stage {
            ATTACK => {
                self.value += self.a;
                self.counter += 1.0;
                if self.counter >= self.attack {
                    if self.value > 1.0 {
                        self.value = 1.0;
                    }
                    self.counter = 0.0;
                    self.stage = DECAY;
                }
            }
            DECAY => {
                self.value -= self.d;
                self.counter += 1.0;
                if self.counter >= self.decay {
                    self.value = sustain;
                    self.counter = 0.0;
                    self.stage = SUSTAIN;
                }
            }
            SUSTAIN => self.value = sustain,
            RELEASE => {
                self.value -= self.r;
                self.counter += 1.0;
                if self.counter > self.release {
                    self.value = 0.0;
                    self.counter = 0.0;
                    self.stage = 0;
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    fn note_off(&mut self) {
        if self.stage > 0 && self.stage < RELEASE {
            self.r = self.value / self.release;
            self.counter = 0.0;
            self.stage = RELEASE;
        }
    }
}

/// An LFO: a sine stepped a degree every so many frames, under an ADSR.
#[derive(Clone, Copy, Debug, Default)]
struct Lfo {
    env: Env,
    degree: usize,
    sub: i32,
    value: f32,
}

/// A synth voice (one of a track's sub-channels).
#[derive(Clone, Debug, Default)]
pub(crate) struct Voice {
    data: Data,
    pub env: [Env; 2],
    lfo: [Lfo; 2],
    /// Oscillators 1 and 3, and 2, playing backwards (a reversed sample).
    pub backwards: [bool; 3],
    speed: [f32; 3],
    glide: f32,
    env_volume: [f32; 2],
    env_min: [f32; 2],
    sbuf: [[f32; 2]; 2],
    moog: [[f32; 5]; 2],
    cut: f32,
    res: f32,
    filt_a: f32,
    filt_b: f32,
}

/// What a voice reads its waveforms from: the instrument's sample, its
/// loop and how it's laid out.
pub(crate) struct Source<'a> {
    pub left: Option<&'a [i16]>,
    pub right: Option<&'a [i16]>,
    pub stereo: bool,
    pub loop_type: u8,
    pub length: u32,
    pub loop_sub: u32,
}

/// Its stages: oscillator 1 (and 3) sounding, oscillator 2 sounding.
pub(crate) struct Playing<'a> {
    pub osc1: &'a mut u8,
    pub osc2: &'a mut u8,
}

impl Voice {
    pub fn reset(&mut self) {
        *self = Voice::default();
        self.data.osc1_wave = SAW;
        self.data.osc2_wave = PULSE;
    }

    pub fn osc1_wave(&self) -> u8 {
        self.data.osc1_wave
    }

    pub fn osc2_wave(&self) -> u8 {
        self.data.osc2_wave
    }

    pub fn uses_sample(&self) -> bool {
        self.data.osc1_wave == WAV || self.data.osc2_wave == WAV
    }

    /// Neither oscillator sounds (a note on that clears them first).
    pub fn silence_waves(&mut self) {
        self.data.osc1_wave = NONE;
        self.data.osc2_wave = NONE;
    }

    /// The instrument's parameters, as `ChangeParameters` takes them
    /// (the LFOs' steps from the tempo's frames a row).
    pub fn set_params(&mut self, p: &SynthParams, samples_per_tick: i32) {
        let d = &mut self.data;
        d.osc1_wave = p.osc1_wave;
        d.osc2_wave = p.osc2_wave;
        d.combine = p.combine;
        d.sync = p.sync != 0;
        d.osc3_interval = p.osc3_interval;
        d.osc1_pw = (p.osc1_pw - 256) as f32 / 256.0;
        d.osc2_pw = (p.osc2_pw - 256) as f32 / 256.0;
        d.osc2_detune = (f32::from(p.osc2_detune) - 64.0) * 0.0625;
        d.osc2_finetune = f32::from(p.osc2_finetune) * 0.007_812_5;
        d.cutoff = f32::from(p.vcf_cutoff) * 0.007_812_5;
        d.resonance = f32::from(p.vcf_resonance) * 0.007_812_5;
        d.vcf_type = p.vcf_type;
        d.env = [Stage::of(&p.env1), Stage::of(&p.env2)];
        d.lfo_env = [Stage::of(&p.lfo1_adsr), Stage::of(&p.lfo2_adsr)];
        for (i, period) in [p.lfo1_period, p.lfo2_period].into_iter().enumerate() {
            let period = (period * 2) as f32 + 1.0;
            d.lfo_subgr_max[i] = (samples_per_tick as f32 * 0.000_277 * period) as i32;
        }
        d.lfo = [Targets::of(&p.lfo1), Targets::of(&p.lfo2)];
        d.envm = [Targets::of(&p.env1_mods), Targets::of(&p.env2_mods)];
        d.osc3_volume = (f32::from(p.osc3_volume) - 64.0) * 0.015_625;
        d.osc3_on = p.osc3_switch != 0;
        d.glide = f32::from(p.glide) * f32::from(p.glide) * 0.000_001_562_5;
        self.glide = d.glide;
        d.disto = (f32::from(p.disto) - 64.0) * 0.015_625;
    }

    /// A note starts (or glides on, its envelopes going on as they were):
    /// `note` in semitones from the synth's C, `note_smp` the sample's.
    pub fn note_on(&mut self, note: i32, looping: bool, length: u32, loop_length: u32, note_smp: f32, glide: bool) {
        let d = self.data;
        let note1 = if d.osc1_wave == WAV { note_smp } else { note as f32 };
        let note2 = if d.osc2_wave == WAV { note_smp } else { note as f32 };
        let step1 = pow2(note1 / 12.0);
        let step2 = pow2((note2 + d.osc2_finetune + d.osc2_detune) / 12.0);
        if !glide {
            for e in &mut self.env {
                *e = Env { stage: ATTACK, ..Env::default() };
            }
            self.env_volume = [0.0; 2];
            self.env_min = [0.0; 2];
            self.sbuf = [[0.0; 2]; 2];
            self.moog = [[0.0; 5]; 2];
        }
        // Each envelope's (and LFO's) lengths, from its oscillator's
        // cycle: the sample's (its loop's, for the release) or a stock one.
        let mut lfo_times = [(0.0f32, 0.0f32, 0.0f32); 2];
        // Oscillator 1's stock cycle sets the lengths oscillator 2 sees too.
        let (mut len, mut loop_len) = (length, loop_length);
        for (k, step) in [step1, step2].into_iter().enumerate() {
            let wave = if k == 0 { d.osc1_wave } else { d.osc2_wave };
            if wave != WAV {
                len = CYCLE as u32;
                loop_len = CYCLE as u32;
            }
            let smp_freq = len as f32 / step;
            let ratio = smp_freq / 1024.0;
            let e = &mut self.env[k];
            e.attack = d.env[k].attack / ratio * smp_freq;
            e.decay = d.env[k].decay / ratio * smp_freq;
            let (la, ld) = (d.lfo_env[k].attack / ratio * smp_freq, d.lfo_env[k].decay / ratio * smp_freq);
            let (smp_freq, ratio) = if looping {
                let f = loop_len as f32 / step;
                (f, f / 1024.0)
            } else {
                (smp_freq, ratio)
            };
            e.release = d.env[k].release / ratio * smp_freq;
            lfo_times[k] = (la, ld, d.lfo_env[k].release / ratio * smp_freq);
        }
        for (k, e) in self.env.iter_mut().enumerate() {
            e.attack = e.attack.max(1.0);
            e.decay = e.decay.max(1.0);
            e.release = e.release.max(1.0);
            e.d = (1.0 - d.env[k].sustain) / e.decay;
            e.r = d.env[k].sustain / e.release;
            e.a = (1.0 - e.value) / e.attack;
        }
        if !glide {
            self.backwards = [false; 3];
            for l in &mut self.lfo {
                *l = Lfo { env: Env { stage: ATTACK, ..Env::default() }, ..Lfo::default() };
            }
        }
        for (k, l) in self.lfo.iter_mut().enumerate() {
            let (a, dd, r) = lfo_times[k];
            l.env.attack = a.max(1.0);
            l.env.decay = dd.max(1.0);
            l.env.release = r.max(1.0);
            l.env.d = (1.0 - d.lfo_env[k].sustain) / l.env.decay;
            l.env.r = d.lfo_env[k].sustain / l.env.release;
            l.env.a = (1.0 - l.env.value) / l.env.attack;
        }
    }

    pub fn note_off(&mut self) {
        for e in &mut self.env {
            e.note_off();
        }
        for l in &mut self.lfo {
            l.env.note_off();
        }
    }

    /// The LFOs, a frame on.
    fn lfo_advance(&mut self, sin: &[f32; 360]) {
        for k in 0..2 {
            let sustain = self.data.lfo_env[k].sustain;
            let max = self.data.lfo_subgr_max[k];
            let l = &mut self.lfo[k];
            l.sub += 1;
            l.env.run(sustain);
            if l.sub > max {
                l.sub = 0;
                l.degree += 1;
                if l.degree > 359 {
                    l.degree = 0;
                }
            }
            l.value = sin[l.degree] * l.env.value;
        }
    }

    /// The envelopes, a frame on; an oscillator whose release has ended
    /// stops.
    fn env_run(&mut self, playing: &mut Playing) {
        let d = self.data;
        if self.env[0].run(d.env[0].sustain) {
            self.speed[0] = 0.0;
            self.speed[2] = 0.0;
            *playing.osc1 = 0;
        }
        if self.env[1].run(d.env[1].sustain) {
            self.speed[1] = 0.0;
            *playing.osc2 = 0;
        }
        self.env_volume[0] = self.env[0].value * d.envm[0].osc1_volume * d.envm[1].osc1_volume;
        self.env_volume[1] = self.env[1].value * d.envm[0].osc2_volume * d.envm[1].osc2_volume;
        self.env_min[0] = if d.lfo[0].osc1_volume == 0.0 && d.lfo[1].osc1_volume == 0.0 { 1.0 } else { 0.0 };
        self.env_min[1] = if d.lfo[0].osc2_volume == 0.0 && d.lfo[1].osc2_volume == 0.0 { 1.0 } else { 0.0 };
    }

    /// The two oscillators' outputs combined.
    fn combine(&self, old: f32, new: f32) -> f32 {
        match self.data.combine {
            1 => old - new,
            2 => old * ((new / 32767.0) * 0.5 + 0.5),
            3 => old / ((new / 32767.0) * 0.5 + 0.5 + 1.0),
            _ => old + new,
        }
    }

    /// A base amount with the four sources' values times what each does
    /// to it added on, one after another as the replayer adds them.
    fn on_base(&self, base: f32, f: impl Fn(&Targets) -> f32) -> f32 {
        base + self.lfo[0].value * f(&self.data.lfo[0]) + self.lfo[1].value * f(&self.data.lfo[1]) + self.env[0].value * f(&self.data.envm[0]) + self.env[1].value * f(&self.data.envm[1])
    }

    /// A modulated amount: the four sources' values times what each
    /// does to it.
    fn modulated(&self, f: impl Fn(&Targets) -> f32) -> f32 {
        self.lfo[0].value * f(&self.data.lfo[0]) + self.lfo[1].value * f(&self.data.lfo[1]) + self.env[0].value * f(&self.data.envm[0]) + self.env[1].value * f(&self.data.envm[1])
    }

    /// The next frame: the left side, and the right added to `right`
    /// when the sample is stereo. `vol` is the voice's level, `speed`
    /// the note's step (1 at the sample's base note).
    pub fn sample(&mut self, t: &Tables, src: &Source, right: &mut f32, vol: f32, playing: &mut Playing, pos: &mut [Pos; 3], speed: f32) -> f32 {
        let d = self.data;
        let (mut gl, mut gr) = (0.0f32, 0.0f32);
        let stereo = src.stereo;
        // Oscillator 1 reads its wave as 1 and 3 do.
        let mut wave1: Option<(&[i16], &[i16])> = None;
        let mut len1 = src.length as i32;
        let mut sub1 = src.loop_sub as i32;
        let mut type1 = src.loop_type;
        let mut osc_speed2 = 0.0f32;
        let lfo1 = self.lfo[0].value;
        let lfo2 = self.lfo[1].value;
        if self.env[0].stage != 0 && d.osc1_wave != NONE {
            let vol1 = (lfo1 * d.lfo[0].osc1_volume + lfo2 * d.lfo[1].osc1_volume + self.env_min[0]) * self.env_volume[0];
            if *playing.osc1 != 0 {
                if self.glide != 0.0 && self.speed[0] != 0.0 {
                    if speed > self.speed[0] {
                        self.speed[0] = (self.speed[0] + self.glide).min(speed);
                    } else {
                        self.speed[0] = (self.speed[0] - self.glide).max(speed);
                    }
                } else {
                    self.speed[0] = speed;
                }
                let mut speed1 = self.modulated(|m| m.osc1_pitch);
                osc_speed2 = self.speed[0];
                if d.osc1_wave != WAV {
                    len1 = CYCLE as i32;
                    sub1 = CYCLE as i32;
                    if let Some(c) = t.cycle(d.osc1_wave) {
                        if d.osc1_wave == WHITE {
                            len1 = NOISE as i32;
                            sub1 = NOISE as i32;
                        }
                        wave1 = Some((c, c));
                    }
                    speed1 *= ADJUST;
                    osc_speed2 *= ADJUST;
                    type1 = LOOP_FORWARD;
                } else if let Some(l) = src.left {
                    wave1 = Some((l, src.right.unwrap_or(l)));
                }
                if let Some((wl, wr)) = wave1 {
                    let at = bounded(pos[0].int, type1, self.backwards[0], len1 as u32, len1 as u32, (len1 - sub1) as u32);
                    let pw = d.osc1_pw * self.modulated(|m| m.osc1_pw);
                    let pl = f32::from(wl.get(at).copied().unwrap_or(0));
                    let ml = if d.osc1_pw != 0.0 && pl > 0.0 { pw * 2.0 } else { 1.0 };
                    gl = pl * ml * vol1;
                    if stereo {
                        let pr = f32::from(wr.get(at).copied().unwrap_or(0));
                        let mr = if d.osc1_pw != 0.0 && pr > 0.0 { pw * 2.0 } else { 1.0 };
                        gr = pr * mr * vol1;
                    }
                    osc_speed2 += speed1;
                    if step_wave(&mut pos[0], osc_speed2, &mut self.backwards[0], type1, len1, sub1).0 {
                        *playing.osc1 = 0;
                    }
                }
            }
        }
        // The sub-oscillator: oscillator 1's wave, an interval down.
        if d.osc3_on && d.osc1_wave != NONE && *playing.osc1 != 0 {
            const INTERVALS: [f32; 13] = [2.0, 1.890_72, 1.786_15, 1.686_31, 1.591_18, 1.500_76, 1.415_08, 1.334_10, 1.257_84, 1.186_30, 1.119_49, 1.057_38, 1.0];
            let step = osc_speed2 / 2.0 * INTERVALS[usize::from(d.osc3_interval.min(12))];
            if let Some((wl, wr)) = wave1 {
                let at = bounded(pos[2].int, type1, self.backwards[2], len1 as u32, len1 as u32, (len1 - sub1) as u32);
                let pw = d.osc1_pw * self.modulated(|m| m.osc1_pw);
                let vol1 = (lfo1 * d.lfo[0].osc1_volume + lfo2 * d.lfo[1].osc1_volume + self.env_min[0]) * self.env_volume[0];
                let pl = f32::from(wl.get(at).copied().unwrap_or(0));
                let ml = if d.osc1_pw != 0.0 && pl > 0.0 { pw * 2.0 } else { 1.0 };
                gl = self.combine(gl, pl * ml * vol1 * d.osc3_volume);
                if stereo {
                    let pr = f32::from(wr.get(at).copied().unwrap_or(0));
                    let mr = if d.osc1_pw != 0.0 && pr > 0.0 { pw * 2.0 } else { 1.0 };
                    gr = self.combine(gr, pr * mr * vol1 * d.osc3_volume);
                }
            }
            if step_wave(&mut pos[2], step, &mut self.backwards[2], type1, len1, sub1).0 {
                *playing.osc1 = 0;
            }
        }
        // Oscillator 2.
        if self.env[1].stage != 0 && d.osc2_wave != NONE && *playing.osc2 != 0 {
            let tune = speed + d.osc2_finetune * 0.125 + d.osc2_detune * 0.125;
            if self.glide != 0.0 && self.speed[1] != 0.0 {
                if tune > self.speed[1] {
                    self.speed[1] = (self.speed[1] + self.glide).min(tune);
                } else {
                    self.speed[1] = (self.speed[1] - self.glide).max(tune);
                }
            } else {
                self.speed[1] = tune;
            }
            let mut speed1b = self.modulated(|m| m.osc2_pitch);
            let mut osc_speed2 = self.speed[1];
            let (mut len2, mut sub2, mut type2) = (src.length as i32, src.loop_sub as i32, src.loop_type);
            let mut wave2: Option<(&[i16], &[i16])> = src.left.map(|l| (l, src.right.unwrap_or(l)));
            if d.osc2_wave != WAV {
                len2 = CYCLE as i32;
                sub2 = CYCLE as i32;
                if let Some(c) = t.cycle(d.osc2_wave) {
                    if d.osc2_wave == WHITE {
                        len2 = NOISE as i32;
                        sub2 = NOISE as i32;
                    }
                    wave2 = Some((c, c));
                }
                speed1b *= ADJUST;
                osc_speed2 *= ADJUST;
                type2 = LOOP_FORWARD;
            }
            if let Some((wl, wr)) = wave2 {
                let vol2 = (lfo1 * d.lfo[0].osc2_volume + lfo2 * d.lfo[1].osc2_volume + self.env_min[1]) * self.env_volume[1];
                let at = bounded(pos[1].int, type2, self.backwards[1], len2 as u32, len2 as u32, (len2 - sub2) as u32);
                let pw = d.osc2_pw * self.modulated(|m| m.osc2_pw);
                let pl = f32::from(wl.get(at).copied().unwrap_or(0));
                let ml = if d.osc2_pw != 0.0 && pl > 0.0 { pw * 2.0 } else { 1.0 };
                gl = self.combine(gl, pl * ml * vol2);
                if stereo {
                    let pr = f32::from(wr.get(at).copied().unwrap_or(0));
                    let mr = if d.osc2_pw != 0.0 && pr > 0.0 { pw * 2.0 } else { 1.0 };
                    gr = self.combine(gr, pr * mr * vol2);
                }
                let (end, round) = step_wave(&mut pos[1], osc_speed2 + speed1b, &mut self.backwards[1], type2, len2, sub2);
                if end {
                    *playing.osc2 = 0;
                }
                // Hard sync: oscillator 2 coming round restarts 1 and 3.
                if d.sync && round {
                    pos[0] = Pos::default();
                    pos[2] = Pos::default();
                }
            }
        }
        // Distortion, then the filter, each side.
        let disto = self.on_base(d.disto, |m| m.disto);
        if d.vcf_type < 2 {
            self.cut = self.on_base(d.cutoff, |m| m.cutoff).clamp(0.05, 0.90);
            self.res = self.on_base(d.resonance, |m| m.resonance).clamp(0.05, 0.98);
            self.filt_a = 1.0 - self.cut;
            self.filt_b = self.res * (1.0 + 1.0 / self.filt_a);
        } else if d.vcf_type > 2 {
            self.cut = (self.on_base(d.cutoff, |m| m.cutoff).max(0.0) + 0.55).min(1.55);
            self.res = self.on_base(d.resonance, |m| m.resonance).clamp(0.0, 1.0);
        }
        let left = self.shape(gl, disto, 0, 0.0);
        if stereo {
            // The four-pole filter's right side filters the left's output.
            *right += self.shape(gr, disto, 1, left) * vol;
        }
        self.env_run(playing);
        self.lfo_advance(&t.degrees);
        left * vol
    }

    /// Distortion and the filter on one side.
    fn shape(&mut self, mut x: f32, disto: f32, side: usize, left_out: f32) -> f32 {
        let d = self.data;
        if d.disto != 0.0 {
            x /= 32767.0;
            x = x * (x.abs() + disto) / (x * x + (disto - 1.0) * x.abs() + 1.0);
            x *= 32767.0;
        }
        if d.vcf_type < 2 {
            let b = &mut self.sbuf[side];
            b[0] = self.filt_a * b[0] + self.cut * (x + self.filt_b * (b[0] - b[1]));
            b[1] = self.filt_a * b[1] + self.cut * b[0];
            if d.vcf_type == 0 {
                b[1]
            } else {
                x - b[1]
            }
        } else if d.vcf_type > 2 {
            let cut = self.cut * 0.5;
            let q = 0.85 - cut;
            let p = cut * cut * 0.45;
            let f = p + p - 1.0;
            let res = self.res * 5.0;
            let q = (res * (1.0 + 0.5 * q * (1.0 - q + 5.6 * q * q))).min(2.42);
            // The replayer's right side filters the left's output.
            let input = if side == 1 { left_out } else { x };
            let m = &mut self.moog[side];
            let i = input / 32767.0 - q * m[4];
            let (t1, t2, t3) = (m[1], m[2], m[3]);
            m[1] = (i + m[0]) * p - m[1] * f;
            m[2] = (m[1] + t1) * p - m[2] * f;
            m[3] = (m[2] + t2) * p - m[3] * f;
            m[4] = (m[3] + t3) * p - m[4] * f;
            m[0] = i;
            if d.vcf_type == 3 {
                m[4] * 32767.0
            } else {
                3.0 * (m[3] - m[4]) * 32767.0
            }
        } else {
            x
        }
    }
}

/// Moves a place along its wave by `step`, round its loop: whether a
/// wave that doesn't loop has ended, and whether it ended or came round
/// (or turned) at all.
fn step_wave(p: &mut Pos, step: f32, backwards: &mut bool, kind: u8, len: i32, sub: i32) -> (bool, bool) {
    if *backwards {
        if p.int > 0 {
            p.flt -= step;
            p.int = p.flt as i32;
        }
    } else {
        p.flt += step;
        p.int = p.flt as i32;
    }
    match kind {
        LOOP_FORWARD => {
            if *backwards {
                if p.int <= len - sub {
                    p.int += len;
                    p.flt += len as f32;
                    return (false, true);
                }
            } else if p.int >= len {
                p.int -= sub;
                p.flt -= sub as f32;
                return (false, true);
            }
            (false, false)
        }
        LOOP_PINGPONG => {
            if *backwards {
                if p.int <= len - sub {
                    *p = Pos::at(len - sub);
                    *backwards = false;
                    return (false, true);
                }
            } else if p.int >= len {
                *p = Pos::at(len);
                *backwards = true;
                return (false, true);
            }
            (false, false)
        }
        _ => {
            if *backwards {
                if p.int <= 0 {
                    *p = Pos::default();
                    return (true, true);
                }
            } else if p.int >= len {
                *p = Pos::at(len);
                return (true, true);
            }
            (false, false)
        }
    }
}
