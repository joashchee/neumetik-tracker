// Copyright 2026 ansiapps. Neumetik Synth: MIT licensed, see the LICENSE at the repository root.

//! The Amiga's Paula, as the Amiga replayers here play it: channels that
//! each step through a part of memory (8-bit points) at a period of the
//! PAL clock, at a volume of 0 to 64. A channel started plays its part
//! once, then the loop it's been given, over and over (a loop set while
//! it plays waits for the part playing to end, as the chip's registers
//! do); with none, it falls silent. The memory is the replayer's, read
//! as the channel plays, so a replayer that remakes a wave in place is
//! heard doing it.

use crate::replay::Bufs;

/// The PAL Amiga's clock, which periods count.
pub const CLOCK: f64 = 3_546_895.0;
/// Its screen's refresh, which times some replayers' ticks: 313 lines of
/// 227 clocks.
pub const VBLANK: f64 = CLOCK / (313.0 * 227.0);
/// The CIA timer at "50 a second", which times others': set to 14,187 of
/// its 709,379 a second, it counts 14,188 a tick.
pub const CIA_50: f64 = 709_379.0 / 14_188.0;

/// A part chosen afresh at each of a channel's reloads, as a replayer's
/// audio interrupt would choose it (the chip raises one each time it
/// takes its registers up): from `base`, `len` points `offset` on, the
/// offset then stepped on; once past `limit`, silence at `empty`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stepper {
    pub base: usize,
    pub offset: usize,
    pub step: usize,
    pub limit: usize,
    pub len: usize,
    pub empty: usize,
    pub stopped: bool,
}

impl Stepper {
    /// The part for the next reload, the offset stepped on.
    fn next(&mut self) -> (usize, usize) {
        let at = self.offset;
        self.offset += self.step;
        if self.stopped || self.offset >= self.limit {
            self.stopped = true;
            self.offset = 0;
            (self.empty, self.len)
        } else {
            (self.base + at, self.len)
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Chan {
    on: bool,
    start: usize,
    len: usize,
    /// Where it is in the part, in points.
    pos: f64,
    looped: Option<(usize, usize)>,
    stepper: Option<Stepper>,
    period: u32,
    /// 0 to 256.
    volume: u32,
}

/// The chip, its memory and its channels.
#[derive(Clone, Debug)]
pub struct Paula {
    pub mem: Vec<i8>,
    chans: Vec<Chan>,
    /// Each channel's place, 0 left to 1 right.
    pans: Vec<f32>,
}

impl Paula {
    /// `channels` channels (left, right, right, left, as the Amiga's
    /// four go) over `mem`.
    pub fn new(channels: usize, mem: Vec<i8>) -> Paula {
        Paula { mem, chans: vec![Chan::default(); channels], pans: (0..channels).map(|c| if matches!(c % 4, 0 | 3) { 0.0 } else { 1.0 }).collect() }
    }

    pub fn channels(&self) -> usize {
        self.chans.len()
    }

    /// More memory, `len` points of silence; where it starts.
    pub fn alloc(&mut self, len: usize) -> usize {
        let at = self.mem.len();
        self.mem.resize(at + len, 0);
        at
    }

    /// `data` put in memory; where it starts.
    pub fn load(&mut self, data: &[i8]) -> usize {
        let at = self.mem.len();
        self.mem.extend_from_slice(data);
        at
    }

    /// The channel starts `len` points at `start`, from their first, with
    /// no loop until one is set.
    pub fn play(&mut self, c: usize, start: usize, len: usize) {
        let Some(ch) = self.chans.get_mut(c) else { return };
        ch.on = len > 0;
        ch.start = start;
        ch.len = len;
        ch.pos = 0.0;
        ch.looped = None;
    }

    /// What the channel plays once its part ends: `len` points at
    /// `start`, round and round.
    pub fn set_loop(&mut self, c: usize, start: usize, len: usize) {
        if let Some(ch) = self.chans.get_mut(c) {
            ch.looped = (len > 0).then_some((start, len));
        }
    }

    /// The channel's next parts chosen at each reload by `stepper`
    /// (none: its loop, as set).
    pub fn set_stepper(&mut self, c: usize, stepper: Option<Stepper>) {
        if let Some(ch) = self.chans.get_mut(c) {
            ch.stepper = stepper;
        }
    }

    pub fn stepper_mut(&mut self, c: usize) -> Option<&mut Stepper> {
        self.chans.get_mut(c).and_then(|ch| ch.stepper.as_mut())
    }

    /// The channel stops at once.
    pub fn stop(&mut self, c: usize) {
        if let Some(ch) = self.chans.get_mut(c) {
            ch.on = false;
            ch.looped = None;
        }
    }

    pub fn set_period(&mut self, c: usize, period: u32) {
        if let Some(ch) = self.chans.get_mut(c) {
            ch.period = period;
        }
    }

    /// 0 to 64.
    pub fn set_volume(&mut self, c: usize, volume: u32) {
        self.set_volume_fine(c, volume.min(64) * 4);
    }

    /// 0 to 256, for the replayers that keep their volumes finer (and
    /// mixers that play them so).
    pub fn set_volume_fine(&mut self, c: usize, volume: u32) {
        if let Some(ch) = self.chans.get_mut(c) {
            ch.volume = volume.min(256);
        }
    }

    pub fn set_pan(&mut self, c: usize, pan: f32) {
        if let Some(p) = self.pans.get_mut(c) {
            *p = pan.clamp(0.0, 1.0);
        }
    }

    /// Whether the channel is playing.
    pub fn playing(&self, c: usize) -> bool {
        self.chans.get(c).is_some_and(|ch| ch.on)
    }

    /// The next `frames` frames of every channel, at `rate`, each point
    /// held until the next (as the chip does), at the Tracker's level.
    pub fn render(&mut self, rate: u32, frames: usize, out: &mut Bufs) {
        let mem = &self.mem;
        for (c, ch) in self.chans.iter_mut().enumerate() {
            let (l, r) = (&mut out.left[c][..frames], &mut out.right[c][..frames]);
            l.fill(0.0);
            r.fill(0.0);
            if !ch.on || ch.period == 0 {
                continue;
            }
            let step = CLOCK / f64::from(ch.period) / f64::from(rate.max(1));
            let gain = ch.volume as f32 / 256.0 * 0.4 / 128.0;
            let (gl, gr) = (gain * (1.0 - self.pans[c]), gain * self.pans[c]);
            for i in 0..frames {
                let x = f32::from(mem.get(ch.start + ch.pos as usize).copied().unwrap_or(0));
                l[i] = x * gl;
                r[i] = x * gr;
                ch.pos += step;
                while ch.pos >= ch.len as f64 {
                    ch.pos -= ch.len as f64;
                    match ch.looped {
                        Some((s, n)) => {
                            ch.start = s;
                            ch.len = n;
                            if let Some(st) = ch.stepper.as_mut() {
                                ch.looped = Some(st.next());
                            }
                            if n == 0 {
                                ch.on = false;
                                break;
                            }
                        }
                        None => {
                            ch.on = false;
                            break;
                        }
                    }
                }
                if !ch.on {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_plays_then_its_loop() {
        let mut p = Paula::new(1, vec![10, 20, 30, 40, 50, 60]);
        p.play(0, 0, 4);
        p.set_loop(0, 4, 2);
        p.set_volume(0, 64);
        // One point a frame.
        p.set_period(0, 100);
        let rate = (CLOCK / 100.0) as u32;
        let mut b = Bufs { left: vec![vec![0.0; 10]], right: vec![vec![0.0; 10]] };
        p.render(rate, 10, &mut b);
        let pts: Vec<i32> = b.left[0].iter().map(|&x| (x * 128.0 / 0.4).round() as i32).collect();
        assert_eq!(pts, [10, 20, 30, 40, 50, 60, 50, 60, 50, 60]);
    }

    #[test]
    fn with_no_loop_it_stops() {
        let mut p = Paula::new(1, vec![100; 8]);
        p.play(0, 0, 4);
        p.set_volume(0, 64);
        p.set_period(0, 100);
        let mut b = Bufs { left: vec![vec![0.0; 8]], right: vec![vec![0.0; 8]] };
        p.render((CLOCK / 100.0) as u32, 8, &mut b);
        assert!(b.left[0][..4].iter().all(|&x| x != 0.0) && b.left[0][4..].iter().all(|&x| x == 0.0));
        assert!(!p.playing(0));
    }

    #[test]
    fn a_stepper_picks_each_next_part() {
        let mut p = Paula::new(1, (0..16).map(|x| x as i8).collect());
        p.play(0, 0, 2);
        p.set_loop(0, 0, 2);
        p.set_stepper(0, Some(Stepper { base: 4, offset: 0, step: 4, limit: 8, len: 2, empty: 15, stopped: false }));
        p.set_volume(0, 64);
        p.set_period(0, 100);
        let mut b = Bufs { left: vec![vec![0.0; 10]], right: vec![vec![0.0; 10]] };
        p.render((CLOCK / 100.0) as u32, 10, &mut b);
        let pts: Vec<i32> = b.left[0].iter().map(|&x| (x * 128.0 / 0.4).round() as i32).collect();
        // Its first part, the loop it had, then 4 on; 8 on is past the
        // limit, so silence (15, then past the memory's end).
        assert_eq!(pts, [0, 1, 0, 1, 4, 5, 15, 0, 15, 0]);
    }
}
