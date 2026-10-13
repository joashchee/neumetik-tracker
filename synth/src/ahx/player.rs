// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! AHX's replayer: every tick each channel's row is read (on the tick a
//! row starts), then its frame worked (envelope, slides, vibrato, the
//! instrument's play list, the square's pulse width and the filter
//! moving), then its waveform copied out to the 640-point buffer the
//! mixer loops; the mixer steps through it at the channel's period
//! (the Amiga's clock), ring-modulated by a second when HivelyTracker
//! says so.

use std::sync::Arc;

use super::waves::{period, vibrato, waves, NOISE, OFFSETS};
use super::{Instrument, Kind, Song};
use crate::replay::{Bufs, Driver};

/// The Amiga's (PAL) clock, which periods count.
const CLOCK: f64 = 3_546_895.0;
/// Its screen's refresh, which times the ticks: 313 lines of 227 clocks.
const VBLANK: f64 = CLOCK / (313.0 * 227.0);
/// The mixer's buffer, looped.
const BUF: usize = 0x280;
const FIX: i64 = 1 << 16;
/// No transposition set by EF1.
const NO_OVERRIDE: i32 = 1000;

/// Where a waveform comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    None,
    Triangle(usize),
    Sawtooth(usize),
    /// The channel's own square, just made.
    Square,
    Noise(usize),
}

#[derive(Clone, Copy, Debug, Default)]
struct Adsr {
    a_frames: i32,
    a_volume: i32,
    d_frames: i32,
    d_volume: i32,
    s_frames: i32,
    r_frames: i32,
    r_volume: i32,
}

#[derive(Clone)]
struct Voice {
    track: usize,
    transpose: i32,
    next_track: usize,
    override_transpose: i32,
    adsr_volume: i32,
    adsr: Adsr,
    instrument: Option<usize>,
    instr_period: i32,
    track_period: i32,
    vibrato_period: i32,
    note_max_volume: i32,
    perf_sub_volume: i32,
    track_master_volume: i32,
    new_waveform: bool,
    waveform: i32,
    plant_square: bool,
    plant_period: bool,
    ignore_square: bool,
    track_on: bool,
    fixed_note: bool,
    volume_slide_up: i32,
    volume_slide_down: i32,
    hard_cut: i32,
    hard_cut_release: bool,
    hard_cut_release_f: i32,
    period_slide_speed: i32,
    period_slide_period: i32,
    period_slide_limit: i32,
    period_slide_on: bool,
    period_slide_with_limit: bool,
    perf_slide_speed: i32,
    perf_slide_period: i32,
    perf_slide_on: bool,
    vibrato_delay: i32,
    vibrato_current: i32,
    vibrato_depth: i32,
    vibrato_speed: i32,
    square_on: bool,
    square_init: bool,
    square_wait: i32,
    square_lower: i32,
    square_upper: i32,
    square_pos: i32,
    square_sign: i32,
    square_sliding_in: bool,
    filter_on: bool,
    filter_init: bool,
    filter_wait: i32,
    filter_lower: i32,
    filter_upper: i32,
    filter_pos: i32,
    filter_sign: i32,
    filter_speed: i32,
    filter_sliding_in: bool,
    ignore_filter: i32,
    perf_current: usize,
    perf_speed: i32,
    perf_wait: i32,
    wave_length: i32,
    perf_list: bool,
    note_delay_wait: i32,
    note_delay_on: bool,
    note_cut_wait: i32,
    note_cut_on: bool,
    pan: i32,
    set_pan: i32,
    // Ring modulation (HivelyTracker).
    ring_source: Source,
    ring_offset: usize,
    ring_plant_period: bool,
    ring_base_period: i32,
    ring_audio_period: i32,
    ring_new_waveform: bool,
    ring_waveform: i32,
    ring_fixed_period: bool,
    ring_playing: bool,
    audio_source: Source,
    audio_offset: usize,
    audio_period: i32,
    audio_volume: i32,
    noise_random: i32,
    // What the mixer plays.
    voice_volume: i32,
    delta: i64,
    sample_pos: i64,
    ring_delta: i64,
    ring_pos: i64,
    buffer: [i8; BUF + 1],
    ring_buffer: [i8; BUF + 1],
    square_buffer: [i8; 0x80],
    /// Whether the buffer has had a waveform (the mixer plays silence till then).
    sounding: bool,
}

impl Voice {
    fn new(channel: usize, song: &Song) -> Voice {
        const LEFT: [i32; 5] = [128, 96, 64, 32, 0];
        const RIGHT: [i32; 5] = [128, 160, 193, 225, 255];
        let pan = if matches!(channel % 4, 0 | 3) { LEFT[song.stereo] } else { RIGHT[song.stereo] };
        Voice {
            track: 0,
            transpose: 0,
            next_track: 0,
            override_transpose: NO_OVERRIDE,
            adsr_volume: 0,
            adsr: Adsr::default(),
            instrument: None,
            instr_period: 0,
            track_period: 0,
            vibrato_period: 0,
            note_max_volume: 0,
            perf_sub_volume: 0,
            track_master_volume: 0x40,
            new_waveform: false,
            waveform: 0,
            plant_square: false,
            plant_period: false,
            ignore_square: false,
            track_on: true,
            fixed_note: false,
            volume_slide_up: 0,
            volume_slide_down: 0,
            hard_cut: 0,
            hard_cut_release: false,
            hard_cut_release_f: 0,
            period_slide_speed: 0,
            period_slide_period: 0,
            period_slide_limit: 0,
            period_slide_on: false,
            period_slide_with_limit: false,
            perf_slide_speed: 0,
            perf_slide_period: 0,
            perf_slide_on: false,
            vibrato_delay: 0,
            vibrato_current: 0,
            vibrato_depth: 0,
            vibrato_speed: 0,
            square_on: false,
            square_init: false,
            square_wait: 0,
            square_lower: 0,
            square_upper: 0,
            square_pos: 0,
            square_sign: 0,
            square_sliding_in: false,
            filter_on: false,
            filter_init: false,
            filter_wait: 0,
            filter_lower: 0,
            filter_upper: 0,
            filter_pos: 32,
            filter_sign: 0,
            filter_speed: 0,
            filter_sliding_in: false,
            ignore_filter: 0,
            perf_current: 0,
            perf_speed: 0,
            perf_wait: 0,
            wave_length: 0,
            perf_list: false,
            note_delay_wait: 0,
            note_delay_on: false,
            note_cut_wait: 0,
            note_cut_on: false,
            pan,
            set_pan: pan,
            ring_source: Source::None,
            ring_offset: 0,
            ring_plant_period: false,
            ring_base_period: 0,
            ring_audio_period: 0,
            ring_new_waveform: false,
            ring_waveform: 0,
            ring_fixed_period: false,
            ring_playing: false,
            audio_source: Source::None,
            audio_offset: 0,
            audio_period: 0,
            audio_volume: 0,
            noise_random: 0x280,
            voice_volume: 0,
            delta: 1,
            sample_pos: 0,
            ring_delta: 0,
            ring_pos: 0,
            buffer: [0; BUF + 1],
            ring_buffer: [0; BUF + 1],
            square_buffer: [0; 0x80],
            sounding: false,
        }
    }

    /// The envelope's deltas (8.8 a frame) from the instrument's levels.
    fn calc_adsr(&mut self, ins: &Instrument) {
        let e = &ins.envelope;
        let per = |vol: i32, frames: i32| if frames != 0 { vol * 256 / frames } else { vol * 256 };
        self.adsr = Adsr {
            a_frames: e.a_frames,
            a_volume: per(e.a_volume, e.a_frames),
            d_frames: e.d_frames,
            d_volume: if e.d_frames != 0 { (e.d_volume - e.a_volume) * 256 / e.d_frames } else { e.d_volume * 256 },
            s_frames: e.s_frames,
            r_frames: e.r_frames,
            r_volume: if e.r_frames != 0 { (e.r_volume - e.d_volume) * 256 / e.r_frames } else { e.r_volume * 256 },
        };
    }
}

pub struct Player {
    song: Arc<Song>,
    start: usize,
    voices: Vec<Voice>,
    step_wait: i32,
    new_position: bool,
    pattern_break: bool,
    tempo: i32,
    pos: usize,
    pos_jump: usize,
    note: usize,
    pos_jump_note: usize,
    /// The position and row now sounding, for the screen.
    shown: (usize, usize),
    visited: Vec<bool>,
    done: bool,
    /// A speed of 0 was set: the song starts again on the next tick.
    again: bool,
}

impl Player {
    pub fn new(song: Arc<Song>, start: usize) -> Player {
        let mut p = Player { song: song.clone(), start, voices: Vec::new(), step_wait: 0, new_position: true, pattern_break: false, tempo: 6, pos: start, pos_jump: 0, note: 0, pos_jump_note: 0, shown: (start, 0), visited: vec![false; song.positions.len()], done: false, again: false };
        p.restart();
        p
    }

    fn restart(&mut self) {
        let song = self.song.clone();
        self.voices = (0..song.channels).map(|c| Voice::new(c, &song)).collect();
        self.step_wait = 0;
        self.new_position = true;
        self.pattern_break = false;
        self.tempo = 6;
        self.pos = self.start;
        self.pos_jump = 0;
        self.note = 0;
        self.pos_jump_note = 0;
        self.visited.iter_mut().for_each(|v| *v = false);
    }

    fn ahx1(&self) -> bool {
        self.song.kind == Kind::Ahx1
    }

    fn hvl(&self) -> bool {
        self.song.kind == Kind::Hvl
    }

    /// One tick: rows read when one's due, every channel's frame, the
    /// row and position moved on, the mixer told.
    fn play_tick(&mut self) {
        if std::mem::take(&mut self.again) {
            self.restart();
        }
        let song = self.song.clone();
        let n = song.channels;
        if self.step_wait <= 0 {
            if self.new_position {
                let next = if self.pos + 1 >= song.positions.len() { song.restart } else { self.pos + 1 };
                for (i, v) in self.voices.iter_mut().enumerate() {
                    v.track = usize::from(song.positions[self.pos].track[i]);
                    v.transpose = i32::from(song.positions[self.pos].transpose[i]);
                    v.next_track = usize::from(song.positions[next].track[i]);
                }
                self.new_position = false;
                if self.visited[self.pos] {
                    self.done = true;
                    self.visited.iter_mut().for_each(|v| *v = false);
                }
                self.visited[self.pos] = true;
            }
            self.shown = (self.pos, self.note);
            for i in 0..n {
                self.process_step(i);
            }
            self.step_wait = self.tempo;
        }
        for i in 0..n {
            self.process_frame(i);
        }
        if self.tempo > 0 {
            self.step_wait -= 1;
            if self.step_wait <= 0 {
                if !self.pattern_break {
                    self.note += 1;
                    if self.note >= song.track_length {
                        self.pos_jump = self.pos + 1;
                        self.pos_jump_note = 0;
                        self.pattern_break = true;
                    }
                }
                if self.pattern_break {
                    self.pattern_break = false;
                    self.note = self.pos_jump_note;
                    self.pos_jump_note = 0;
                    self.pos = self.pos_jump;
                    self.pos_jump = 0;
                    if self.pos >= song.positions.len() {
                        self.pos = song.restart;
                    }
                    self.new_position = true;
                }
            }
        }
        for i in 0..n {
            self.set_audio(i);
        }
        // A speed of 0 ends the song; it starts again.
        if self.tempo == 0 {
            self.done = true;
            self.again = true;
        }
    }

    fn process_step(&mut self, v: usize) {
        let song = self.song.clone();
        if !self.voices[v].track_on {
            return;
        }
        {
            let voice = &mut self.voices[v];
            voice.volume_slide_up = 0;
            voice.volume_slide_down = 0;
        }
        let step = song.step(self.pos, v, self.note);
        let mut note = i32::from(step.note);
        let instrument = usize::from(step.instrument);
        let fxs = [(step.fx & 0xF, step.param), (step.fx2 & 0xF, step.param2)];
        // A note delay (EDx) waits; once waited, the row plays.
        let mut delayed = false;
        for &(fx, param) in &fxs {
            if delayed {
                break;
            }
            if fx == 0xE && param & 0xF0 == 0xD0 {
                let voice = &mut self.voices[v];
                if voice.note_delay_on {
                    voice.note_delay_on = false;
                    delayed = true;
                } else if i32::from(param & 0xF) < self.tempo {
                    voice.note_delay_wait = i32::from(param & 0xF);
                    if voice.note_delay_wait != 0 {
                        voice.note_delay_on = true;
                        return;
                    }
                }
            }
        }
        if note != 0 {
            self.voices[v].override_transpose = NO_OVERRIDE;
        }
        for &(fx, param) in &fxs {
            self.step_fx1(v, fx, param);
        }
        if instrument != 0 && instrument < song.instruments.len() {
            let ins = &song.instruments[instrument];
            let voice = &mut self.voices[v];
            voice.pan = voice.set_pan;
            voice.period_slide_speed = 0;
            voice.period_slide_period = 0;
            voice.period_slide_limit = 0;
            voice.perf_sub_volume = 0x40;
            voice.adsr_volume = 0;
            voice.instrument = Some(instrument);
            voice.sample_pos = 0;
            voice.calc_adsr(ins);
            voice.wave_length = ins.wave_length;
            voice.note_max_volume = ins.volume;
            voice.vibrato_current = 0;
            voice.vibrato_delay = ins.vibrato_delay;
            voice.vibrato_depth = ins.vibrato_depth;
            voice.vibrato_speed = ins.vibrato_speed;
            voice.vibrato_period = 0;
            voice.hard_cut_release = ins.hard_cut_release;
            voice.hard_cut = ins.hard_cut_frames;
            voice.ignore_square = false;
            voice.square_sliding_in = false;
            voice.square_wait = 0;
            voice.square_on = false;
            let shift = 5 - voice.wave_length;
            let (lo, hi) = (ins.square_lower >> shift, ins.square_upper >> shift);
            voice.square_lower = lo.min(hi);
            voice.square_upper = lo.max(hi);
            voice.ignore_filter = 0;
            voice.filter_wait = 0;
            voice.filter_on = false;
            voice.filter_sliding_in = false;
            let mut speed = ins.filter_speed;
            let (mut lo, mut hi) = (ins.filter_lower, ins.filter_upper);
            if lo & 0x80 != 0 {
                speed |= 0x20;
            }
            if hi & 0x80 != 0 {
                speed |= 0x40;
            }
            voice.filter_speed = speed;
            lo &= !0x80;
            hi &= !0x80;
            voice.filter_lower = lo.min(hi);
            voice.filter_upper = lo.max(hi);
            voice.filter_pos = 32;
            voice.perf_wait = 0;
            voice.perf_current = 0;
            voice.perf_speed = ins.speed;
            voice.perf_list = true;
            voice.ring_playing = false;
            voice.ring_pos = 0;
            voice.ring_plant_period = false;
            voice.ring_new_waveform = false;
        }
        self.voices[v].period_slide_on = false;
        for &(fx, param) in &fxs {
            self.step_fx2(v, fx, param, &mut note);
        }
        if note != 0 {
            let voice = &mut self.voices[v];
            voice.track_period = note;
            voice.plant_period = true;
        }
        for &(fx, param) in &fxs {
            self.step_fx3(v, fx, param);
        }
    }

    /// The commands that work before an instrument is set up.
    fn step_fx1(&mut self, v: usize, fx: u8, param: u8) {
        let p = i32::from(param);
        match fx {
            // The position jump's hundreds.
            0x0 => {
                if (1..=9).contains(&(param & 0xF)) {
                    self.pos_jump = usize::from(param & 0xF);
                }
            }
            0x5 | 0xA => {
                let voice = &mut self.voices[v];
                voice.volume_slide_down = p & 0xF;
                voice.volume_slide_up = p >> 4;
            }
            0x7 => {
                let voice = &mut self.voices[v];
                voice.pan = i32::from(param as i8) + 128;
                voice.set_pan = voice.pan;
            }
            0xB => {
                self.pos_jump = self.pos_jump * 100 + usize::from(param & 0xF) + usize::from(param >> 4) * 10;
                self.pattern_break = true;
            }
            0xD => {
                self.pos_jump = self.pos + 1;
                self.pos_jump_note = if self.ahx1() { 0 } else { usize::from(param & 0xF) + usize::from(param >> 4) * 10 };
                self.pattern_break = true;
                if self.pos_jump_note > self.song.track_length {
                    self.pos_jump_note = 0;
                }
            }
            0xE if param >> 4 == 0xC => {
                if i32::from(param & 0xF) < self.tempo {
                    let voice = &mut self.voices[v];
                    voice.note_cut_wait = i32::from(param & 0xF);
                    if voice.note_cut_wait != 0 {
                        voice.note_cut_on = true;
                        voice.hard_cut_release = false;
                    }
                }
            }
            0xF => self.tempo = p,
            _ => {}
        }
    }

    /// The commands that change the note: tone portamento, the square's offset.
    fn step_fx2(&mut self, v: usize, fx: u8, param: u8, note: &mut i32) {
        let voice = &mut self.voices[v];
        match fx {
            0x9 => {
                voice.square_pos = i32::from(param) >> (5 - voice.wave_length);
                voice.ignore_square = true;
            }
            0x3 | 0x5 => {
                if fx == 0x3 && param != 0 {
                    voice.period_slide_speed = i32::from(param);
                }
                if *note != 0 {
                    let diff = period(voice.track_period) - period(*note);
                    if diff + voice.period_slide_period != 0 {
                        voice.period_slide_limit = -diff;
                    }
                }
                voice.period_slide_on = true;
                voice.period_slide_with_limit = true;
                *note = 0;
            }
            _ => {}
        }
    }

    /// The commands that work once the note's set.
    fn step_fx3(&mut self, v: usize, fx: u8, param: u8) {
        let mut p = i32::from(param);
        let ahx1 = self.ahx1();
        let hvl = self.hvl();
        match fx {
            0x1 => {
                let voice = &mut self.voices[v];
                voice.period_slide_speed = -p;
                voice.period_slide_on = true;
                voice.period_slide_with_limit = false;
            }
            0x2 => {
                let voice = &mut self.voices[v];
                voice.period_slide_speed = p;
                voice.period_slide_on = true;
                voice.period_slide_with_limit = false;
            }
            // The filter's position, or one the play list's next sets.
            0x4 => {
                if ahx1 {
                    p &= 0xF;
                }
                let voice = &mut self.voices[v];
                if p == 0 || p == 0x40 {
                } else if p < 0x40 {
                    voice.ignore_filter = p;
                } else if p <= 0x7F {
                    voice.filter_pos = p - 0x40;
                }
            }
            // Volume: the note's, every channel's (50 to 90) or this one's (A0 to E0).
            0xC => {
                if p <= 0x40 {
                    self.voices[v].note_max_volume = p;
                } else if p >= 0x50 && p - 0x50 <= 0x40 {
                    for voice in self.voices.iter_mut() {
                        voice.track_master_volume = p - 0x50;
                    }
                } else if p >= 0xA0 && p - 0xA0 <= 0x40 {
                    self.voices[v].track_master_volume = p - 0xA0;
                }
            }
            0xE => {
                let voice = &mut self.voices[v];
                let x = p & 0xF;
                match p >> 4 {
                    0x1 => {
                        voice.period_slide_period -= x;
                        voice.plant_period = true;
                    }
                    0x2 => {
                        voice.period_slide_period += x;
                        voice.plant_period = true;
                    }
                    0x4 => voice.vibrato_depth = x,
                    0xA => voice.note_max_volume = (voice.note_max_volume + x).min(0x40),
                    0xB => voice.note_max_volume = (voice.note_max_volume - x).max(0),
                    0xF if hvl && x == 1 => voice.override_transpose = voice.transpose,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn process_frame(&mut self, v: usize) {
        let song = self.song.clone();
        let hvl = song.kind == Kind::Hvl;
        if !self.voices[v].track_on {
            return;
        }
        if self.voices[v].note_delay_on {
            if self.voices[v].note_delay_wait <= 0 {
                self.process_step(v);
            } else {
                self.voices[v].note_delay_wait -= 1;
            }
        }
        let tempo = self.tempo;
        let next_instrument = if self.note + 1 < song.track_length {
            song.tracks.get(self.voices[v].track).and_then(|t| t.get(self.note + 1)).map_or(0, |s| s.instrument)
        } else {
            song.tracks.get(self.voices[v].next_track).and_then(|t| t.first()).map_or(0, |s| s.instrument)
        };
        let ins = self.voices[v].instrument.and_then(|i| song.instruments.get(i));
        let voice = &mut self.voices[v];
        // A hard cut: the note let go a few frames before the next instrument.
        if voice.hard_cut != 0 && next_instrument != 0 {
            let d1 = (tempo - voice.hard_cut).max(0);
            if !voice.note_cut_on {
                voice.note_cut_on = true;
                voice.note_cut_wait = d1;
                voice.hard_cut_release_f = -(d1 - tempo);
            } else {
                voice.hard_cut = 0;
            }
        }
        if voice.note_cut_on {
            if voice.note_cut_wait <= 0 {
                voice.note_cut_on = false;
                if voice.hard_cut_release {
                    voice.adsr.r_frames = voice.hard_cut_release_f;
                    voice.adsr.r_volume = 0;
                    if voice.adsr.r_frames > 0 {
                        let r = ins.map_or(0, |i| i.envelope.r_volume);
                        voice.adsr.r_volume = -(voice.adsr_volume - (r << 8)) / voice.adsr.r_frames;
                    }
                    voice.adsr.a_frames = 0;
                    voice.adsr.d_frames = 0;
                    voice.adsr.s_frames = 0;
                } else {
                    voice.note_max_volume = 0;
                }
            } else {
                voice.note_cut_wait -= 1;
            }
        }
        // The envelope.
        let env = ins.map(|i| i.envelope).unwrap_or_default();
        if voice.adsr.a_frames != 0 {
            voice.adsr_volume += voice.adsr.a_volume;
            voice.adsr.a_frames -= 1;
            if voice.adsr.a_frames <= 0 {
                voice.adsr_volume = env.a_volume << 8;
            }
        } else if voice.adsr.d_frames != 0 {
            voice.adsr_volume += voice.adsr.d_volume;
            voice.adsr.d_frames -= 1;
            if voice.adsr.d_frames <= 0 {
                voice.adsr_volume = env.d_volume << 8;
            }
        } else if voice.adsr.s_frames != 0 {
            voice.adsr.s_frames -= 1;
        } else if voice.adsr.r_frames != 0 {
            voice.adsr_volume += voice.adsr.r_volume;
            voice.adsr.r_frames -= 1;
            if voice.adsr.r_frames <= 0 {
                voice.adsr_volume = env.r_volume << 8;
            }
        }
        voice.note_max_volume = (voice.note_max_volume + voice.volume_slide_up - voice.volume_slide_down).clamp(0, 0x40);
        // Portamento.
        if voice.period_slide_on {
            if voice.period_slide_with_limit {
                let mut d0 = voice.period_slide_period - voice.period_slide_limit;
                let mut d2 = voice.period_slide_speed;
                if d0 > 0 {
                    d2 = -d2;
                }
                if d0 != 0 {
                    d0 = if ((d0 + d2) ^ d0) >= 0 { voice.period_slide_period + d2 } else { voice.period_slide_limit };
                    voice.period_slide_period = d0;
                    voice.plant_period = true;
                }
            } else {
                voice.period_slide_period += voice.period_slide_speed;
                voice.plant_period = true;
            }
        }
        if voice.vibrato_depth != 0 {
            if voice.vibrato_delay <= 0 {
                voice.vibrato_period = (vibrato(voice.vibrato_current as usize) * voice.vibrato_depth) >> 7;
                voice.plant_period = true;
                voice.vibrato_current = (voice.vibrato_current + voice.vibrato_speed) & 0x3F;
            } else {
                voice.vibrato_delay -= 1;
            }
        }
        // The play list.
        let mut commands = None;
        if voice.perf_list {
            match ins {
                Some(ins) if voice.perf_current < ins.list.len() => {
                    // Its wait counts as a signed byte (128 waits at once).
                    let overflow = voice.perf_wait == 128;
                    voice.perf_wait -= 1;
                    if overflow || (voice.perf_wait as i8) <= 0 {
                        let entry = ins.list[voice.perf_current];
                        voice.perf_current += 1;
                        voice.perf_wait = voice.perf_speed;
                        if entry.waveform != 0 {
                            voice.waveform = i32::from(entry.waveform) - 1;
                            voice.new_waveform = true;
                            voice.perf_slide_speed = 0;
                            voice.perf_slide_period = 0;
                        }
                        voice.perf_slide_on = false;
                        commands = Some(entry);
                    }
                }
                _ => {
                    if voice.perf_wait != 0 {
                        voice.perf_wait -= 1;
                    } else {
                        voice.perf_slide_speed = 0;
                    }
                }
            }
        }
        if let Some(entry) = commands {
            for k in 0..2 {
                self.plist_command(v, entry.fx[k], entry.param[k]);
            }
            if entry.note != 0 {
                let voice = &mut self.voices[v];
                voice.instr_period = i32::from(entry.note);
                voice.plant_period = true;
                voice.fixed_note = entry.fixed;
            }
        }
        let voice = &mut self.voices[v];
        if voice.perf_slide_on {
            voice.perf_slide_period -= voice.perf_slide_speed;
            if voice.perf_slide_period != 0 {
                voice.plant_period = true;
            }
        }
        // The square's pulse width moving.
        if voice.waveform == 2 && voice.square_on {
            voice.square_wait -= 1;
            if voice.square_wait <= 0 {
                let (d1, d2) = (voice.square_lower, voice.square_upper);
                let mut d3 = voice.square_pos;
                if voice.square_init {
                    voice.square_init = false;
                    if d3 <= d1 {
                        voice.square_sliding_in = true;
                        voice.square_sign = 1;
                    } else if d3 >= d2 {
                        voice.square_sliding_in = true;
                        voice.square_sign = -1;
                    }
                }
                if d1 == d3 || d2 == d3 {
                    if voice.square_sliding_in {
                        voice.square_sliding_in = false;
                    } else {
                        voice.square_sign = -voice.square_sign;
                    }
                }
                d3 += voice.square_sign;
                voice.square_pos = d3;
                voice.plant_square = true;
                voice.square_wait = ins.map_or(0, |i| i.square_speed);
            }
        }
        // The filter moving.
        if voice.filter_on {
            voice.filter_wait -= 1;
            if voice.filter_wait <= 0 {
                let (d1, d2) = (voice.filter_lower, voice.filter_upper);
                let mut d3 = voice.filter_pos;
                if voice.filter_init {
                    voice.filter_init = false;
                    if d3 <= d1 {
                        voice.filter_sliding_in = true;
                        voice.filter_sign = 1;
                    } else if d3 >= d2 {
                        voice.filter_sliding_in = true;
                        voice.filter_sign = -1;
                    }
                }
                let steps = if voice.filter_speed < 4 { 5 - voice.filter_speed } else { 1 };
                for _ in 0..steps {
                    if d1 == d3 || d2 == d3 {
                        if voice.filter_sliding_in {
                            voice.filter_sliding_in = false;
                        } else {
                            voice.filter_sign = -voice.filter_sign;
                        }
                    }
                    d3 += voice.filter_sign;
                }
                voice.filter_pos = d3.clamp(1, 63);
                voice.new_waveform = true;
                voice.filter_wait = (voice.filter_speed - 3).max(1);
            }
        }
        let w = waves(hvl);
        if voice.waveform == 2 || voice.plant_square {
            // The square, at its pulse width, from the filter's set.
            let set = &w.sets[(voice.filter_pos - 1).clamp(0, 62) as usize];
            let mut x = voice.square_pos << (5 - voice.wave_length);
            if x > 0x20 {
                x = 0x40 - x;
            }
            let mut at = if x > 0 { ((x - 1) << 7) as usize } else { 0 };
            let step = (32 >> voice.wave_length) as usize;
            for i in 0..(1usize << voice.wave_length) * 4 {
                voice.square_buffer[i] = set.squares[at.min(0x80 * 0x20 - 1)];
                at += step;
            }
            voice.new_waveform = true;
            voice.waveform = 2;
            voice.plant_square = false;
        }
        if voice.waveform == 3 {
            voice.new_waveform = true;
        }
        if voice.ring_new_waveform {
            voice.ring_waveform = voice.ring_waveform.min(1);
            voice.ring_source = if voice.ring_waveform == 0 { Source::Triangle(31) } else { Source::Sawtooth(31) };
            voice.ring_offset = OFFSETS[voice.wave_length as usize];
        }
        if voice.new_waveform {
            let set = (voice.filter_pos - 1).clamp(0, 62) as usize;
            voice.audio_source = match voice.waveform {
                0 => Source::Triangle(set),
                1 => Source::Sawtooth(set),
                2 => Source::Square,
                3 => Source::Noise(set),
                _ => Source::None,
            };
            voice.audio_offset = 0;
            if voice.waveform < 2 {
                voice.audio_offset = OFFSETS[voice.wave_length as usize];
            }
            if voice.waveform == 3 {
                // The noise read from somewhere new each frame: AHX rotates
                // its seed; HivelyTracker's shift of it keeps the sign.
                voice.audio_offset = ((voice.noise_random & (2 * 0x280 - 1)) & !1) as usize;
                let r = voice.noise_random.wrapping_add(2_239_384);
                let turned = if hvl { (r >> 8) | (r << 24) } else { r.rotate_right(8) };
                voice.noise_random = (turned.wrapping_add(782_323) ^ 75).wrapping_sub(6735);
            }
        }
        let transpose = if voice.override_transpose != NO_OVERRIDE { voice.override_transpose } else { voice.transpose };
        if voice.ring_source != Source::None {
            let mut p = voice.ring_base_period;
            if !voice.ring_fixed_period {
                p += transpose + voice.track_period - 1;
            }
            let mut p = period(p.clamp(0, 60));
            if !voice.ring_fixed_period {
                p += voice.period_slide_period;
            }
            p += voice.perf_slide_period + voice.vibrato_period;
            voice.ring_audio_period = p.clamp(0x71, 0xD60);
        }
        let mut p = voice.instr_period;
        if !voice.fixed_note {
            p += transpose + voice.track_period - 1;
        }
        let mut p = period(p.clamp(0, 60));
        if !voice.fixed_note {
            p += voice.period_slide_period;
        }
        p += voice.perf_slide_period + voice.vibrato_period;
        voice.audio_period = p.clamp(0x71, 0xD60);
        voice.audio_volume = (((((voice.adsr_volume >> 8) * voice.note_max_volume) >> 6) * voice.perf_sub_volume >> 6) * voice.track_master_volume) >> 6;
    }

    fn plist_command(&mut self, v: usize, fx: u8, param: u8) {
        let ahx1 = self.ahx1();
        let hvl = self.hvl();
        let voice = &mut self.voices[v];
        let p = i32::from(param);
        match fx {
            0 => {
                if !ahx1 && p > 0 && p < 0x40 {
                    if voice.ignore_filter != 0 {
                        voice.filter_pos = voice.ignore_filter;
                        voice.ignore_filter = 0;
                    } else {
                        voice.filter_pos = p;
                    }
                    voice.new_waveform = true;
                }
            }
            1 => {
                voice.perf_slide_speed = p;
                voice.perf_slide_on = true;
            }
            2 => {
                voice.perf_slide_speed = -p;
                voice.perf_slide_on = true;
            }
            3 => {
                if !voice.ignore_square {
                    voice.square_pos = p >> (5 - voice.wave_length);
                } else {
                    voice.ignore_square = false;
                }
            }
            4 => {
                if ahx1 || p == 0 {
                    voice.square_on = !voice.square_on;
                    voice.square_init = voice.square_on;
                    voice.square_sign = 1;
                } else {
                    if p & 0x0F != 0 {
                        voice.square_on = !voice.square_on;
                        voice.square_init = voice.square_on;
                        voice.square_sign = if p & 0x0F == 0x0F { -1 } else { 1 };
                    }
                    if p & 0xF0 != 0 {
                        voice.filter_on = !voice.filter_on;
                        voice.filter_init = voice.filter_on;
                        voice.filter_sign = if p & 0xF0 == 0xF0 { -1 } else { 1 };
                    }
                }
            }
            5 => voice.perf_current = usize::from(param),
            // Ring modulation by a triangle (7) or a sawtooth (8).
            7 | 8 if hvl => {
                if (1..=0x3C).contains(&p) {
                    voice.ring_base_period = p;
                    voice.ring_fixed_period = true;
                } else if (0x81..=0xBC).contains(&p) {
                    voice.ring_base_period = p - 0x80;
                    voice.ring_fixed_period = false;
                } else {
                    voice.ring_base_period = 0;
                    voice.ring_fixed_period = false;
                    voice.ring_new_waveform = false;
                    voice.ring_source = Source::None;
                    voice.ring_offset = 0;
                    voice.ring_playing = false;
                    return;
                }
                voice.ring_waveform = i32::from(fx == 8);
                voice.ring_new_waveform = true;
                voice.ring_plant_period = true;
            }
            9 if hvl => voice.pan = i32::from(param as i8) + 128,
            12 => {
                if p <= 0x40 {
                    voice.note_max_volume = p;
                } else if p >= 0x50 && p - 0x50 <= 0x40 {
                    voice.perf_sub_volume = p - 0x50;
                } else if p >= 0xA0 && p - 0xA0 <= 0x40 {
                    voice.track_master_volume = p - 0xA0;
                }
            }
            15 => {
                voice.perf_speed = p;
                voice.perf_wait = p;
            }
            _ => {}
        }
    }

    /// The mixer's settings from the frame: volume, period, the waveform
    /// copied out to the buffer (and the ring modulator's).
    fn set_audio(&mut self, v: usize) {
        let voice = &mut self.voices[v];
        if !voice.track_on {
            voice.voice_volume = 0;
            return;
        }
        voice.voice_volume = voice.audio_volume;
        if voice.plant_period {
            voice.plant_period = false;
            voice.delta = step_of(voice.audio_period);
        }
        let w = waves(self.song.kind == Kind::Hvl);
        if voice.new_waveform {
            let wl = voice.wave_length as usize;
            if voice.waveform == 3 {
                if let Source::Noise(s) = voice.audio_source {
                    let at = voice.audio_offset.min(NOISE - BUF);
                    voice.buffer[..BUF].copy_from_slice(&w.sets[s].noise[at..at + BUF]);
                }
            } else {
                let len = 4 << wl;
                let src: Option<&[i8]> = match voice.audio_source {
                    Source::Triangle(s) => Some(&w.sets[s].triangles[voice.audio_offset..voice.audio_offset + len]),
                    Source::Sawtooth(s) => Some(&w.sets[s].sawtooths[voice.audio_offset..voice.audio_offset + len]),
                    Source::Square => Some(&voice.square_buffer[..len]),
                    _ => None,
                };
                let src: Vec<i8> = src.map_or_else(|| vec![0; len], |s| s.to_vec());
                for chunk in voice.buffer[..BUF].chunks_exact_mut(len) {
                    chunk.copy_from_slice(&src);
                }
            }
            voice.buffer[BUF] = voice.buffer[0];
            voice.sounding = true;
        }
        if voice.ring_plant_period {
            voice.ring_plant_period = false;
            voice.ring_delta = step_of(voice.ring_audio_period);
        }
        if voice.ring_new_waveform {
            let len = 4 << voice.wave_length as usize;
            let src: Vec<i8> = match voice.ring_source {
                Source::Triangle(s) => w.sets[s].triangles[voice.ring_offset..voice.ring_offset + len].to_vec(),
                Source::Sawtooth(s) => w.sets[s].sawtooths[voice.ring_offset..voice.ring_offset + len].to_vec(),
                _ => vec![0; len],
            };
            for chunk in voice.ring_buffer[..BUF].chunks_exact_mut(len) {
                chunk.copy_from_slice(&src);
            }
            voice.ring_buffer[BUF] = voice.ring_buffer[0];
            voice.ring_playing = true;
        }
    }
}

/// A period's step through the buffer a frame, in 16.16, at the mixer's
/// rate (set when rendering; this is at 1 Hz).
fn step_of(period: i32) -> i64 {
    // Kept as the frequency in 16.16; `render` divides by the rate.
    (CLOCK * FIX as f64 / f64::from(period.max(1))) as i64
}

impl Driver for Player {
    fn channels(&self) -> usize {
        self.song.channels
    }

    fn tick(&mut self) {
        self.play_tick();
    }

    fn tick_seconds(&self) -> f64 {
        // AHX on the Amiga, by its screen; HivelyTracker at 50 a second.
        let hz = if self.hvl() { 50.0 } else { VBLANK };
        1.0 / (hz * f64::from(self.song.speed_multiplier.max(1)))
    }

    fn render(&mut self, rate: u32, frames: usize, out: &mut Bufs) {
        let gain = self.song.mix_gain as f32 / 256.0;
        let wrap = BUF as i64 * FIX;
        for (c, voice) in self.voices.iter_mut().enumerate() {
            let (l, r) = (&mut out.left[c][..frames], &mut out.right[c][..frames]);
            l.fill(0.0);
            r.fill(0.0);
            if !voice.sounding || voice.voice_volume == 0 {
                // The buffer still moves on.
                let d = (voice.delta / i64::from(rate)).max(1);
                voice.sample_pos = (voice.sample_pos + d * frames as i64) % wrap;
                if voice.ring_playing {
                    let rd = (voice.ring_delta / i64::from(rate)).max(1);
                    voice.ring_pos = (voice.ring_pos + rd * frames as i64) % wrap;
                }
                continue;
            }
            let mut d = voice.delta / i64::from(rate);
            if d > wrap {
                d -= wrap;
            }
            let d = d.max(1);
            let mut rd = voice.ring_delta / i64::from(rate);
            if rd > wrap {
                rd -= wrap;
            }
            let rd = rd.max(1);
            let pan = voice.pan.clamp(0, 255) as f32;
            // HivelyTracker's pan law: a quarter sine across.
            let angle = std::f32::consts::FRAC_PI_2 * pan / 256.0;
            let (pl, pr) = if voice.pan >= 255 { (0.0, 1.0) } else if voice.pan <= 0 { (1.0, 0.0) } else { (angle.cos(), angle.sin()) };
            let vol = voice.voice_volume as f32 / 64.0 * gain * 0.4 / 128.0;
            let (gl, gr) = (vol * pl, vol * pr);
            for i in 0..frames {
                if voice.sample_pos >= wrap {
                    voice.sample_pos -= wrap;
                }
                let mut x = f32::from(voice.buffer[(voice.sample_pos >> 16) as usize]);
                if voice.ring_playing {
                    if voice.ring_pos >= wrap {
                        voice.ring_pos -= wrap;
                    }
                    x = ((i32::from(voice.buffer[(voice.sample_pos >> 16) as usize]) * i32::from(voice.ring_buffer[(voice.ring_pos >> 16) as usize])) >> 7) as f32;
                    voice.ring_pos += rd;
                }
                l[i] = x * gl;
                r[i] = x * gr;
                voice.sample_pos += d;
            }
        }
    }

    fn position(&self) -> (usize, usize) {
        (self.shown.0.saturating_sub(self.start), self.shown.1)
    }

    fn speed(&self) -> (u32, u32) {
        (self.tempo.max(0) as u32, 125 * self.song.speed_multiplier.max(1) as u32)
    }

    fn take_done(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}
