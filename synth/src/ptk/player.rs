//! The replayer: a row read at its first frame, its effects worked at
//! each sixth of it, each track's sub-channels (a sample, a synth, or
//! both) mixed, through the track's filter, distortion, flanger,
//! compressor, EQ, panning, LFOs and volume, into the mix with its
//! sends to the chorus and the reverb; then the mix's compressor and
//! volume, clipped.

use std::sync::Arc;

use super::dsp::{Chorus, CoefCache, Compressor, Eq, Flanger, Reverb, TrackFilter, COMP_WINDOW};
use super::tb303::Gear;
use super::voice::{bounded, Playing, Pos, Source, Tables, Voice, LOOP_FORWARD, LOOP_NONE, LOOP_PINGPONG, RELEASE, WAV};
use super::*;

const NOSAMPLE: u8 = 0;
const SAMPLE: u8 = 1;
const NOTEOFF: u8 = 2;
const STOCK: u8 = 3;

/// A note waiting for its sub-channel's last to fade.
#[derive(Clone, Copy, Debug, Default)]
struct Schedule {
    inote: i32,
    sample: i32,
    vol: f32,
    vol_synth: f32,
    offset: u32,
    glide: bool,
    start_backward: bool,
    age: u64,
}

/// One of a track's sub-channels.
#[derive(Clone, Debug)]
struct Sub {
    synth: Voice,
    synth_was: u8,
    synth_used: u8,
    stage: u8,
    stage2: u8,
    stage3: u8,
    cut_stage: bool,
    glide_stage: bool,
    pos: Pos,
    osc: [Pos; 3],
    vstep: f32,
    step: f32,
    vstep_arp: f32,
    arp_base: f32,
    vstep_vib: f32,
    vib_base: f32,
    tvol: f32,
    cvol: f32,
    cvol_ramp: f32,
    cvol_dest: f32,
    tvol_synth: f32,
    cvol_synth: f32,
    cvol_synth_ramp: f32,
    cvol_synth_dest: f32,
    channel_sample: i32,
    channel_note: i32,
    /// The sample it reads (instrument, split), and how.
    wave: Option<(usize, usize)>,
    ampli: f32,
    channels: u8,
    loop_type: u8,
    backwards: bool,
    loop_start: u32,
    loop_end: u32,
    loop_len: u32,
    length: u32,
    old_note: i32,
    sched: Schedule,
}

impl Default for Sub {
    fn default() -> Sub {
        let mut synth = Voice::default();
        synth.reset();
        Sub {
            synth,
            synth_was: 0,
            synth_used: 0,
            stage: NOSAMPLE,
            stage2: NOSAMPLE,
            stage3: NOSAMPLE,
            cut_stage: false,
            glide_stage: false,
            pos: Pos::default(),
            osc: [Pos::default(); 3],
            vstep: 0.0,
            step: 0.0,
            vstep_arp: 0.0,
            arp_base: 0.0,
            vstep_vib: 0.0,
            vib_base: 0.0,
            tvol: 0.0,
            cvol: 0.0,
            cvol_ramp: 0.0,
            cvol_dest: 10.0,
            tvol_synth: 0.0,
            cvol_synth: 0.0,
            cvol_synth_ramp: 0.0,
            cvol_synth_dest: 10.0,
            channel_sample: -1,
            channel_note: 120,
            wave: None,
            ampli: 0.0,
            channels: 0,
            loop_type: 0,
            backwards: false,
            loop_start: 0,
            loop_end: 0,
            loop_len: 0,
            length: 0,
            old_note: 0,
            sched: Schedule::default(),
        }
    }
}

/// A track as it plays.
struct TrackState {
    subs: Vec<Sub>,
    reserved: [i32; POLYPHONY],
    note_sub: [i32; POLYPHONY],
    old_sample: [u8; POLYPHONY],
    tvol_mod: f32,
    pan: f32,
    old_pan: f32,
    lvol: f32,
    rvol: f32,
    old_lvol: f32,
    old_rvol: f32,
    tcut: f32,
    ccut: f32,
    icut: f32,
    ftype: i32,
    frez: i32,
    ramper: f32,
    decay: f32,
    dist_threshold: f32,
    dist_clamp: f32,
    reverb_send: f32,
    chorus_send: f32,
    filter: TrackFilter,
    flanger_on: bool,
    flanger: Flanger,
    lfo_on: bool,
    lfo_rate: f32,
    lfo_scale: f32,
    lfo_filter: f32,
    lfo_volume: f32,
    lfo_panning: f32,
    carrier: [f32; 3],
    compress: bool,
    comp_threshold_raw: f32,
    comp_ratio_raw: f32,
    comp_threshold: f32,
    comp_ratio: f32,
    comp: [Compressor; 2],
    eq: Eq,
    denoise: u8,
    history: [[f32; 32]; 2],
    history_pos: usize,
    glide: bool,
    glide_step: f32,
    arpeggio: u8,
    vibrato: u8,
    vibcounter: i32,
    reverse: bool,
    reverse_dat: u8,
    fade_mode: u8,
    fade_coef: f32,
    new_instrument: bool,
    segue_pos: i32,
    segue_volume: f32,
    segue: [f32; 2],
    level: f32,
}

/// ProTrekkr's replayer for a song.
pub struct Machine {
    song: Arc<Song>,
    t: Tables,
    coefs: CoefCache,
    tracks: Vec<TrackState>,
    gears: [Gear; 2],
    tb: [Tb303; 2],
    track303: [usize; 2],
    chorus: Chorus,
    reverb: Reverb,
    comp_pos: usize,
    // Timing.
    samples_per_tick: i32,
    samples_per_sub: i32,
    pos_in_tick: i32,
    sub_counter: i32,
    subicounter: i32,
    bpm: i32,
    tpb: i32,
    shuffle: i32,
    shufflestep: i32,
    shuffleswitch: i32,
    position: usize,
    line: usize,
    patbreak: i32,
    pos_jump: i32,
    playing: bool,
    done: bool,
    loop_pos: i32,
    loop_counter: i32,
    loop_in: bool,
    pl_note: [u8; POLYPHONY],
    /// The tracks whose notes have been let go at each position (they're
    /// let go once, as the tracker's replayer does).
    history: Vec<[bool; TRACKS]>,
    frames: u64,
    rand: u32,
    // The mix.
    local_mas_vol: f32,
    curr_mas_vol: f32,
    local_ramp_vol: f32,
    curr_ramp_vol: f32,
    reverb_cutoff: f32,
    reverb_resonance: f32,
    reverb_damp: f32,
    chorus_filter: [i32; 3],
    /// Only the song's timing is worked out (no sound).
    timing_only: bool,
    /// Tracks the host has muted, and their faders.
    muted: [bool; TRACKS],
    gains: [f32; TRACKS],
    // Resampling to the host's rate.
    frac: f64,
    last: (f32, f32),
    next: (f32, f32),
}

impl Machine {
    /// The replayer at the song's start (it allocates its delay lines).
    pub fn new(song: Arc<Song>) -> Machine {
        let tracks = (0..TRACKS)
            .map(|_| TrackState {
                subs: vec![Sub::default(); POLYPHONY],
                reserved: [-1; POLYPHONY],
                note_sub: [-1; POLYPHONY],
                old_sample: [0; POLYPHONY],
                tvol_mod: 1.0,
                pan: 0.5,
                old_pan: 0.5,
                lvol: 0.0,
                rvol: 0.0,
                old_lvol: 0.0,
                old_rvol: 0.0,
                tcut: 126.0,
                ccut: 0.0,
                icut: 0.0039,
                ftype: 4,
                frez: 64,
                ramper: 0.0,
                decay: 0.0,
                dist_threshold: 32767.0,
                dist_clamp: 32767.0,
                reverb_send: 0.0,
                chorus_send: 0.0,
                filter: TrackFilter::default(),
                flanger_on: false,
                flanger: Flanger::new(),
                lfo_on: false,
                lfo_rate: 0.0,
                lfo_scale: 1.0,
                lfo_filter: 0.0,
                lfo_volume: 0.0,
                lfo_panning: 0.0,
                carrier: [0.0; 3],
                compress: false,
                comp_threshold_raw: 100.0,
                comp_ratio_raw: 0.0,
                comp_threshold: 0.1,
                comp_ratio: 0.0,
                comp: [Compressor::new(), Compressor::new()],
                eq: Eq::default(),
                denoise: 0,
                history: [[0.0; 32]; 2],
                history_pos: 0,
                glide: false,
                glide_step: 0.0,
                arpeggio: 0,
                vibrato: 0,
                vibcounter: 0,
                reverse: false,
                reverse_dat: 0,
                fade_mode: 0,
                fade_coef: 0.0,
                new_instrument: false,
                segue_pos: 0,
                segue_volume: 0.0,
                segue: [0.0; 2],
                level: 0.0,
            })
            .collect();
        let mut m = Machine {
            t: Tables::new(),
            coefs: CoefCache::new(),
            tracks,
            gears: [Gear::new(song.tb303[0].volume), Gear::new(song.tb303[1].volume)],
            tb: [song.tb303[0].clone(), song.tb303[1].clone()],
            track303: [255; 2],
            chorus: Chorus::new(),
            reverb: Reverb::new(),
            comp_pos: 0,
            samples_per_tick: 1,
            samples_per_sub: 1,
            pos_in_tick: 0,
            sub_counter: 0,
            subicounter: 0,
            bpm: 125,
            tpb: 4,
            shuffle: 0,
            shufflestep: 0,
            shuffleswitch: -1,
            position: 0,
            line: 0,
            patbreak: 255,
            pos_jump: -1,
            playing: true,
            done: false,
            loop_pos: 0,
            loop_counter: 0,
            loop_in: false,
            pl_note: [NO_NOTE; POLYPHONY],
            history: vec![[false; TRACKS]; POSITIONS],
            frames: 0,
            rand: 0x1234_5678,
            local_mas_vol: 1.0,
            curr_mas_vol: 0.0,
            local_ramp_vol: 1.0,
            curr_ramp_vol: 0.0,
            reverb_cutoff: 0.08,
            reverb_resonance: 0.2,
            reverb_damp: 1.0,
            chorus_filter: [4, 0, 64],
            timing_only: false,
            muted: [false; TRACKS],
            gains: [1.0; TRACKS],
            frac: 1.0,
            last: (0.0, 0.0),
            next: (0.0, 0.0),
            song,
        };
        for (mu, t) in m.muted.iter_mut().zip(&m.song.tracks) {
            *mu = t.muted;
        }
        m.seek(0, 0);
        m
    }

    /// Fades the song out over a few milliseconds (a stop), as the
    /// replayer does; `faded` says when it's silent.
    pub fn fade_out(&mut self) {
        self.local_ramp_vol = 0.0;
    }

    pub fn faded(&self) -> bool {
        self.local_ramp_vol == 0.0 && self.curr_ramp_vol <= 0.0
    }

    pub fn song(&self) -> &Arc<Song> {
        &self.song
    }

    /// Plays from a position and row, everything as the song starts
    /// (ProTrekkr's play from a place).
    pub fn seek(&mut self, position: usize, row: usize) {
        let song = self.song.clone();
        for (t, st) in self.tracks.iter_mut().zip(&song.tracks) {
            for s in t.subs.iter_mut() {
                *s = Sub::default();
            }
            t.reserved = [-1; POLYPHONY];
            t.note_sub = [-1; POLYPHONY];
            t.old_sample = [0; POLYPHONY];
            t.tvol_mod = 1.0;
            t.pan = st.pan;
            t.old_pan = st.pan;
            t.tcut = st.cutoff;
            t.ccut = 0.0;
            t.icut = st.inertia;
            t.ftype = st.filter;
            t.frez = st.resonance;
            t.ramper = 0.0;
            t.decay = 0.0;
            t.dist_threshold = st.dist_threshold;
            t.dist_clamp = st.dist_clamp;
            t.reverb_send = st.reverb_send;
            t.chorus_send = st.chorus_send as f32 / 127.0;
            t.filter = TrackFilter::default();
            t.flanger_on = st.flanger.on;
            t.flanger.clear(st.flanger.delay);
            t.flanger.phase = 0.0;
            t.lfo_on = st.lfo.on;
            t.lfo_rate = st.lfo.rate;
            t.lfo_scale = st.lfo.scale;
            t.lfo_filter = st.lfo.filter;
            t.lfo_volume = st.lfo.volume;
            t.lfo_panning = st.lfo.panning;
            t.carrier = [0.0; 3];
            t.compress = st.compress;
            set_comp(&mut t.comp_threshold_raw, &mut t.comp_ratio_raw, &mut t.comp_threshold, &mut t.comp_ratio, st.comp_threshold, st.comp_ratio);
            t.comp[0].clear();
            t.comp[1].clear();
            t.eq = Eq::default();
            t.denoise = st.denoise.min(4);
            t.history = [[0.0; 32]; 2];
            t.history_pos = 0;
            t.glide = false;
            t.glide_step = 0.0;
            t.arpeggio = 0;
            t.vibrato = 0;
            t.vibcounter = 0;
            t.reverse = false;
            t.reverse_dat = 0;
            t.fade_mode = 0;
            t.fade_coef = 0.0;
            t.new_instrument = false;
            t.segue_pos = 0;
            t.segue_volume = 0.0;
            t.segue = [0.0; 2];
            t.level = 0.0;
            let (l, r) = pan_gains(&self.t, t.pan);
            t.old_lvol = l;
            t.old_rvol = r;
            t.lvol = l;
            t.rvol = r;
        }
        self.tb = [song.tb303[0].clone(), song.tb303[1].clone()];
        for (g, tb) in self.gears.iter_mut().zip(&song.tb303) {
            g.volume = tb.volume;
            g.reset();
        }
        self.track303 = [255; 2];
        self.chorus.clear(song.chorus_delay);
        self.reverb.clear(song.reverb_threshold, song.reverb_stereo);
        self.comp_pos = 0;
        self.bpm = song.bpm.max(1);
        self.tpb = song.tpb.clamp(1, 32);
        self.shuffle = song.shuffle;
        self.calc_tempo();
        self.shufflestep = 0;
        self.shuffleswitch = -1;
        self.pos_in_tick = 0;
        self.sub_counter = 0;
        self.subicounter = 0;
        self.position = position.min(song.length.max(1) - 1);
        self.line = row;
        if self.line >= song.pattern_at(self.position).1 {
            self.line = 0;
        }
        self.patbreak = 255;
        self.pos_jump = -1;
        self.playing = true;
        self.done = false;
        self.loop_pos = 0;
        self.loop_counter = 0;
        self.loop_in = false;
        self.pl_note = [NO_NOTE; POLYPHONY];
        for h in self.history.iter_mut() {
            *h = [false; TRACKS];
        }
        self.local_mas_vol = 1.0;
        self.curr_mas_vol = 0.0;
        self.local_ramp_vol = 1.0;
        self.curr_ramp_vol = 0.0;
        self.reverb_cutoff = song.reverb_cutoff;
        self.reverb_resonance = song.reverb_resonance;
        self.reverb_damp = song.reverb_damp;
        self.chorus_filter = song.chorus_filter;
        self.frac = 1.0;
        self.last = (0.0, 0.0);
        self.next = (0.0, 0.0);
    }

    /// Where the song is: its position in the sequence and its row.
    pub fn position(&self) -> (usize, usize) {
        (self.position, self.line)
    }

    /// The song still reads rows (a speed of 0 stops it).
    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Whether the song came round (or stopped) since this was last asked.
    pub fn take_done(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }

    /// The tempo: beats a minute and rows a beat.
    pub fn tempo(&self) -> (i32, i32) {
        (self.bpm, self.tpb)
    }

    /// A row's length in seconds now.
    pub fn row_seconds(&self) -> f64 {
        f64::from(self.samples_per_tick + 1) / f64::from(RATE)
    }

    pub fn set_muted(&mut self, track: usize, muted: bool) {
        if let Some(m) = self.muted.get_mut(track) {
            *m = muted;
        }
    }

    pub fn set_gain(&mut self, track: usize, gain: f32) {
        if let Some(g) = self.gains.get_mut(track) {
            *g = gain;
        }
    }

    /// Each track's level now (0 to 1), for meters.
    pub fn levels(&self, out: &mut [f32]) {
        for (o, t) in out.iter_mut().zip(&self.tracks) {
            *o = t.level.min(1.0);
        }
    }

    /// Adds the next frames, at `rate` and `gain`, into `left` and `right`.
    pub fn render(&mut self, rate: u32, gain: f32, left: &mut [f32], right: &mut [f32]) {
        let song = self.song.clone();
        let step = f64::from(RATE) / f64::from(rate.max(1));
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            while self.frac >= 1.0 {
                self.frac -= 1.0;
                self.last = self.next;
                self.next = self.frame(&song);
            }
            let f = self.frac as f32;
            *l += (self.last.0 + (self.next.0 - self.last.0) * f) * gain;
            *r += (self.last.1 + (self.next.1 - self.last.1) * f) * gain;
            self.frac += step;
        }
    }

    /// Renders the replayer's own frames (44.1 kHz) into `left` and
    /// `right`, replacing what's there.
    pub fn render_native(&mut self, left: &mut [f32], right: &mut [f32]) {
        let song = self.song.clone();
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            (*l, *r) = self.frame(&song);
        }
    }

    /// How long the song plays once through (to where it comes round or
    /// stops), in seconds, at most `most`: its rows timed without its sound.
    pub fn length(song: &Arc<Song>, most: f64) -> f64 {
        let mut m = Machine::new(song.clone());
        m.timing_only = true;
        let s = song.clone();
        let limit = (most * f64::from(RATE)) as u64;
        let mut frames = 0u64;
        while frames < limit && !m.done {
            m.sp_player(&s, &mut (0.0, 0.0), &mut (0.0, 0.0), &mut (0.0, 0.0));
            frames += 1;
            if !m.playing {
                break;
            }
        }
        (frames as f64 / f64::from(RATE)).min(most)
    }

    fn calc_tempo(&mut self) {
        self.samples_per_tick = ((60 * RATE as i32) / self.bpm.max(1)) / self.tpb.max(1);
        self.samples_per_sub = (self.samples_per_tick / 6).max(1);
    }

    fn update_shuffle(&mut self) {
        let s = (self.samples_per_tick * self.shuffle) / 200;
        self.shufflestep = if self.shuffleswitch == 1 { -s } else { s };
    }

    fn active(&self, song: &Song, track: usize) -> bool {
        song.active.get(self.position).is_none_or(|a| a[track])
    }

    fn cell(&self, song: &Song, track: usize) -> Cell {
        let (p, rows) = song.pattern_at(self.position);
        if self.line >= rows {
            return Cell::default();
        }
        p.and_then(|p| p.cell(self.line, track)).copied().unwrap_or_default()
    }

    /// One frame of the mix.
    fn frame(&mut self, song: &Song) -> (f32, f32) {
        self.frames += 1;
        self.comp_pos += 1;
        if self.comp_pos > COMP_WINDOW - 1 {
            self.comp_pos = 0;
        }
        let mut mix = (0.0f32, 0.0f32);
        let mut chorus = (0.0f32, 0.0f32);
        let mut delay = (0.0f32, 0.0f32);
        self.sp_player(song, &mut mix, &mut chorus, &mut delay);
        let filter = (self.chorus_filter[0] != 4).then(|| self.coefs.get(self.chorus_filter[1], self.chorus_filter[2], self.chorus_filter[0]));
        let (cl, cr) = self.chorus.run(chorus, song.chorus_feedback, filter);
        mix.0 += cl;
        mix.1 += cr;
        if song.reverb {
            let (rl, rr) = self.reverb.run(delay, &song.reverb_decays, self.reverb_damp, song.reverb_feedback, self.reverb_cutoff, self.reverb_resonance);
            mix.0 += rl;
            mix.1 += rr;
        }
        // The mix's compressor isn't heard: the editor's replayer sets its
        // ratio to 0 as a song starts (only moving its slider then brings it in).
        let (mut l, mut r) = (mix.0 / 32767.0, mix.1 / 32767.0);
        l *= song.volume;
        r *= song.volume;
        if self.curr_mas_vol >= self.local_mas_vol {
            self.curr_mas_vol = (self.curr_mas_vol - 0.003).max(0.0);
        } else {
            self.curr_mas_vol = (self.curr_mas_vol + 0.003).min(1.0);
        }
        if self.curr_ramp_vol >= self.local_ramp_vol {
            self.curr_ramp_vol = (self.curr_ramp_vol - 0.003).max(0.0);
        } else {
            self.curr_ramp_vol = (self.curr_ramp_vol + 0.003).min(1.0);
        }
        // One level, then the other, as the replayer rounds them.
        let (l, r) = (l * self.curr_mas_vol * self.curr_ramp_vol, r * self.curr_mas_vol * self.curr_ramp_vol);
        (l.clamp(-1.0, 1.0), r.clamp(-1.0, 1.0))
    }

    /// The replayer's frame: the song moved on, then every track mixed.
    fn sp_player(&mut self, song: &Song, mix: &mut (f32, f32), chorus: &mut (f32, f32), delay: &mut (f32, f32)) {
        let nt = song.song_tracks.min(TRACKS);
        if self.playing {
            if self.pos_in_tick == 0 {
                self.row_start(song, nt);
            }
            if self.sub_counter == 0 {
                self.effects_ticks(song, nt);
            }
            self.sub_counter += 1;
            if self.sub_counter > self.samples_per_sub - 1 {
                self.sub_counter = 0;
                self.subicounter += 1;
            }
            self.pos_in_tick += 1;
            if self.pos_in_tick > self.samples_per_tick + self.shufflestep {
                self.shuffleswitch = -self.shuffleswitch;
                self.update_shuffle();
                self.sub_counter = 0;
                self.pos_in_tick = 0;
                self.next_row(song);
            }
        }
        if self.timing_only {
            return;
        }
        for c in 0..nt {
            self.mix_track(song, c, mix, chorus, delay);
        }
    }

    /// The song's next row: a pattern loop's, a break's, or the next.
    fn next_row(&mut self, song: &Song) {
        if self.loop_in {
            self.line = (self.line as i32 - self.loop_pos).max(0) as usize;
            self.loop_in = false;
        } else {
            let length = song.length.max(1);
            if self.patbreak > 127 {
                self.line += 1;
            } else {
                if self.pos_jump >= 0 {
                    if self.pos_jump as usize <= self.position {
                        self.done = true;
                    }
                    self.position = self.pos_jump as usize;
                    self.pos_jump = -1;
                } else {
                    self.position += 1;
                }
                self.line = self.patbreak as usize;
                if self.position >= length {
                    self.position = 0;
                    self.done = true;
                }
                if let Some(h) = self.history.get_mut(self.position) {
                    *h = [false; TRACKS];
                }
                self.loop_pos = 0;
                self.loop_counter = 0;
                self.loop_in = false;
            }
            if self.line >= song.pattern_at(self.position).1 {
                if self.pos_jump > 0 {
                    if self.pos_jump as usize <= self.position {
                        self.done = true;
                    }
                    self.position = self.pos_jump as usize;
                    self.pos_jump = -1;
                } else {
                    self.position += 1;
                }
                self.line = 0;
                if self.position >= length {
                    self.position = 0;
                    self.done = true;
                }
                if let Some(h) = self.history.get_mut(self.position) {
                    *h = [false; TRACKS];
                }
                self.loop_pos = 0;
                self.loop_counter = 0;
                self.loop_in = false;
            }
        }
    }

    /// A row's first frame: its tick-0 effects, the 303s, every track's
    /// notes.
    fn row_start(&mut self, song: &Song, nt: usize) {
        self.effects_tick0(song, nt);
        self.subicounter = 0;
        self.patbreak = 255;
        let mut fired = [false; 2];
        for ct in 0..nt {
            let cell = self.cell(song, ct);
            let st = &song.tracks[ct];
            let multi = usize::from(st.notes).clamp(1, POLYPHONY);
            let mut note_off = false;
            for i in 0..multi {
                self.pl_note[i] = cell.notes[i];
                if cell.notes[i] == NOTE_OFF {
                    note_off = true;
                }
            }
            for &(e, d) in cell.fx.iter().take(usize::from(st.effects).clamp(1, 4)) {
                self.live303(e, d);
                if e == 0x31 {
                    self.track303[0] = ct;
                    self.fire303(d, 0);
                    fired[0] = true;
                }
                if e == 0x32 {
                    self.track303[1] = ct;
                    self.fire303(d, 1);
                    fired[1] = true;
                }
            }
            if note_off && (!fired[0] || !fired[1]) {
                self.note_off_303(ct);
            }
        }
        self.go303();
        for ct in 0..nt {
            self.track_row(song, ct);
        }
    }

    fn track_row(&mut self, song: &Song, ct: usize) {
        let cell = self.cell(song, ct);
        let st = &song.tracks[ct];
        let multi = usize::from(st.notes).clamp(1, POLYPHONY);
        let nfx = usize::from(st.effects).clamp(1, 4);
        let active = self.active(song, ct);
        let (mut notes, mut samples) = ([NO_NOTE; POLYPHONY], [NO_INSTR; POLYPHONY]);
        for i in 0..multi {
            notes[i] = cell.notes[i];
            samples[i] = cell.instrs[i];
            self.pl_note[i] = notes[i];
            if samples[i] != NO_INSTR {
                self.tracks[ct].old_sample[i] = samples[i];
            }
        }
        let (vol, pan) = (cell.volume, cell.panning);
        let fx = &cell.fx[..nfx];
        let t = &mut self.tracks[ct];
        if fx.iter().any(|&(e, _)| vol <= 64 || e == 3) {
            t.tvol_mod = 1.0;
        }
        if vol <= 64 {
            t.tvol_mod *= f32::from(vol) * 0.015_625;
        }
        for &(e, d) in fx {
            if e == 3 {
                t.tvol_mod *= f32::from(d) * 0.003_906_2;
            }
        }
        if pan <= 128 {
            t.pan = f32::from(pan) * 0.007_812_5;
        }
        if active {
            for &(e, d) in fx {
                if e == 0x0D && usize::from(d) < MAX_ROWS {
                    self.patbreak = i32::from(d);
                }
            }
            for &(e, d) in fx {
                if e == 0x1F {
                    if self.patbreak >= MAX_ROWS as i32 {
                        self.patbreak = 0;
                    }
                    self.pos_jump = i32::from(d);
                }
            }
        }
        let mut offset = 0u32;
        let t = &mut self.tracks[ct];
        t.glide = false;
        for &(e, d) in fx {
            if e == 9 {
                offset = u32::from(d);
            } else if e == 5 {
                t.glide = true;
            }
        }
        let glide = t.glide;
        // A new note lets the note before go; its synth glides on (the
        // replayer's test of whether to let it go is always false).
        let mut glide_synth = [-1i32; POLYPHONY];
        for i in 0..multi {
            if notes[i] < NOTE_OFF && t.note_sub[i] != -1 {
                let j = t.reserved[i];
                if let Some(s) = usize::try_from(j).ok().and_then(|j| t.subs.get_mut(j)) {
                    if s.stage == SAMPLE {
                        s.stage = NOTEOFF;
                    }
                }
                glide_synth[i] = j;
            }
        }
        for i in 0..multi {
            if active && (notes[i] < NOTE_OFF || (notes[i] > NOTE_OFF && samples[i] != NO_INSTR)) {
                let mut free = glide_synth[i];
                if free == -1 {
                    free = i as i32;
                }
                let t = &mut self.tracks[ct];
                t.note_sub[i] = i as i32;
                t.reserved[i] = free;
                if vol > 64 && !fx.iter().any(|&(e, _)| e == 3) && samples[i] != NO_INSTR {
                    t.tvol_mod = 1.0;
                }
                self.schedule(song, ct, free as usize, i32::from(notes[i]), i32::from(samples[i]), offset, glide, i + 1);
            }
        }
        let t = &mut self.tracks[ct];
        for i in 0..multi {
            if notes[i] == NOTE_OFF && t.note_sub[i] != -1 {
                if let Some(s) = usize::try_from(t.reserved[i]).ok().and_then(|j| t.subs.get_mut(j)) {
                    if s.stage == SAMPLE {
                        s.stage = NOTEOFF;
                    }
                    s.synth.note_off();
                    s.stage = NOTEOFF;
                }
                t.reserved[i] = -1;
                t.note_sub[i] = -1;
            }
        }
        if active {
            self.pattern_loop(&cell, nfx);
        }
    }

    /// The pattern loop (06): 0 marks where, xx goes back that many times.
    fn pattern_loop(&mut self, cell: &Cell, nfx: usize) {
        for &(e, d) in &cell.fx[..nfx] {
            if e != 6 {
                continue;
            }
            if d == 0 {
                if self.loop_counter == 0 {
                    self.loop_pos = self.line as i32;
                }
            } else if self.loop_pos != -1 {
                if self.loop_counter == 0 {
                    self.loop_counter = i32::from(d);
                    self.loop_pos = self.line as i32 - self.loop_pos;
                    self.loop_in = true;
                } else {
                    self.loop_counter -= 1;
                    if self.loop_counter == 0 {
                        self.loop_pos = -1;
                        self.loop_in = false;
                    } else {
                        self.loop_in = true;
                    }
                }
            }
        }
    }

    /// A note made ready for a sub-channel, to start once what it's
    /// playing has faded.
    #[allow(clippy::too_many_arguments)]
    fn schedule(&mut self, song: &Song, ch: usize, sub: usize, inote: i32, sample: i32, offset: u32, glide: bool, midi_sub: usize) {
        if !self.active(song, ch) || sub >= POLYPHONY {
            return;
        }
        let multi = usize::from(song.tracks[ch].notes).clamp(1, POLYPHONY);
        let t = &mut self.tracks[ch];
        let mut sample = sample;
        if multi - 1 <= sub && sample == i32::from(NO_INSTR) {
            sample = i32::from(t.old_sample[midi_sub - 1]);
        }
        let ins = usize::try_from(sample).ok().and_then(|s| song.instruments.get(s));
        let s = &mut t.subs[sub];
        s.old_note = inote;
        s.sched = Schedule {
            inote,
            sample,
            vol: ins.map_or(0.0, |i| i.volume),
            vol_synth: ins.map_or(0.0, |i| f32::from(i.synth.volume) * 0.007_812_5),
            offset,
            glide,
            start_backward: false,
            age: self.frames,
        };
        s.cvol_dest = 10.0;
        if !glide {
            s.cut_stage = true;
        } else {
            s.glide_stage = true;
        }
    }

    /// The first free sub-channel (the replayer's choice when a note is
    /// retriggered).
    fn free_sub(&self, ch: usize, polyphony: usize) -> usize {
        let subs = &self.tracks[ch].subs;
        for (i, s) in subs.iter().enumerate().take(polyphony) {
            if s.stage == NOSAMPLE && s.stage2 == NOSAMPLE && s.stage3 == NOSAMPLE && !s.cut_stage {
                return i;
            }
        }
        for (i, s) in subs.iter().enumerate().take(polyphony) {
            if !s.cut_stage && s.stage == NOTEOFF && s.stage2 == NOTEOFF && s.stage3 == NOTEOFF {
                return i;
            }
        }
        // The oldest (as the replayer finds it: the last older than the first).
        let mut oldest = 0;
        for i in 1..polyphony {
            if subs[i].sched.age < subs[0].sched.age {
                oldest = i;
            }
        }
        oldest
    }

    /// A scheduled note starts on its sub-channel.
    fn play_instrument(&mut self, song: &Song, ch: usize, sub: usize) {
        if !self.active(song, ch) {
            return;
        }
        let spt = self.samples_per_tick;
        let t = &mut self.tracks[ch];
        let s = &mut t.subs[sub];
        let sched = s.sched;
        let mut inote = sched.inote;
        let mut sample = sched.sample;
        let mut vol = sched.vol;
        let mut vol_synth = sched.vol_synth;
        let mut glide = sched.glide;
        let offset = sched.offset;
        let (mut no_retrig_adsr, mut no_retrig_note) = (false, false);
        if inote > 120 {
            inote = i32::from(s.channel_note as u8);
            no_retrig_note = true;
        }
        if sample == 255 {
            sample = i32::from(s.channel_sample as u8);
            vol = s.tvol;
            vol_synth = s.tvol_synth;
            if glide {
                no_retrig_adsr = true;
            }
        }
        let prg_of = |x: i32| usize::try_from(x).ok().and_then(|x| song.instruments.get(x)).map_or(0, |i| i.synth_prg);
        let prg = prg_of(sample);
        let assoc: i32 = match prg {
            0 | 1 => sample,
            p => i32::from(p) - 2,
        };
        let Some(ains) = usize::try_from(assoc).ok().filter(|&a| a != 255).and_then(|a| song.instruments.get(a)) else { return };
        let assoc = assoc as usize;
        let mut split = 0;
        for (i, sp) in ains.splits.iter().enumerate() {
            if let Some(sp) = sp {
                if inote >= i32::from(sp.base_note) {
                    split = i;
                }
            }
        }
        let sp = ains.splits[split].as_ref();
        if !no_retrig_note {
            if s.synth.osc1_wave() == WAV && !glide {
                s.pos = Pos::default();
                s.osc[0] = Pos::default();
                s.osc[2] = Pos::default();
            }
            if s.synth.osc2_wave() == WAV && !glide {
                s.pos = Pos::default();
                s.osc[1] = Pos::default();
            }
        }
        if i64::from(sample) != i64::from(s.channel_sample) {
            glide = false;
        }
        let note2 = inote - i32::from(BASE_NOTE);
        let base = sp.map_or(BASE_NOTE, |x| x.base_note);
        let fine = sp.map_or(0, |x| x.finetune);
        let note = inote as f32 - f32::from(base) + f32::from(fine) * 0.007_812_5;
        let (loop_type, loop_start, loop_end, length) = sp.map_or((0, 0, 0, 0u32), |x| (x.loop_type, x.loop_start, x.loop_end, x.len() as u32));
        if !no_retrig_adsr && !no_retrig_note {
            if !glide {
                s.synth.silence_waves();
                if s.synth.env[0].stage == 0 {
                    s.synth_used = 0;
                } else if prg != 0 {
                    s.synth.env[0].stage = RELEASE;
                    s.synth.env[1].stage = RELEASE;
                }
            }
            if prg != 0 {
                let params = usize::try_from(sample).ok().and_then(|x| song.instruments.get(x)).map(|i| i.synth).unwrap_or_default();
                s.synth.set_params(&params, spt);
                let len = if loop_type > LOOP_NONE { loop_end } else { length.wrapping_sub(2) };
                s.synth.note_on(note2, loop_type > LOOP_NONE, len, loop_end.wrapping_sub(loop_start), note, glide);
                s.synth_used = ains.synth_prg;
            }
        }
        s.channels = 0;
        s.tvol = vol;
        s.cvol_dest = 10.0;
        s.tvol_synth = vol_synth;
        s.cvol_synth_dest = 10.0;
        let spread = pow2(note2 as f32 / 12.0);
        s.vstep_arp = spread;
        s.arp_base = note2 as f32;
        s.vstep_vib = spread;
        s.vib_base = note2 as f32;
        if !no_retrig_note {
            if glide {
                s.step = spread;
            } else {
                s.vstep = spread;
                s.step = spread;
            }
        }
        s.stage = NOSAMPLE;
        if prg == 0 {
            s.stage2 = NOSAMPLE;
            s.stage3 = NOSAMPLE;
        } else {
            s.stage2 = STOCK;
            s.stage3 = STOCK;
        }
        s.channel_sample = sample;
        s.channel_note = inote;
        if let Some(x) = sp {
            if prg != 0 {
                if s.synth.osc1_wave() != WAV && s.synth.osc2_wave() != WAV {
                    s.stage = SAMPLE;
                }
            } else {
                s.stage = SAMPLE;
                s.stage2 = NOSAMPLE;
                s.stage3 = NOSAMPLE;
            }
            s.arp_base = note;
            s.vib_base = note;
            if ains.beat_sync {
                let spread = x.len() as f32 / (f32::from(ains.beat_lines) * spt as f32);
                s.vstep_arp = spread;
                s.vstep_vib = spread;
                if !no_retrig_note {
                    s.vstep = spread;
                    s.step = spread;
                }
            } else {
                let spread = pow2(note / 12.0);
                s.vstep_arp = spread;
                s.vstep_vib = spread;
                if !no_retrig_note {
                    if glide {
                        s.step = spread;
                    } else {
                        s.vstep = spread;
                        s.step = spread;
                    }
                }
            }
            s.synth_was = prg;
            s.loop_start = x.loop_start;
            s.loop_end = x.loop_end;
            s.length = x.len() as u32;
            if !glide && !no_retrig_note {
                s.pos = Pos::at((offset << 8) as i32);
            }
            s.loop_len = s.loop_end.wrapping_sub(s.loop_start);
            s.ampli = x.amplify;
            s.loop_type = x.loop_type;
            if !no_retrig_note || s.loop_type != LOOP_PINGPONG {
                s.backwards = false;
            }
            s.channels = if x.right.is_some() { 2 } else { 1 };
            t.decay = x.decay;
            s.wave = Some((assoc, split));
            if !glide {
                t.ramper = 0.0;
            }
        } else {
            s.wave = None;
            s.ampli = 1.0;
            if !glide {
                t.ramper = 0.0;
                if !no_retrig_note {
                    s.pos = Pos::at((offset << 8) as i32);
                }
            }
        }
        if length != 0 && s.pos.int >= length as i32 {
            if loop_type != 0 {
                s.pos = Pos::at(s.loop_end as i32);
            } else {
                s.stage = NOSAMPLE;
                s.stage2 = NOSAMPLE;
                s.stage3 = NOSAMPLE;
            }
        }
        if !no_retrig_adsr && !no_retrig_note && !glide {
            s.osc = [s.pos; 3];
        }
        t.segue_pos = 0;
        t.segue_volume = 1.0;
        t.new_instrument = true;
        if !glide {
            s.synth.backwards = [false; 3];
            s.backwards = false;
        }
        if sched.start_backward {
            if !glide {
                s.synth.backwards = [true; 3];
                s.backwards = true;
            }
            let max = s.length.min(s.loop_end) as i32;
            s.pos = Pos::at(max);
            if s.synth.osc1_wave() == WAV {
                s.osc[0] = Pos::at(max);
                s.osc[2] = Pos::at(max);
            }
            if s.synth.osc2_wave() == WAV {
                s.osc[1] = Pos::at(max);
            }
        }
    }

    /// The row's effects that work on its first frame, before its notes.
    fn effects_tick0(&mut self, song: &Song, nt: usize) {
        for tr in 0..nt {
            let cell = self.cell(song, tr);
            let nfx = usize::from(song.tracks[tr].effects).clamp(1, 4);
            let poly = usize::from(song.tracks[tr].polyphony).clamp(1, POLYPHONY);
            for &(e, d) in &cell.fx[..nfx] {
                let t = &mut self.tracks[tr];
                let v = f32::from(d) / 255.0;
                match e {
                    0x1B => {
                        t.arpeggio = d;
                        if d == 0 {
                            for s in t.subs.iter_mut().take(poly) {
                                s.vstep = s.vstep_arp;
                            }
                        }
                    }
                    0x1D => {
                        t.vibcounter = 0;
                        t.vibrato = d;
                        if d == 0 {
                            for s in t.subs.iter_mut().take(poly) {
                                s.vstep = s.vstep_vib;
                            }
                        }
                    }
                    0x1E => {
                        t.reverse = true;
                        t.reverse_dat = d;
                    }
                    0x25 => {
                        self.shuffle = (f32::from(d) * 0.392_16) as i32;
                        self.update_shuffle();
                    }
                    0x26 => self.reverb_cutoff = (v * 0.99).max(0.02),
                    0x27 => self.reverb_resonance = (v * 0.99).max(0.02),
                    0x29 => t.compress = d & 1 != 0,
                    0x2A => {
                        let ratio = t.comp_ratio_raw;
                        set_comp(&mut t.comp_threshold_raw, &mut t.comp_ratio_raw, &mut t.comp_threshold, &mut t.comp_ratio, v * 100.0, ratio);
                    }
                    0x2B => {
                        // The replayer hands the scaled threshold back as a raw one.
                        let threshold = t.comp_threshold;
                        set_comp(&mut t.comp_threshold_raw, &mut t.comp_ratio_raw, &mut t.comp_threshold, &mut t.comp_ratio, threshold, v * 100.0);
                    }
                    0x2C => self.reverb_damp = v.max(0.02),
                    0x16 => {
                        let c = v * 359.0;
                        t.carrier = [c; 3];
                    }
                    0x43 => t.lfo_rate = (v * 0.007_812_5).max(0.0001),
                    0x44 => t.lfo_scale = v * 15.0 + 1.0,
                    0x45 => t.lfo_filter = v * 128.0,
                    0x46 => t.lfo_volume = v * 128.0,
                    0x47 => t.lfo_panning = v * 128.0,
                    0xF0 => {
                        if d >= 20 {
                            self.bpm = i32::from(d);
                            self.calc_tempo();
                        }
                    }
                    0x0F => {
                        if d == 0 {
                            self.playing = false;
                            self.done = true;
                        } else {
                            self.tpb = i32::from(d).clamp(1, 32);
                            self.calc_tempo();
                            self.update_shuffle();
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// The effects worked at each sixth of a row.
    fn effects_ticks(&mut self, song: &Song, nt: usize) {
        for tr in 0..nt {
            let cell = self.cell(song, tr);
            let st = &song.tracks[tr];
            let multi = usize::from(st.notes).clamp(1, POLYPHONY);
            let poly = usize::from(st.polyphony).clamp(1, POLYPHONY);
            let nfx = usize::from(st.effects).clamp(1, 4);
            let mut notes = [NO_NOTE; POLYPHONY];
            let mut samples = [NO_INSTR; POLYPHONY];
            notes[..multi].copy_from_slice(&cell.notes[..multi]);
            samples[..multi].copy_from_slice(&cell.instrs[..multi]);
            let sub = self.subicounter;
            {
                let t = &mut self.tracks[tr];
                match t.fade_mode {
                    1 => {
                        t.tvol_mod += t.fade_coef;
                        if t.tvol_mod > 1.0 {
                            t.tvol_mod = 1.0;
                            t.fade_mode = 0;
                        }
                    }
                    2 => {
                        t.tvol_mod -= t.fade_coef;
                        if t.tvol_mod < 0.0 {
                            t.tvol_mod = 0.0;
                            t.fade_mode = 0;
                        }
                    }
                    _ => {}
                }
                // F0 to FF in the volume column: the note cut that sixth.
                if cell.volume & 0xF0 == 0xF0 && sub == i32::from(cell.volume & 0x0F) {
                    let ftype = t.ftype;
                    for s in t.subs.iter_mut().take(poly) {
                        if s.stage == SAMPLE {
                            if ftype == 4 {
                                s.stage = NOTEOFF;
                            } else {
                                t.tvol_mod = 0.001;
                            }
                        }
                        s.synth.note_off();
                    }
                }
            }
            for k in 0..nfx {
                let (e, d) = cell.fx[k];
                let v = f32::from(d);
                if self.pos_in_tick == 0 {
                    let t = &mut self.tracks[tr];
                    match e {
                        0x20 => t.tvol_mod = (t.tvol_mod + v * 0.003_906_2).min(1.0),
                        0x21 => t.tvol_mod = (t.tvol_mod - v * 0.003_906_2).max(0.0),
                        0x22 => {
                            for s in t.subs.iter_mut().take(poly) {
                                if s.vstep < 5.660_29 {
                                    s.vstep += v / 2048.0;
                                }
                            }
                        }
                        0x23 => {
                            for s in t.subs.iter_mut().take(poly) {
                                if s.vstep > 0.005_862_36 {
                                    s.vstep -= v / 2048.0;
                                }
                            }
                        }
                        0x10 => t.chorus_send = v / 255.0,
                        0x11 => t.reverb_send = v / 255.0,
                        0x12 => t.dist_threshold = v * 128.0,
                        0x13 => t.dist_clamp = v * 128.0,
                        0x08 => t.tcut = v / 2.0,
                        0x14 => t.frez = i32::from(d / 2),
                        0x24 => t.flanger_on = d & 1 != 0,
                        0x28 => t.lfo_on = d & 1 != 0,
                        0x15 => {
                            if d <= 23 {
                                t.ftype = i32::from(d);
                            }
                        }
                        0x2D => {
                            if d <= 4 {
                                self.chorus_filter[0] = i32::from(d);
                            }
                        }
                        0x2E => self.chorus_filter[1] = i32::from(d / 2),
                        0x2F => self.chorus_filter[2] = i32::from(d / 2),
                        0x30 => t.denoise = d.min(4),
                        _ => {}
                    }
                }
                let t = &mut self.tracks[tr];
                match e {
                    0x01 => {
                        for s in t.subs.iter_mut().take(poly) {
                            if s.vstep < 5.660_29 {
                                s.vstep += v / 2048.0;
                            }
                        }
                    }
                    0x02 => {
                        for s in t.subs.iter_mut().take(poly) {
                            if s.vstep > 0.005_862_36 {
                                s.vstep -= v / 2048.0;
                            }
                        }
                    }
                    0x04 => {
                        if sub >= i32::from(d) {
                            t.tvol_mod = 0.0;
                        }
                    }
                    0x05 => {
                        if d != 0 {
                            t.glide_step = v / 2048.0;
                        }
                        let gs = t.glide_step;
                        for s in t.subs.iter_mut().take(poly) {
                            if s.vstep < s.step {
                                s.vstep = (s.vstep + gs).min(s.step);
                            } else if s.vstep > s.step {
                                s.vstep = (s.vstep - gs).max(s.step);
                            }
                        }
                    }
                    _ => {}
                }
                match e {
                    0x0A => {
                        if sub == 0 {
                            self.rand = self.rand.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                            let r = (self.rand >> 16) & 0xFF;
                            let t = &mut self.tracks[tr];
                            t.tcut = (r as f32 * (v / 255.0)).clamp(1.0, 127.0);
                        }
                    }
                    0x0B => {
                        if sub == 0 {
                            let t = &mut self.tracks[tr];
                            t.tcut = (t.tcut + v).min(127.0);
                        }
                    }
                    0x0C => {
                        if sub == 0 {
                            let t = &mut self.tracks[tr];
                            t.tcut = (t.tcut - v).max(1.0);
                        }
                    }
                    0x0E => {
                        {
                            let t = &mut self.tracks[tr];
                            for i in 0..poly {
                                if notes[i] == NO_NOTE {
                                    notes[i] = t.subs[i].old_note.clamp(0, 255) as u8;
                                }
                                t.subs[i].old_note = i32::from(notes[i]);
                            }
                        }
                        if d > 0 && sub % i32::from(d) == 0 {
                            let t = &mut self.tracks[tr];
                            for i in 0..multi {
                                if self.pl_note[i] < NOTE_OFF && t.note_sub[i] != -1 {
                                    if let Some(s) = usize::try_from(t.reserved[i]).ok().and_then(|j| t.subs.get_mut(j)) {
                                        if s.stage == SAMPLE {
                                            s.stage = NOTEOFF;
                                        }
                                        s.synth.note_off();
                                    }
                                }
                            }
                            for i in 0..multi {
                                let free = self.free_sub(tr, poly);
                                let t = &mut self.tracks[tr];
                                t.note_sub[i] = i as i32;
                                t.reserved[i] = free as i32;
                                self.schedule(song, tr, free, i32::from(notes[i]), i32::from(samples[i]), 0, false, i + 1);
                            }
                        }
                    }
                    0x17 => {
                        let t = &mut self.tracks[tr];
                        if d > 0 {
                            t.fade_coef = 0.166_666_7 / v;
                            t.fade_mode = 1;
                        } else {
                            t.fade_mode = 0;
                        }
                    }
                    0x18 => {
                        let t = &mut self.tracks[tr];
                        if d > 0 {
                            t.fade_coef = 0.166_666_7 / v;
                        } else {
                            t.fade_mode = 0;
                        }
                    }
                    0x19 => {
                        let t = &mut self.tracks[tr];
                        t.tvol_mod = (t.tvol_mod + v * 0.003_906_2).min(1.0);
                    }
                    0x1A => {
                        let t = &mut self.tracks[tr];
                        t.tvol_mod = (t.tvol_mod - v * 0.003_906_2).max(0.0);
                    }
                    0x1C => self.local_mas_vol = v / 255.0,
                    _ => {}
                }
            }
            let t = &mut self.tracks[tr];
            if t.arpeggio != 0 {
                let arp = t.arpeggio;
                for s in t.subs.iter_mut().take(poly) {
                    s.vstep = match sub % 3 {
                        0 => s.vstep_arp,
                        1 => pow2((s.arp_base + f32::from(arp >> 4)) / 12.0),
                        _ => pow2((s.arp_base + f32::from(arp & 0x0F)) / 12.0),
                    };
                }
            }
            if t.vibrato != 0 {
                let speed = f32::from(t.vibrato >> 4) * t.vibcounter as f32;
                let speed = ((speed * 0.000_45) / (std::f32::consts::PI * 2.0)).sin();
                let amp = f32::from(t.vibrato & 0x0F) * 0.09;
                let vib = speed * amp;
                for s in t.subs.iter_mut().take(poly) {
                    s.vstep = pow2((s.vib_base + vib) / 12.0);
                }
            }
            if t.reverse {
                t.reverse = false;
                let back = t.reverse_dat != 0;
                for s in t.subs.iter_mut().take(poly) {
                    s.sched.start_backward = back;
                    if s.stage != NOSAMPLE || s.cut_stage {
                        s.backwards = back;
                    }
                    if s.stage2 != NOSAMPLE || s.cut_stage {
                        s.synth.backwards[0] = back;
                        s.synth.backwards[2] = back;
                    }
                    if s.stage3 != NOSAMPLE || s.cut_stage {
                        s.synth.backwards[1] = back;
                    }
                }
            }
        }
    }

    fn live303(&mut self, e: u8, d: u8) {
        let half = d / 2;
        match e {
            0x33 => self.tb[0].cutoff = half,
            0x34 => self.tb[1].cutoff = half,
            0x35 => self.tb[0].resonance = half,
            0x36 => self.tb[1].resonance = half,
            0x37 => self.tb[0].envmod = half,
            0x38 => self.tb[1].envmod = half,
            0x39 => self.tb[0].decay = half,
            0x3A => self.tb[1].decay = half,
            0x3B => self.tb[0].accent = half,
            0x3C => self.tb[1].accent = half,
            0x3D => self.tb[0].tune = half,
            0x3E => self.tb[1].tune = half,
            0x3F | 0x40 => {
                let u = usize::from(e - 0x3F);
                let d = d.clamp(1, 16);
                self.tb[u].scale = d;
                self.gears[u].multiple = i32::from(d);
            }
            0x41 => self.gears[0].target_real_volume = f32::from(d) / 255.0,
            0x42 => self.gears[1].target_real_volume = f32::from(d) / 255.0,
            _ => {}
        }
    }

    fn fire303(&mut self, number: u8, unit: usize) {
        let g = &mut self.gears[unit];
        g.line = 0;
        g.note_off = false;
        let pattern = match number {
            0x00 => Some(usize::from(self.tb[unit].selected)),
            0xA1..=0xA8 => Some(usize::from(number - 0xA1)),
            0xB1..=0xB8 => Some(usize::from(number - 0xB1) + 8),
            0xC1..=0xC8 => Some(usize::from(number - 0xC1) + 16),
            0xD1..=0xD8 => Some(usize::from(number - 0xD1) + 24),
            _ => None,
        };
        match pattern {
            Some(p) => {
                g.pattern = p.min(31);
                g.ramp_volume = 1.0;
                g.target_real_volume = 1.0;
                g.multiple = i32::from(self.tb[unit].scale);
            }
            None => {
                g.line = 255;
                g.ramp_volume = 0.0;
                g.target_real_volume = 0.0;
            }
        }
    }

    fn note_off_303(&mut self, track: usize) {
        for u in 0..2 {
            if self.track303[u] == track {
                self.gears[u].ramp_volume = 0.0;
                self.gears[u].note_off = true;
            }
        }
    }

    fn go303(&mut self) {
        let spt = self.samples_per_tick;
        for u in 0..2 {
            let tb = &self.tb[u];
            let g = &mut self.gears[u];
            if g.pattern == 255 {
                continue;
            }
            g.multiple += 1;
            if g.multiple >= i32::from(tb.scale) {
                g.multiple = 0;
                let (p, l) = (g.pattern.min(31), g.line.min(15));
                if !g.note_off {
                    g.note_on(i32::from(tb.tones[p][l]), tb, spt);
                }
                // The step is a byte: past 255 it comes back to 0.
                g.line = (g.line + 1) & 0xFF;
                if g.line == usize::from(tb.lengths[g.pattern.min(31)]) {
                    g.line = 0;
                }
            }
        }
    }

    /// A track's frame: its sub-channels and 303s through its effects
    /// into the mix and the sends.
    fn mix_track(&mut self, song: &Song, c: usize, mix: &mut (f32, f32), chorus: &mut (f32, f32), delay: &mut (f32, f32)) {
        let poly = usize::from(song.tracks[c].polyphony).clamp(1, POLYPHONY);
        let active = self.active(song, c);
        self.tracks[c].vibcounter += 1;
        let mut grown = false;
        let mut gotsome = false;
        let (mut al, mut ar) = (0.0f32, 0.0f32);
        for i in 0..poly {
            let (mut cl, mut cr) = (0.0f32, 0.0f32);
            {
                let s = &mut self.tracks[c].subs[i];
                if s.cut_stage {
                    if s.cvol <= 0.0 && s.cvol_synth <= 0.0 {
                        s.cut_stage = false;
                        self.play_instrument(song, c, i);
                    }
                } else if s.glide_stage {
                    s.glide_stage = false;
                    self.play_instrument(song, c, i);
                }
            }
            let tvol_mod = self.tracks[c].tvol_mod;
            let s = &mut self.tracks[c].subs[i];
            let split = s.wave.and_then(|(ins, sp)| song.instruments.get(ins).and_then(|x| x.splits.get(sp)).and_then(|x| x.as_ref()));
            // The sample.
            if s.stage == SAMPLE || s.stage == NOTEOFF || s.cut_stage {
                let bypass = s.synth_was == 0 || !s.synth.uses_sample();
                if bypass {
                    if s.stage == NOTEOFF && s.cvol <= 0.02 {
                        s.stage = NOSAMPLE;
                    }
                    if s.cut_stage {
                        if s.cvol > 0.0 {
                            if s.cvol_dest != 0.0 {
                                s.cvol_ramp = s.cvol / 48.0;
                                s.cvol_dest = 0.0;
                            }
                            s.cvol -= s.cvol_ramp;
                            if s.cvol <= 0.0 {
                                s.cvol = 0.0;
                                s.cvol_dest = 10.0;
                            }
                        }
                    } else {
                        if s.stage == NOTEOFF {
                            s.tvol = 0.0;
                            s.cvol_dest = 10.0;
                        }
                        let dest = s.tvol * tvol_mod;
                        ramp(&mut s.cvol, &mut s.cvol_ramp, &mut s.cvol_dest, dest, true);
                        if s.stage == NOSAMPLE && s.cvol < 0.02 {
                            s.cvol = 0.0;
                            s.pos = Pos::default();
                        }
                    }
                    gotsome = true;
                    let at = bounded(s.pos.int, s.loop_type, s.backwards, s.length, s.loop_end, s.loop_start);
                    if let Some(sp) = split {
                        cl = f32::from(sp.left.get(at).copied().unwrap_or(0)) * s.cvol * s.ampli;
                        if s.channels == 2 {
                            grown = true;
                            cr = f32::from(sp.right.as_ref().and_then(|r| r.get(at)).copied().unwrap_or(0)) * s.cvol * s.ampli;
                        }
                    }
                    if s.backwards {
                        if s.pos.int > 0 {
                            s.pos.flt -= s.vstep;
                            s.pos.int = s.pos.flt as i32;
                        }
                    } else {
                        s.pos.flt += s.vstep;
                        s.pos.int = s.pos.flt as i32;
                    }
                    match s.loop_type {
                        LOOP_FORWARD => {
                            if !s.backwards {
                                if s.pos.int >= s.loop_end as i32 {
                                    s.pos = Pos::at(s.loop_start as i32);
                                }
                            } else if s.pos.int <= s.loop_start as i32 {
                                s.pos = Pos::at(s.loop_end as i32);
                            }
                        }
                        LOOP_PINGPONG => {
                            if !s.backwards {
                                if s.pos.int >= s.loop_end as i32 {
                                    s.pos = Pos::at(s.loop_end as i32);
                                    s.backwards = true;
                                }
                            } else if s.pos.int <= s.loop_start as i32 {
                                s.backwards = false;
                                s.pos = Pos::at(s.loop_start as i32);
                            }
                        }
                        _ => {
                            if !s.backwards {
                                if s.pos.int >= s.length as i32 {
                                    s.pos = Pos::at(s.length as i32);
                                    s.stage = NOSAMPLE;
                                }
                            } else if s.pos.int <= 0 {
                                s.pos = Pos::default();
                                s.stage = NOSAMPLE;
                            }
                        }
                    }
                }
            }
            // The synth.
            if s.synth.env[0].stage != 0 || s.synth.env[1].stage != 0 || (s.cut_stage && s.synth_used != 0) {
                if s.synth.env[0].stage == RELEASE && s.synth.env[1].stage == RELEASE && s.cvol_synth <= 0.02 {
                    s.synth.env[0].stage = 0;
                    s.synth.env[1].stage = 0;
                    s.stage = NOSAMPLE;
                    s.stage2 = NOSAMPLE;
                    s.stage3 = NOSAMPLE;
                }
                if s.cut_stage {
                    if s.cvol_synth > 0.0 {
                        if s.cvol_synth_dest != 0.0 {
                            s.cvol_synth_ramp = s.cvol_synth / 48.0;
                            s.cvol_synth_dest = 0.0;
                        }
                        s.cvol_synth = (s.cvol_synth - s.cvol_synth_ramp).max(0.0);
                    }
                } else {
                    let dest = s.tvol_synth * tvol_mod;
                    ramp(&mut s.cvol_synth, &mut s.cvol_synth_ramp, &mut s.cvol_synth_dest, dest, false);
                }
                let looping = s.loop_type > LOOP_NONE;
                let src = Source {
                    left: split.map(|x| x.left.as_slice()),
                    right: split.and_then(|x| x.right.as_deref().map(|r| r.as_slice())),
                    stereo: s.channels == 2,
                    loop_type: s.loop_type,
                    length: if looping { s.loop_end } else { s.length },
                    loop_sub: if looping { s.loop_len } else { 0 },
                };
                let mut playing = Playing { osc1: &mut s.stage2, osc2: &mut s.stage3 };
                cl += s.synth.sample(&self.t, &src, &mut cr, s.cvol_synth, &mut playing, &mut s.osc, s.vstep);
                if s.synth.uses_sample() && s.channels == 2 {
                    grown = true;
                }
                gotsome = true;
            }
            if gotsome {
                al += cl;
                if grown {
                    ar += cr;
                }
            }
        }
        let spt = self.samples_per_tick;
        for u in 0..2 {
            if self.track303[u] == c && active {
                let x = self.gears[u].sample(&self.tb[u], spt);
                al += x;
                if grown {
                    ar += x;
                }
                gotsome = true;
            }
        }
        // A track turned off for this position lets its notes go (once).
        let once = self.history.get(self.position).is_some_and(|h| h[c]);
        if !active && !once {
            if let Some(h) = self.history.get_mut(self.position) {
                h[c] = true;
            }
            for s in self.tracks[c].subs.iter_mut().take(poly) {
                if s.stage == SAMPLE {
                    s.stage = NOTEOFF;
                    s.cvol_dest = 10.0;
                }
                s.synth.note_off();
            }
            self.note_off_303(c);
        }
        let t = &mut self.tracks[c];
        // A new note's first frames cross-faded from the last's.
        if t.new_instrument && poly == 1 {
            al = al * (1.0 - t.segue_volume) + t.segue[0] * t.segue_volume;
            if grown {
                ar = ar * (1.0 - t.segue_volume) + t.segue[1] * t.segue_volume;
            }
            t.segue_pos += 1;
            t.segue_volume -= 1.0 / 127.0;
            if t.segue_pos >= 128 {
                t.new_instrument = false;
            }
        } else {
            t.segue[0] = al;
            if grown {
                t.segue[1] = ar;
            }
        }
        let st = &song.tracks[c];
        let muted = self.muted[c];
        let (mut realcut, mut gco) = (0.0f32, 0);
        if t.ftype != 4 {
            let dfi = t.tcut - t.ccut;
            if !(-1.0..=1.0).contains(&dfi) {
                t.ccut += dfi * t.icut;
            }
            realcut = lfo_filter(&self.t, t, t.ccut - t.ramper);
            t.ramper += t.decay * realcut * 0.015_625;
            gco = realcut as i32;
        }
        if gotsome {
            if t.ftype != 4 {
                t.filter.run(&mut self.coefs, t.ftype, realcut, gco, t.frez, &mut al, &mut ar, &mut grown, &self.t.degrees);
            }
            if st.distortion {
                if al > t.dist_threshold {
                    al = t.dist_clamp;
                } else if al < -t.dist_threshold {
                    al = -t.dist_clamp;
                }
                if grown {
                    if ar > t.dist_threshold {
                        ar = t.dist_clamp;
                    } else if ar < -t.dist_threshold {
                        ar = -t.dist_clamp;
                    }
                }
            }
            if !grown {
                ar = al;
            }
            if muted {
                al = 0.0;
                ar = 0.0;
            }
        }
        if t.flanger_on {
            let f = st.flanger;
            t.flanger.run(&mut al, &mut ar, f.amount, f.feedback, f.dephase, f.ampl, f.rate, f.delay, &self.t.degrees);
        }
        if muted {
            al = 0.0;
            ar = 0.0;
        }
        if t.compress {
            let (xl, xr) = (al / 32767.0, ar / 32767.0);
            al = xl * t.comp[0].gain(xl, self.comp_pos, t.comp_threshold, t.comp_ratio) * 32767.0;
            ar = xr * t.comp[1].gain(xr, self.comp_pos, t.comp_threshold, t.comp_ratio) * 32767.0;
        }
        if st.eq != [1.0; 3] {
            al = t.eq.run(st.eq, al, 0);
            ar = t.eq.run(st.eq, ar, 1);
        }
        if st.surround {
            ar = -ar;
        }
        // Panning, eased to where it's going.
        let mut pan = t.pan;
        let mut changed = false;
        if t.old_pan != t.pan {
            t.old_pan = t.pan;
            changed = true;
        }
        if t.lfo_on && t.lfo_panning != 0.0 {
            pan = ((pan - 0.5) * 2.0 * (self.t.degrees[(t.carrier[2] as usize).min(359)] * t.lfo_panning / 128.0) * 0.5 + 0.5).clamp(0.0, 1.0);
            t.carrier[2] += t.lfo_rate * t.lfo_scale;
            if t.carrier[2] >= 360.0 {
                t.carrier[2] -= 360.0;
            }
            changed = true;
        }
        if changed {
            let (l, r) = pan_gains(&self.t, pan);
            t.old_lvol = l;
            t.old_rvol = r;
        }
        al *= t.lvol;
        ar *= t.rvol;
        ease(&mut t.lvol, t.old_lvol);
        ease(&mut t.rvol, t.old_rvol);
        // The volume LFO, read (and moved on) once a side.
        al *= lfo_volume(&self.t, t);
        ar *= lfo_volume(&self.t, t);
        al *= st.volume * self.gains[c];
        ar *= st.volume * self.gains[c];
        if t.denoise != 0 {
            const SIZES: [usize; 5] = [0, 4, 8, 16, 32];
            let size = SIZES[usize::from(t.denoise)];
            let coeff = 1.0 / size as f32;
            let mask = size - 1;
            let (ol, or) = (al, ar);
            let (mut sl, mut sr) = (0.0, 0.0);
            for k in 0..size {
                let p = (t.history_pos.wrapping_sub(k)) & mask;
                sl += t.history[0][p];
                sr += t.history[1][p];
            }
            al = sl;
            ar = sr;
            let p = t.history_pos & mask;
            t.history[0][p] = ol * coeff;
            t.history[1][p] = or * coeff;
            t.history_pos = (t.history_pos + 1) & mask;
        }
        t.level = t.level * 0.999 + (al.abs() + ar.abs()) / 65534.0 * 0.001 * 8.0;
        mix.0 += al;
        mix.1 += ar;
        if t.reverb_send > 0.008 {
            delay.0 += al * t.reverb_send;
            delay.1 += ar * t.reverb_send;
        }
        if t.chorus_send > 0.008 {
            chorus.0 += al * t.chorus_send;
            chorus.1 += ar * t.chorus_send;
        }
    }
}

/// A compressor's threshold and ratio set (0.01 to 100 each), kept raw
/// and scaled as the compressor works with them.
fn set_comp(raw_t: &mut f32, raw_r: &mut f32, t: &mut f32, r: &mut f32, threshold: f32, ratio: f32) {
    let threshold = threshold.clamp(0.01, 100.0);
    let ratio = ratio.clamp(0.01, 100.0);
    *raw_t = threshold;
    *raw_r = ratio;
    *t = threshold * 0.001;
    *r = ratio * 0.01;
}

/// A volume ramped toward `dest` over 48 frames; `snap` resets the
/// ramp's target once it's there (the sample's ramp does, the synth's
/// doesn't).
fn ramp(cvol: &mut f32, step: &mut f32, target: &mut f32, dest: f32, snap: bool) {
    if *cvol == dest {
        return;
    }
    if *cvol > dest {
        if *target != dest {
            *step = (*cvol - dest) / 48.0;
            *target = dest;
        }
        *cvol -= *step;
        if *cvol <= dest && snap || *cvol < dest {
            *cvol = dest;
            if snap {
                *target = 10.0;
            }
        }
    } else {
        if *target != dest {
            *step = (dest - *cvol) / 48.0;
            *target = dest;
        }
        *cvol += *step;
        if *cvol >= dest && snap || *cvol > dest {
            *cvol = dest;
            if snap {
                *target = 10.0;
            }
        }
    }
    *cvol = cvol.clamp(0.0, 1.0);
}

fn ease(v: &mut f32, to: f32) {
    if *v > to {
        *v = (*v - 0.001).max(to);
    } else if *v < to {
        *v = (*v + 0.001).min(to);
    }
}

fn pan_gains(t: &Tables, pan: f32) -> (f32, f32) {
    let i = |x: f32| ((x * 1024.0) as i32).clamp(0, 1024) as usize;
    (t.sqrt[i(1.0 - pan)], t.sqrt[i(pan)])
}

/// The cutoff with the filter LFO on it, 1 to 126.
fn lfo_filter(t: &Tables, s: &mut TrackState, value: f32) -> f32 {
    let mut v = value;
    if s.lfo_on {
        if s.lfo_filter != 0.0 {
            v += (t.degrees[(s.carrier[0] as usize).min(359)] + 1.0) * 0.5 * s.lfo_filter;
        }
        s.carrier[0] += s.lfo_rate * s.lfo_scale;
        if s.carrier[0] >= 360.0 {
            s.carrier[0] -= 360.0;
        }
    }
    v.clamp(1.0, 126.0)
}

fn lfo_volume(t: &Tables, s: &mut TrackState) -> f32 {
    let mut v = 1.0;
    if s.lfo_on {
        if s.lfo_volume != 0.0 {
            v = 1.0 - (t.degrees[(s.carrier[1] as usize).min(359)] + 1.0) * 0.5 * s.lfo_volume / 128.0;
        }
        s.carrier[1] += s.lfo_rate * s.lfo_scale;
        if s.carrier[1] >= 360.0 {
            s.carrier[1] -= 360.0;
        }
    }
    v.clamp(0.0, 1.0)
}
