// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! **Face The Music's SEL scripts** ("sound effect language"): up to 64
//! numbered scripts of up to 512 lines, started on a track by a pattern
//! event, that play the song's samples by program: waits and jumps,
//! loops and conditions, interrupts on what the pattern does next, the
//! track's pitch, detune, volume and sample, where in the sample it
//! plays, four LFOs (on pitch, volume or each other), other tracks
//! worked on or copied from, and the song's volume, tempo and speed.
//!
//! The machine is here; the player it runs in is the host ([`Host`]):
//! each tick, after the pattern's row, [`Sel::tick`] runs every track's
//! script, which reads and sets the tracks through the host; then
//! [`Sel::mods`] gives what modulates each track's sound this tick (the
//! LFOs' volume and pitch, the detune), for the host to apply.
//!
//! Units: pitch in eighths of a semitone above C-4 (0 to 271, the
//! patterns' 34 notes); volume 0 to 64; positions in frames (bytes of
//! the 8-bit samples); the tempo as the CIA timer value
//! (1,777,517.482 / value beats a minute).
//!
//! Written from Face The Music's manual's description of the commands,
//! and OpenMPT's reading of the format (its BSD-licensed loader and
//! script player, read for what each command does, not copied).

/// A line of a script: its command and three bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Line {
    pub op: u8,
    pub data: [u8; 3],
}

impl Line {
    fn u24(&self) -> u32 {
        u32::from(self.data[0]) << 16 | u32::from(self.data[1]) << 8 | u32::from(self.data[2])
    }
    fn u16(&self) -> u16 {
        (self.u24() & 0xFFFF) as u16
    }
    fn lo12(&self) -> u16 {
        (self.u24() & 0xFFF) as u16
    }
    fn hi12(&self) -> u16 {
        (self.u24() >> 12) as u16
    }
    fn u8(&self) -> u8 {
        self.data[2]
    }
}

/// A song's scripts, by number (0 to 63), and its rows a measure (for
/// JUMP TO SONGLINE).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Program {
    pub scripts: Vec<Option<Vec<Line>>>,
    pub rows_per_measure: u16,
}

impl Program {
    pub fn new(rows_per_measure: u16) -> Program {
        Program { scripts: vec![None; 64], rows_per_measure }
    }

    pub fn is_empty(&self) -> bool {
        self.scripts.iter().all(|s| s.is_none())
    }

    fn line(&self, at: Addr) -> Option<&Line> {
        self.scripts.get(usize::from(at.script))?.as_ref()?.get(usize::from(at.line))
    }
}

/// What the pattern did on a track this row: SEL's interrupts wait for these.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct RowEvents {
    /// A note.
    pub note: bool,
    /// A volume.
    pub volume: bool,
    /// A sample named.
    pub sample: bool,
    /// The note let go.
    pub release: bool,
    /// A pitch bend.
    pub bend: bool,
    /// A volume fade.
    pub fade: bool,
}

/// The player SEL runs in: its tracks, read and set.
pub trait Host {
    fn tracks(&self) -> usize;
    /// In eighths of a semitone above C-4.
    fn pitch(&self, track: usize) -> i32;
    fn set_pitch(&mut self, track: usize, pitch: i32);
    /// 0 to 64.
    fn volume(&self, track: usize) -> u8;
    fn set_volume(&mut self, track: usize, volume: u8);
    /// The sample the track plays next (1-based), going on at the end of
    /// what's playing (the Amiga's way).
    fn set_sample(&mut self, track: usize, sample: u8);
    /// Its sample from `from` (frames), at its pitch.
    fn play(&mut self, track: usize, from: u32);
    /// Its sample stopped.
    fn quiet(&mut self, track: usize);
    fn position(&self, track: usize) -> u32;
    fn set_position(&mut self, track: usize, frames: u32);
    /// Its sample's loop, as playing: (start, end, the sample's frames).
    fn loop_points(&self, track: usize) -> (u32, u32, u32);
    /// Its loop changed (for this note only); start == end: none.
    fn set_loop(&mut self, track: usize, start: u32, end: u32);
    /// Another track's pitch (1), volume (2), sample (4: and where it is in it), or all (8).
    fn copy(&mut self, track: usize, from: usize, what: u8);
    /// 0 to 64.
    fn set_global_volume(&mut self, volume: u8);
    fn set_tempo(&mut self, cia: u16);
    fn set_speed(&mut self, ticks: u8);
    fn jump(&mut self, measure: u16, row: u16);
}

/// What modulates a track's sound this tick.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Mods {
    /// Added to its final volume, of 16,384 for the whole of it (LFO on volume).
    pub volume_add: Option<i32>,
    /// Its pitch slid, in 64ths of a semitone (LFO on pitch).
    pub slide: i32,
    /// Its detune, in semitones.
    pub detune: f32,
}

/// A place in the scripts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Addr {
    script: u8,
    line: u16,
}

#[derive(Clone, Copy, Debug, Default)]
struct Lfo {
    /// Bits 0-2 the wave (sine, square, triangle, ramp down, ramp up),
    /// bit 3 once only, bits 4-7 the target.
    target: u8,
    speed: u8,
    depth: u8,
    pos: u8,
}

/// The interrupts: new pitch, volume, sample, release, bend, fade.
const INTERRUPTS: usize = 6;

#[derive(Clone, Debug, Default)]
struct Track {
    at: Option<Addr>,
    wait: u16,
    loops: u16,
    on: [Option<Addr>; INTERRUPTS],
    lfo: [Lfo; 4],
    /// The track it works on (1-based), when not its own.
    work: Option<usize>,
    sample_start: u32,
    /// In the units the manual gives (4096ths of a semitone, down).
    detune: Option<i32>,
    volume_add: Option<i32>,
    slide: i32,
}

/// Every track's script, running.
#[derive(Clone, Debug, Default)]
pub struct Sel {
    tracks: Vec<Track>,
}

/// The sine the LFOs use: 256 points, ±64.
fn sine(i: u8) -> i32 {
    (64.0 * (f64::from(i) * std::f64::consts::TAU / 256.0).sin()).round() as i32
}

impl Sel {
    /// Room for `tracks` tracks (made once; `reset` doesn't allocate).
    pub fn new(tracks: usize) -> Sel {
        Sel { tracks: vec![Track::default(); tracks] }
    }

    /// Every script stopped, as at the song's start.
    pub fn reset(&mut self) {
        for t in &mut self.tracks {
            *t = Track::default();
        }
    }

    /// Script `index` started on a track by its pattern (on the row's
    /// first tick): it runs from its first line, working on its own
    /// track with no interrupts set. A number with no script stops it.
    pub fn start(&mut self, program: &Program, track: usize, index: u8) {
        let Some(t) = self.tracks.get_mut(track) else { return };
        t.work = None;
        t.on = [None; INTERRUPTS];
        t.wait = 0;
        t.at = program.scripts.get(usize::from(index)).and_then(|s| s.as_ref()).map(|_| Addr { script: index, line: 0 });
    }

    /// A tick: each track's script run, after the row's events (`rows`,
    /// a track each) are known.
    pub fn tick(&mut self, program: &Program, host: &mut dyn Host, rows: &[RowEvents]) {
        let n = host.tracks().min(self.tracks.len());
        for i in 0..n {
            self.run(program, host, rows, i);
            self.lfos(i);
        }
    }

    /// What modulates `track` this tick: the LFOs of every script working
    /// on it, and the detune of its pair (the Amiga's channels in pairs).
    pub fn mods(&self, track: usize) -> Mods {
        let mut m = Mods::default();
        for (i, t) in self.tracks.iter().enumerate() {
            if t.work.map_or(i, |w| w - 1) != track {
                continue;
            }
            if let Some(v) = t.volume_add {
                m.volume_add = Some(m.volume_add.unwrap_or(0) + v);
            }
            m.slide += t.slide;
            if i % 2 == 0 {
                if let Some(d) = t.detune {
                    m.detune = -(d as f32) / 4096.0;
                }
            }
        }
        m
    }

    fn real(&self, i: usize, tracks: usize) -> usize {
        self.tracks[i].work.map_or(i, |w| (w - 1) % tracks.max(1))
    }

    fn run(&mut self, program: &Program, host: &mut dyn Host, rows: &[RowEvents], i: usize) {
        let tracks = host.tracks();
        // An interrupt waiting for what the pattern did: it goes once.
        let ev = rows.get(self.real(i, tracks)).copied().unwrap_or_default();
        let t = &mut self.tracks[i];
        for (k, happened) in [ev.note, ev.volume, ev.sample, ev.release, ev.bend, ev.fade].into_iter().enumerate() {
            if happened {
                if let Some(to) = t.on[k].take() {
                    t.at = Some(to);
                    t.wait = 0;
                }
            }
        }
        if t.wait > 0 {
            t.wait -= 1;
            return;
        }
        let mut jumps = 0;
        let mut steps = 0;
        while let Some(at) = self.tracks[i].at {
            steps += 1;
            if steps > 4096 {
                break;
            }
            let Some(line) = program.line(at).copied() else {
                self.stop(i);
                break;
            };
            self.tracks[i].at = Some(Addr { script: at.script, line: at.line + 1 });
            let real = self.real(i, tracks);
            match self.exec(program, host, i, real, at, line) {
                Flow::Next => {}
                Flow::Jumped => {
                    jumps += 1;
                    if jumps > 10 {
                        break;
                    }
                }
                Flow::Yield => break,
            }
        }
    }

    fn stop(&mut self, i: usize) {
        let t = &mut self.tracks[i];
        t.at = None;
        t.on = [None; INTERRUPTS];
    }

    fn exec(&mut self, program: &Program, host: &mut dyn Host, i: usize, real: usize, at: Addr, l: Line) -> Flow {
        let local = |line: u16| Addr { script: at.script, line: line.min(0x200) };
        let tracks = host.tracks();
        match l.op {
            // WAIT t TICKS: 0 stops the script.
            1 => {
                if l.u16() == 0 {
                    self.tracks[i].at = None;
                } else {
                    self.tracks[i].wait = l.u16() - 1;
                }
                return Flow::Yield;
            }
            // GOTO LINE
            2 => {
                self.tracks[i].at = Some(local(l.lo12()));
                return Flow::Jumped;
            }
            // LOOP x TIMES TO LINE: counted down, set when it's run out.
            3 => {
                let t = &mut self.tracks[i];
                if t.loops == 0 {
                    t.loops = 1 + l.hi12();
                }
                if t.loops > 1 {
                    t.at = Some(local(l.lo12()));
                }
                t.loops -= 1;
                return Flow::Jumped;
            }
            // GOTO EFFECT e, LINE l
            4 => {
                let script = (l.hi12() & 63) as u8;
                let ok = program.scripts.get(usize::from(script)).is_some_and(|s| s.is_some());
                if ok {
                    self.tracks[i].at = Some(Addr { script, line: l.lo12().min(0x200) });
                } else {
                    self.stop(i);
                }
                return Flow::Jumped;
            }
            // END OF EFFECT
            5 => {
                self.stop(i);
                return Flow::Yield;
            }
            // IF PITCH (=, <, >) v GOTO l; IF VOLUME (=, <, >) v GOTO l
            6..=11 => {
                let v = i32::from(l.hi12());
                let now = if l.op < 9 { host.pitch(real) } else { i32::from(host.volume(real)) };
                let yes = match (l.op - 6) % 3 {
                    0 => now == v,
                    1 => now < v,
                    _ => now > v,
                };
                if yes {
                    self.tracks[i].at = Some(local(l.lo12()));
                    return Flow::Jumped;
                }
            }
            // ON NEW PITCH, VOLUME, SAMPLE, RELEASE, PITCH BEND, VOLUME DOWN: GOTO l
            12..=17 => self.tracks[i].on[usize::from(l.op - 12)] = Some(local(l.lo12())),
            18 => host.play(real, 0),
            19 => host.quiet(real),
            // PLAY POSITION =, +, - (words)
            20..=22 => {
                let p = (l.u24() & 0xFFFFF) << 1;
                let now = host.position(real);
                let to = match l.op {
                    20 => p.saturating_add(self.tracks[i].sample_start),
                    21 => now.saturating_add(p),
                    _ => now.saturating_sub(p),
                };
                let (_, _, frames) = host.loop_points(real);
                host.set_position(real, to.min(frames));
            }
            23 => host.set_pitch(real, i32::from(l.lo12().min(0x10F))),
            // Detune goes on the first of the real track's pair.
            24 => {
                let pair = (real & !1).min(self.tracks.len() - 1);
                self.tracks[pair].detune = Some(i32::from(l.lo12()));
            }
            // DETUNE/PITCH ADD, SUB
            25 | 26 => {
                let sign = if l.op == 26 { -1 } else { 1 };
                let k = (real & !1).min(self.tracks.len() - 1);
                let pair = &mut self.tracks[k];
                pair.detune = Some(pair.detune.unwrap_or(0) + sign * i32::from(l.hi12()));
                let d = sign * i32::from(l.lo12());
                if d != 0 {
                    let p = host.pitch(real);
                    let limit = if d < 0 { 0 } else { 0x10F };
                    host.set_pitch(real, if d < 0 { (p + d).max(limit.min(p)) } else { (p + d).min(limit.max(p)) });
                }
            }
            27 => host.set_volume(real, l.u8().min(64)),
            28 | 29 => {
                let v = i32::from(host.volume(real)) + if l.op == 29 { -i32::from(l.u8()) } else { i32::from(l.u8()) };
                host.set_volume(real, v.clamp(0, 64) as u8);
            }
            30 => host.set_sample(real, l.u8().saturating_add(1)),
            // SAMPLE START =, +, - (the relative ones in bytes, as the program has it)
            31..=33 => {
                let v = ((l.u24() & 0xFFFFF) >> 1).min(0xFFFF);
                let s = &mut self.tracks[i].sample_start;
                *s = match l.op {
                    31 => v * 2,
                    32 => s.saturating_add(v),
                    _ => s.saturating_sub(v),
                };
            }
            // ONE-SHOT LENGTH =, +, - (where the loop starts): the loop moved, its length kept
            34..=36 => {
                let (start, end, frames) = host.loop_points(real);
                let len = end.saturating_sub(start);
                let v = i64::from(l.u16()) * 2;
                let s = match l.op {
                    34 => v,
                    35 => i64::from(start) + v,
                    _ => i64::from(start) - v,
                }
                .clamp(0, i64::from(frames)) as u32;
                host.set_loop(real, s, (s + len).min(frames));
            }
            // REPEAT LENGTH =, +, -
            37..=39 => {
                let (start, end, frames) = host.loop_points(real);
                let v = i64::from(l.u16()) * 2;
                let e = match l.op {
                    37 => i64::from(start) + v,
                    38 => i64::from(end) + v,
                    _ => i64::from(end) - v,
                }
                .clamp(i64::from(start), i64::from(frames)) as u32;
                host.set_loop(real, start, e);
            }
            // GET PITCH, VOLUME, SAMPLE OF TRACK t; CLONE TRACK t
            40..=43 => {
                let from = usize::from(l.u8() & 7);
                if from < tracks {
                    host.copy(real, from, 1 << (l.op - 40));
                }
            }
            // LFO n START (its target and wave), then SPEED/DEPTH ADD or SUB
            44..=55 => {
                let n = usize::from((l.op - 44) / 3);
                let kind = (l.op - 44) % 3;
                let lfo = &mut self.tracks[i].lfo[n];
                if kind == 0 {
                    *lfo = Lfo { target: l.data[0], ..Lfo::default() };
                }
                let (ds, dd) = (i32::from(l.data[1]), i32::from(l.data[2]));
                let sign = if kind == 2 { -1 } else { 1 };
                lfo.speed = (i32::from(lfo.speed) + sign * ds).clamp(0, 0xBF) as u8;
                lfo.depth = (i32::from(lfo.depth) + sign * dd).clamp(0, 0x7F) as u8;
            }
            // WORK ON TRACK t; WORK TRACK ADD t
            56 => self.tracks[i].work = Some(usize::from(l.u8() & 7) + 1),
            57 => {
                let add = usize::from(l.u8() & 7);
                if add > 0 {
                    let from = self.tracks[i].work.map_or(i, |w| w - 1);
                    self.tracks[i].work = Some((from + add) % tracks.max(1) + 1);
                }
            }
            58 => host.set_global_volume(l.u8().min(64)),
            59 => host.set_tempo(l.u16().clamp(0x1000, 0x4FFF)),
            60 => host.set_speed(l.u8().max(1)),
            61 => {
                let rpm = program.rows_per_measure.max(1);
                host.jump(l.u16() / rpm, l.u16() % rpm);
            }
            _ => {}
        }
        Flow::Next
    }

    /// The LFOs a tick on: their value set on their target, then moved on.
    fn lfos(&mut self, i: usize) {
        let t = &mut self.tracks[i];
        t.volume_add = None;
        t.slide = 0;
        for n in 0..4 {
            let lfo = t.lfo[n];
            if lfo.speed == 0 && lfo.depth == 0 {
                continue;
            }
            let at = (u32::from(lfo.pos) * 256 / 192) as u8;
            let mut v = match lfo.target & 7 {
                0 => sine(at),
                1 => {
                    if at < 128 {
                        64
                    } else {
                        -64
                    }
                }
                2 => 64 - ((i32::from(at) + 64) % 256 - 128).abs(),
                3 => 64 - i32::from(at) / 2,
                4 => i32::from(at) / 2 - 64,
                _ => 0,
            };
            // An LFO driving another's speed or depth swings from 0 up.
            if lfo.target & 0xF0 < 0xA0 {
                v += 64;
            }
            v *= i32::from(lfo.depth);
            match lfo.target & 0xF0 {
                0x10..=0x40 => t.lfo[usize::from((lfo.target >> 4) - 1)].speed = (v / 64).clamp(0, 255) as u8,
                0x50..=0x80 => t.lfo[usize::from((lfo.target >> 4) - 5)].depth = (v / 64).clamp(0, 255) as u8,
                0xA0 => t.volume_add = Some((v * 4).clamp(-32768, 32767)),
                0xF0 => t.slide = v / 8,
                _ => {}
            }
            let lfo = &mut t.lfo[n];
            let mut pos = u32::from(lfo.pos) + u32::from(lfo.speed);
            if pos >= 192 {
                pos -= 192;
                if lfo.target & 8 != 0 {
                    lfo.speed = 0;
                    pos = 191;
                }
            }
            lfo.pos = pos as u8;
        }
    }
}

enum Flow {
    Next,
    Jumped,
    Yield,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        pitch: [i32; 8],
        volume: [u8; 8],
        played: Vec<(usize, u32)>,
        speed: u8,
    }

    impl Host for Fake {
        fn tracks(&self) -> usize {
            8
        }
        fn pitch(&self, t: usize) -> i32 {
            self.pitch[t]
        }
        fn set_pitch(&mut self, t: usize, p: i32) {
            self.pitch[t] = p;
        }
        fn volume(&self, t: usize) -> u8 {
            self.volume[t]
        }
        fn set_volume(&mut self, t: usize, v: u8) {
            self.volume[t] = v;
        }
        fn set_sample(&mut self, _: usize, _: u8) {}
        fn play(&mut self, t: usize, from: u32) {
            self.played.push((t, from));
        }
        fn quiet(&mut self, _: usize) {}
        fn position(&self, _: usize) -> u32 {
            0
        }
        fn set_position(&mut self, _: usize, _: u32) {}
        fn loop_points(&self, _: usize) -> (u32, u32, u32) {
            (0, 0, 1000)
        }
        fn set_loop(&mut self, _: usize, _: u32, _: u32) {}
        fn copy(&mut self, t: usize, from: usize, what: u8) {
            if what & 1 != 0 {
                self.pitch[t] = self.pitch[from];
            }
        }
        fn set_global_volume(&mut self, _: u8) {}
        fn set_tempo(&mut self, _: u16) {}
        fn set_speed(&mut self, s: u8) {
            self.speed = s;
        }
        fn jump(&mut self, _: u16, _: u16) {}
    }

    fn line(op: u8, a: u8, b: u8, c: u8) -> Line {
        Line { op, data: [a, b, c] }
    }

    #[test]
    fn waits_loops_and_volume() {
        let mut p = Program::new(16);
        // Volume 40, then 3 times: wait 2 ticks, volume down 10.
        p.scripts[5] = Some(vec![line(27, 0, 0, 40), line(1, 0, 0, 2), line(29, 0, 0, 10), line(3, 0, 0x20, 1), line(5, 0, 0, 0)]);
        let mut sel = Sel::new(8);
        let mut h = Fake::default();
        sel.start(&p, 0, 5);
        let rows = [RowEvents::default(); 8];
        let mut seen = Vec::new();
        for _ in 0..10 {
            sel.tick(&p, &mut h, &rows);
            seen.push(h.volume[0]);
        }
        assert_eq!(seen, vec![40, 40, 30, 30, 20, 20, 10, 10, 10, 10]);
    }

    #[test]
    fn interrupts_fire_once_and_work_tracks_redirect() {
        let mut p = Program::new(16);
        // On a new note go to line 3; wait forever; (2) nothing; (3) work on track 2 and play it.
        p.scripts[0] = Some(vec![line(12, 0, 0, 3), line(1, 0, 0xFF, 0xFF), line(0, 0, 0, 0), line(56, 0, 0, 2), line(18, 0, 0, 0), line(1, 0, 0, 0)]);
        let mut sel = Sel::new(8);
        let mut h = Fake::default();
        sel.start(&p, 1, 0);
        let mut rows = [RowEvents::default(); 8];
        sel.tick(&p, &mut h, &rows);
        assert!(h.played.is_empty());
        rows[1].note = true;
        sel.tick(&p, &mut h, &rows);
        assert_eq!(h.played, vec![(2, 0)]);
        sel.tick(&p, &mut h, &rows);
        assert_eq!(h.played.len(), 1);
    }

    #[test]
    fn an_lfo_slides_the_pitch_and_detune_goes_to_the_pair() {
        let mut p = Program::new(16);
        // LFO 1 on pitch (F0), sine, speed 8, depth 64; detune 0x400.
        p.scripts[1] = Some(vec![line(44, 0xF0, 8, 64), line(24, 0, 0x04, 0x00), line(1, 0, 0, 0)]);
        let mut sel = Sel::new(8);
        let mut h = Fake::default();
        sel.start(&p, 3, 1);
        let rows = [RowEvents::default(); 8];
        let mut most = 0;
        for _ in 0..30 {
            sel.tick(&p, &mut h, &rows);
            most = most.max(sel.mods(3).slide.abs());
        }
        assert!(most > 400 && most <= 64 * 64 / 8);
        assert_eq!(sel.mods(2).detune, -0.25);
        assert_eq!(sel.mods(3).detune, 0.0);
    }
}
