//! ProTrekkr's two TB-303 voices: a sawtooth or square run through a
//! four-pole resonant low-pass whose cutoff, resonance and envelope
//! amount fall a little every frame from where a note sets them, with
//! accents, slides (a glide over half a row) and a step sequencer of 32
//! patterns of up to 16 steps.

use super::Tb303;

/// A step's flags.
pub(crate) const SLIDE: u8 = 1;
pub(crate) const ACCENT: u8 = 2;
pub(crate) const UP: u8 = 4;
pub(crate) const DOWN: u8 = 8;
/// A note (a step without it rests).
pub(crate) const NOTE: u8 = 16;

#[derive(Clone, Debug)]
pub(crate) struct Gear {
    /// The pattern playing (255 none), and its step (255 none).
    pub pattern: usize,
    pub line: usize,
    pub volume: f32,
    pub note_off: bool,
    pub multiple: i32,
    pub ramp_volume: f32,
    pub target_real_volume: f32,
    target_real_volume_ramp: f32,
    cutoff: f32,
    resonance: f32,
    envmod: f32,
    decay: f32,
    accent: f32,
    sample: f32,
    real_cutoff: f32,
    real_resonance: f32,
    real_envmod: f32,
    last_cutoff: f32,
    last_envmod: f32,
    last_resonance: f32,
    real_volume: f32,
    speed_freak: f32,
    current_volume: f32,
    innertime: f32,
    position: f32,
    speed: f32,
    tb_ramp_volume: f32,
    auto_cut: bool,
    auto_res: bool,
    auto_env: bool,
    auto_mod: bool,
    first_row: bool,
    waveform: u8,
    buf: [f32; 5],
}

impl Gear {
    pub fn new(volume: f32) -> Gear {
        Gear { volume, ..Gear::blank() }
    }

    pub fn reset(&mut self) {
        let volume = self.volume;
        *self = Gear { volume, ..Gear::blank() };
    }

    fn blank() -> Gear {
        Gear {
            pattern: 255,
            line: 255,
            volume: 0.5,
            note_off: false,
            multiple: 0,
            ramp_volume: 0.0,
            target_real_volume: 1.0,
            target_real_volume_ramp: 0.0,
            cutoff: 0.5,
            resonance: 0.5,
            envmod: 0.5,
            decay: 0.5,
            accent: 0.0,
            sample: 0.0,
            real_cutoff: 0.0,
            real_resonance: 0.0,
            real_envmod: 0.0,
            last_cutoff: 0.0,
            last_envmod: 0.0,
            last_resonance: 0.0,
            real_volume: 0.0,
            speed_freak: 0.0,
            current_volume: 0.0,
            innertime: 0.0,
            position: 0.0,
            speed: 0.0,
            tb_ramp_volume: 0.0,
            auto_cut: true,
            auto_res: true,
            auto_env: true,
            auto_mod: true,
            first_row: true,
            waveform: 0,
            buf: [0.0; 5],
        }
    }

    /// The next frame. `samples_per_tick` is a row's length in frames.
    pub fn sample(&mut self, p: &Tb303, samples_per_tick: i32) -> f32 {
        self.sample =if self.waveform == 1 {
            if self.position < 0.0 {
                -16384.0 * (1.0 + self.accent)
            } else {
                16384.0 * (1.0 + self.accent)
            }
        } else {
            self.position * (1.0 + self.accent)
        };
        self.position += self.speed;
        if self.innertime > 0.0 {
            self.innertime -= 1.0;
            self.speed += self.speed_freak;
        }
        if self.position >= 16384.0 {
            self.position -= 32768.0;
        }
        // The cutoff, resonance and envelope fall by the decay a frame.
        let tick_pos = (self.decay / f32::from(p.scale.max(1))) / samples_per_tick.max(1) as f32;
        if self.auto_mod {
            if self.auto_cut {
                self.last_cutoff = self.real_cutoff;
                self.real_cutoff -= tick_pos;
                if self.real_cutoff < -0.25 {
                    self.real_cutoff = -0.25;
                    self.auto_cut = false;
                }
            }
            if self.auto_res {
                self.last_resonance = self.real_resonance;
                self.real_resonance -= tick_pos;
                if self.real_resonance < -0.25 {
                    self.real_resonance = -0.25;
                    self.auto_res = false;
                }
            }
            if self.auto_env {
                self.last_envmod = self.real_envmod;
                self.real_envmod -= tick_pos;
                if self.real_envmod < self.envmod - 1.0 {
                    self.real_envmod = self.envmod - 1.0;
                    self.auto_env = false;
                }
            }
        }
        let cut = self.real_cutoff.clamp(-0.25, 1.0);
        let env = self.real_envmod.clamp(-0.25, 1.0);
        let res = self.real_resonance.clamp(0.0, 1.0);
        let target = self.real_volume.min(1.0);
        if self.current_volume < target {
            self.current_volume = (self.current_volume + 0.001).min(target);
        } else {
            self.current_volume = (self.current_volume - 0.001).max(target);
        }
        let out = (self.filter(cut, res, env) * (1.0 + self.accent)).clamp(-32767.0, 32767.0);
        if self.accent > 0.0 {
            self.accent = (self.accent - 0.0001).max(0.0);
        }
        if self.tb_ramp_volume > self.ramp_volume {
            // An explicit note off fades slowly.
            self.tb_ramp_volume = (self.tb_ramp_volume - 0.0005).max(self.ramp_volume);
        } else {
            self.tb_ramp_volume = (self.tb_ramp_volume + 0.05).min(self.ramp_volume);
        }
        if self.target_real_volume_ramp > self.target_real_volume {
            self.target_real_volume_ramp = (self.target_real_volume_ramp - 0.1).max(self.target_real_volume);
        } else {
            self.target_real_volume_ramp = (self.target_real_volume_ramp + 0.1).min(self.target_real_volume);
        }
        out * self.target_real_volume_ramp * self.tb_ramp_volume * self.current_volume
    }

    /// The step's note starts (or slides in, or rests).
    pub fn note_on(&mut self, note: i32, p: &Tb303, samples_per_tick: i32) {
        if self.note_off {
            return;
        }
        let pat = self.pattern.min(31);
        let flags = |line: usize| p.flags[pat][line.min(15)];
        let mut note = note;
        if self.line == 255 {
            self.line = usize::from(p.lengths[pat]).saturating_sub(1);
        }
        let line = self.line;
        if flags(line) & UP != 0 {
            note += 12;
        }
        if flags(line) & DOWN != 0 {
            note -= 12;
        }
        self.ramp_volume = 1.0;
        self.waveform = p.waveform;
        self.speed_freak = 0.0;
        let tune = f32::from(p.tune) * 0.188_976_3;
        let speed_to = super::pow2((note as f32 - 17.0 + tune) / 12.0) * 64.0;
        let prev = if line == 0 { usize::from(p.lengths[pat]).saturating_sub(1) } else { line - 1 };
        let reinit = |g: &mut Gear| {
            g.auto_cut = true;
            g.auto_res = true;
            g.auto_env = true;
            g.auto_mod = true;
            g.real_cutoff = f32::from(p.cutoff) / 127.0;
            g.real_envmod = f32::from(p.envmod) / 127.0;
            g.real_resonance = f32::from(p.resonance) / 127.0;
            g.decay = 1.0 - f32::from(p.decay) / 127.0;
        };
        let resume = |g: &mut Gear| {
            g.real_cutoff = g.last_cutoff;
            g.real_envmod = g.last_envmod;
            g.real_resonance = g.last_resonance;
        };
        if flags(line) & SLIDE != 0 {
            if flags(prev) & SLIDE != 0 && !self.first_row {
                resume(self);
            } else {
                self.first_row = false;
                reinit(self);
            }
        } else if flags(prev) & SLIDE != 0 {
            if self.first_row {
                reinit(self);
            } else {
                resume(self);
            }
        } else {
            reinit(self);
        }
        let mut fresh = true;
        // A slide from the step before (never at the pattern's start).
        if flags(prev) & SLIDE != 0 && !self.first_row {
            fresh = false;
            let mut from = f32::from(p.tones[pat][prev.min(15)]) - 17.0;
            if flags(prev) & UP != 0 {
                from += 12.0;
            }
            if flags(prev) & DOWN != 0 {
                from -= 12.0;
            }
            from += tune;
            self.innertime = samples_per_tick as f32 * 0.5;
            let source = super::pow2(from / 12.0) * 64.0;
            self.speed = source;
            self.speed_freak = (speed_to - source) / self.innertime;
        }
        if flags(line) & NOTE == 0 && fresh {
            // A rest.
            self.real_volume = 0.0;
            self.auto_cut = true;
            self.auto_res = true;
            self.auto_env = true;
            self.auto_mod = true;
            resume(self);
        } else {
            if fresh {
                self.speed = speed_to;
                self.position = 0.0;
                self.cutoff = f32::from(p.cutoff) / 127.0;
                self.real_cutoff = self.cutoff;
                self.envmod = f32::from(p.envmod) / 127.0;
                self.real_envmod = self.envmod;
                self.resonance = f32::from(p.resonance) / 127.0;
                self.real_resonance = self.resonance;
                self.decay = 1.0 - f32::from(p.decay) / 127.0;
            }
            if flags(line) & SLIDE != 0 {
                self.decay += 0.05;
            }
            self.real_volume = self.volume;
            if flags(line) & ACCENT != 0 {
                self.accent = f32::from(p.accent) / 127.0 * 2.0;
                self.decay = 1.0;
            }
        }
    }

    /// The four-pole filter, its cutoff opened by the envelope amount.
    fn filter(&mut self, cut: f32, res: f32, env: f32) -> f32 {
        let envcut = 1.0 + env * 0.85;
        let envcut2 = 1.0 - env * 0.85;
        let cut = (cut + 0.55) * envcut * 0.5;
        let q = 0.85 - cut;
        let p = cut * cut * 0.45;
        let f = p + p - 1.0;
        let res = res * 5.0;
        let q = (res * (1.0 + 0.5 * q * (1.0 - q + 5.6 * q * q))).min(2.42);
        let b = &mut self.buf;
        let input = self.sample / 16384.0 - q * b[4] * envcut2;
        let (t1, t2, t3) = (b[1], b[2], b[3]);
        b[1] = (input + b[0]) * p - b[1] * f;
        b[2] = (b[1] + t1) * p - b[2] * f;
        b[3] = (b[2] + t2) * p - b[3] * f;
        b[4] = (b[3] + t3) * p - b[4] * f;
        b[0] = input;
        b[4] * 16384.0
    }
}
