//! A sample's waveform edited, as FastTracker 2's, OpenMPT's and
//! Renoise's sample editors do: a range cut, copied, pasted, cropped,
//! silenced, faded, made louder or quieter, drawn; the whole resampled,
//! changed in depth or channels, its DC offset taken out; its loop
//! crossfaded or moved to a smooth place; time-stretched and
//! pitch-shifted (overlap-add, as WSOLA has it); sliced at its beats;
//! and synths made into samples.
//!
//! Ranges are in frames, `a` to just before `b`. The loops are kept
//! where they were in the sound: moved with what's before them, cut
//! with it, held inside what's left.

use std::sync::Arc;

use crate::model::*;

/// A range held inside the sample.
fn span(s: &Sample, a: usize, b: usize) -> (usize, usize) {
    let n = s.frames();
    let (a, b) = (a.min(n), b.min(n));
    (a.min(b), a.max(b))
}

fn ch(s: &Sample) -> usize {
    usize::from(s.wave.channels.max(1))
}

/// A sample's loops moved for frames taken out (`len` at `at`) or put in.
fn shift_loops(s: &mut Sample, at: usize, removed: usize, added: usize) {
    let at = at as u32;
    let fix = |l: &mut Loop| {
        let mv = |x: u32| -> u32 {
            if x <= at {
                x
            } else if removed > 0 {
                at.max(x.saturating_sub(removed as u32))
            } else {
                x + added as u32
            }
        };
        l.start = mv(l.start);
        l.end = mv(l.end);
    };
    fix(&mut s.looped);
    fix(&mut s.sustain);
    let frames = s.frames() as u32;
    s.looped = s.looped.clamped(frames);
    s.sustain = s.sustain.clamped(frames);
    s.slices.retain(|&x| x < frames);
}

/// A range of the waveform, as a waveform of its own.
pub fn copy(s: &Sample, a: usize, b: usize) -> Wave {
    let (a, b) = span(s, a, b);
    let c = ch(s);
    Wave { bits: s.wave.bits, channels: s.wave.channels.max(1), pcm: s.wave.pcm[a * c..b * c].to_vec() }
}

/// A range taken out.
pub fn cut(s: &mut Sample, a: usize, b: usize) {
    let (a, b) = span(s, a, b);
    let c = ch(s);
    Arc::make_mut(&mut s.wave).pcm.drain(a * c..b * c);
    shift_loops(s, a, b - a, 0);
}

/// Only a range kept.
pub fn crop(s: &mut Sample, a: usize, b: usize) {
    let (a, b) = span(s, a, b);
    let c = ch(s);
    let n = s.frames();
    Arc::make_mut(&mut s.wave).pcm.truncate(b * c);
    shift_loops(s, b, n - b, 0);
    cut(s, 0, a);
}

/// A waveform put in at `at` (made the sample's depth and channels first).
pub fn paste(s: &mut Sample, at: usize, w: &Wave) {
    let at = at.min(s.frames());
    let fitted = convert(w, s.wave.channels.max(1), if s.wave.is_empty() { w.bits } else { s.wave.bits });
    let c = ch(s);
    let frames = fitted.frames();
    let wave = Arc::make_mut(&mut s.wave);
    if wave.pcm.is_empty() {
        *wave = fitted;
    } else {
        wave.pcm.splice(at * c..at * c, fitted.pcm);
    }
    shift_loops(s, at, 0, frames);
}

/// A waveform as `channels` channels (a mix, or one copied) and `bits` deep.
pub fn convert(w: &Wave, channels: u8, bits: u8) -> Wave {
    let from = usize::from(w.channels.max(1));
    let to = usize::from(channels.clamp(1, 2));
    let mut pcm: Vec<i16> = if from == to {
        w.pcm.clone()
    } else if to == 1 {
        w.pcm.chunks(from).map(|f| (f.iter().map(|&v| i32::from(v)).sum::<i32>() / from as i32) as i16).collect()
    } else {
        w.pcm.iter().flat_map(|&v| [v, v]).collect()
    };
    if bits <= 8 {
        // To 8 bits, rounded.
        for v in &mut pcm {
            *v = (((i32::from(*v) + 128) >> 8).clamp(-128, 127) << 8) as i16;
        }
    }
    Wave::new(bits, to as u8, pcm)
}

/// The sample made `bits` deep and `channels` channels.
pub fn set_format(s: &mut Sample, bits: u8, channels: u8) {
    s.wave = Arc::new(convert(&s.wave, channels, bits));
}

/// Every point in a range changed by `f(point, how far through the range, 0 to 1)`.
fn each(s: &mut Sample, a: usize, b: usize, f: impl Fn(f32, f32) -> f32) {
    let (a, b) = span(s, a, b);
    let c = ch(s);
    let eight = s.wave.bits == 8;
    let len = (b - a).max(1) as f32;
    let w = Arc::make_mut(&mut s.wave);
    for i in a..b {
        let t = (i - a) as f32 / len;
        for k in 0..c {
            let v = &mut w.pcm[i * c + k];
            let y = f(f32::from(*v), t).round().clamp(-32768.0, 32767.0) as i32;
            *v = if eight { ((y >> 8) << 8) as i16 } else { y as i16 };
        }
    }
}

pub fn silence(s: &mut Sample, a: usize, b: usize) {
    each(s, a, b, |_, _| 0.0);
}

/// A fade in (from silence) or out (to it), over the range.
pub fn fade(s: &mut Sample, a: usize, b: usize, fade_in: bool) {
    each(s, a, b, |v, t| v * if fade_in { t } else { 1.0 - t });
}

/// The range louder or quieter, `percent` of what it was.
pub fn gain(s: &mut Sample, a: usize, b: usize, percent: f32) {
    let k = percent / 100.0;
    each(s, a, b, |v, _| v * k);
}

/// The range brought to full scale.
pub fn normalize(s: &mut Sample, a: usize, b: usize) {
    let (a, b) = span(s, a, b);
    let c = ch(s);
    let peak = s.wave.pcm[a * c..b * c].iter().map(|v| i32::from(*v).abs()).max().unwrap_or(0);
    if peak > 0 {
        let k = if s.wave.bits == 8 { 32512.0 } else { 32767.0 } / peak as f32;
        each(s, a, b, |v, _| v * k);
    }
}

/// The range played backwards.
pub fn reverse(s: &mut Sample, a: usize, b: usize) {
    let (a, b) = span(s, a, b);
    let c = ch(s);
    let w = Arc::make_mut(&mut s.wave);
    let frames: Vec<Vec<i16>> = w.pcm[a * c..b * c].chunks(c).rev().map(|f| f.to_vec()).collect();
    w.pcm.splice(a * c..b * c, frames.concat());
}

/// The DC offset (each channel's average) taken out of the range.
pub fn remove_dc(s: &mut Sample, a: usize, b: usize) {
    let (a, b) = span(s, a, b);
    let c = ch(s);
    if b <= a {
        return;
    }
    let mut mean = vec![0f64; c];
    for i in a..b {
        for (k, m) in mean.iter_mut().enumerate() {
            *m += f64::from(s.wave.pcm[i * c + k]);
        }
    }
    let mean: Vec<f32> = mean.iter().map(|m| (m / (b - a) as f64) as f32).collect();
    let w = Arc::make_mut(&mut s.wave);
    for i in a..b {
        for (k, m) in mean.iter().enumerate() {
            let v = &mut w.pcm[i * c + k];
            *v = (f32::from(*v) - m).round().clamp(-32768.0, 32767.0) as i16;
        }
    }
}

/// Points drawn by hand from `at`: one value a frame, to every channel.
pub fn draw(s: &mut Sample, at: usize, points: &[i16]) {
    let c = ch(s);
    let n = s.frames();
    let w = Arc::make_mut(&mut s.wave);
    for (i, &v) in points.iter().enumerate() {
        if at + i >= n {
            break;
        }
        for k in 0..c {
            w.pcm[(at + i) * c + k] = v;
        }
    }
}

/// The sample read at another rate: as many frames as `rate` needs for
/// the same sound, its C-5 rate made `rate` so it plays the same pitch,
/// its loops moved with it. (Cubic interpolation.)
pub fn resample(s: &mut Sample, rate: u32) {
    let rate = rate.clamp(1000, 192_000);
    if s.c5 == 0 || s.frames() == 0 || rate == s.c5 {
        s.c5 = rate;
        return;
    }
    let k = f64::from(rate) / f64::from(s.c5);
    let c = ch(s);
    let n = s.frames();
    let out_n = ((n as f64) * k).round().max(1.0) as usize;
    let w = &s.wave;
    let at = |i: isize, k: usize| -> f32 { f32::from(w.pcm[(i.clamp(0, n as isize - 1) as usize) * c + k]) };
    let mut pcm = Vec::with_capacity(out_n * c);
    for j in 0..out_n {
        let x = j as f64 / k;
        let i = x.floor() as isize;
        let t = (x - x.floor()) as f32;
        for kk in 0..c {
            let (s0, s1, s2, s3) = (at(i - 1, kk), at(i, kk), at(i + 1, kk), at(i + 2, kk));
            let a = -0.5 * s0 + 1.5 * s1 - 1.5 * s2 + 0.5 * s3;
            let b = s0 - 2.5 * s1 + 2.0 * s2 - 0.5 * s3;
            let cc = -0.5 * s0 + 0.5 * s2;
            let y = ((a * t + b) * t + cc) * t + s1;
            pcm.push(y.round().clamp(-32768.0, 32767.0) as i16);
        }
    }
    let bits = s.wave.bits;
    s.wave = Arc::new(Wave::new(bits, c as u8, pcm));
    let scale = |x: u32| ((f64::from(x) * k).round() as u32).min(out_n as u32);
    s.looped = Loop { start: scale(s.looped.start), end: scale(s.looped.end), ..s.looped }.clamped(out_n as u32);
    s.sustain = Loop { start: scale(s.sustain.start), end: scale(s.sustain.end), ..s.sustain }.clamped(out_n as u32);
    s.slices = s.slices.iter().map(|&x| scale(x)).collect();
    s.c5 = rate;
}

/// The loop's end blended into what's before its start over `len`
/// frames, so it comes round without a click (OpenMPT's crossfade).
pub fn crossfade_loop(s: &mut Sample, len: usize) {
    let lp = s.looped;
    if !lp.on() {
        return;
    }
    let (start, end) = (lp.start as usize, lp.end as usize);
    let len = len.min(start).min(end - start);
    if len == 0 {
        return;
    }
    let c = ch(s);
    let w = Arc::make_mut(&mut s.wave);
    for i in 0..len {
        // The last `len` frames of the loop fade into the `len` before its start.
        let t = (i + 1) as f32 / (len + 1) as f32;
        for k in 0..c {
            let tail = f32::from(w.pcm[(end - len + i) * c + k]);
            let lead = f32::from(w.pcm[(start - len + i) * c + k]);
            // Equal power.
            let y = tail * (1.0 - t).sqrt() + lead * t.sqrt();
            w.pcm[(end - len + i) * c + k] = y.round().clamp(-32768.0, 32767.0) as i16;
        }
    }
}

/// A loop end near the one it has where the sound comes round most
/// smoothly: within `reach` frames, the place whose few points best
/// match what's at the loop's start (in value and slope). Returns the
/// end, or None when there's no loop.
pub fn smooth_loop_end(s: &Sample, reach: usize) -> Option<u32> {
    let lp = s.looped;
    if !lp.on() {
        return None;
    }
    let c = ch(s);
    let n = s.frames();
    let w = &s.wave.pcm;
    let mono = |i: usize| -> f32 { (0..c).map(|k| f32::from(w[i.min(n - 1) * c + k])).sum::<f32>() / c as f32 };
    let start = lp.start as usize;
    const WIN: isize = 8;
    // What plays round the end should look like what's round the start.
    let cost = |end: usize| -> f32 {
        (-WIN..WIN)
            .filter(|&k| start as isize + k >= 0)
            .map(|k| (mono((end as isize + k).max(0) as usize) - mono((start as isize + k) as usize)).abs())
            .sum()
    };
    let lo = (lp.end as usize).saturating_sub(reach).max(start + 2);
    let hi = (lp.end as usize + reach).min(n);
    (lo..=hi).filter(|&e| e > start + 1 && e <= n).min_by(|&a, &b| cost(a).partial_cmp(&cost(b)).unwrap_or(std::cmp::Ordering::Equal)).map(|e| e as u32)
}

/// The sample made `ratio` times as long without its pitch changing
/// (overlap-add of windowed grains, each placed where it best matches
/// what's already there: WSOLA). Loops and slices scale with it.
pub fn time_stretch(s: &mut Sample, ratio: f32) {
    let ratio = ratio.clamp(0.25, 4.0);
    let n = s.frames();
    if n < 64 || (ratio - 1.0).abs() < 1e-3 {
        return;
    }
    let c = ch(s);
    // Grains of about 40 ms at the sample's rate.
    let grain = ((s.c5 as f32 * 0.04) as usize).clamp(64, 8192) & !1;
    let hop_out = grain / 2;
    let hop_in = hop_out as f32 / ratio;
    let tolerance = grain / 4;
    let out_n = (n as f32 * ratio) as usize;
    let mono: Vec<f32> = s.wave.pcm.chunks(c).map(|f| f.iter().map(|&v| f32::from(v)).sum::<f32>() / c as f32).collect();
    let window: Vec<f32> = (0..grain).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / grain as f32).cos()).collect();
    let mut acc = vec![0f32; (out_n + grain) * c];
    let mut norm = vec![0f32; out_n + grain];
    let mut prev_at: isize = -1;
    let mut out_at = 0usize;
    let mut k = 0usize;
    while out_at < out_n {
        let nominal = (k as f32 * hop_in) as isize;
        // Where the grain best continues the last one's next half.
        let at = if prev_at < 0 {
            nominal
        } else {
            let follow = prev_at + hop_out as isize;
            let mut best = nominal;
            let mut best_score = f32::MIN;
            let from = (nominal - tolerance as isize).max(0);
            let to = (nominal + tolerance as isize).min(n as isize - 1);
            let mut d = from;
            while d <= to {
                let mut score = 0.0;
                let mut i = 0;
                while i < hop_out {
                    let a = mono.get((follow + i as isize).max(0) as usize).copied().unwrap_or(0.0);
                    let b = mono.get((d + i as isize) as usize).copied().unwrap_or(0.0);
                    score += a * b;
                    i += 4;
                }
                if score > best_score {
                    best_score = score;
                    best = d;
                }
                d += 2;
            }
            best
        };
        for i in 0..grain {
            let src = at + i as isize;
            if src < 0 || src as usize >= n {
                continue;
            }
            let wv = window[i];
            for kk in 0..c {
                acc[(out_at + i) * c + kk] += f32::from(s.wave.pcm[src as usize * c + kk]) * wv;
            }
            norm[out_at + i] += wv;
        }
        prev_at = at;
        out_at += hop_out;
        k += 1;
    }
    let pcm: Vec<i16> = (0..out_n * c).map(|i| (acc[i] / norm[i / c].max(1e-3)).round().clamp(-32768.0, 32767.0) as i16).collect();
    let bits = s.wave.bits;
    s.wave = Arc::new(Wave::new(bits, c as u8, pcm));
    let scale = |x: u32| ((x as f32 * ratio) as u32).min(out_n as u32);
    s.looped = Loop { start: scale(s.looped.start), end: scale(s.looped.end), ..s.looped }.clamped(out_n as u32);
    s.sustain = Loop { start: scale(s.sustain.start), end: scale(s.sustain.end), ..s.sustain }.clamped(out_n as u32);
    s.slices = s.slices.iter().map(|&x| scale(x)).collect();
}

/// The pitch moved by `semitones` without the length changing: stretched,
/// then read back at the rate that brings it to its length again.
pub fn pitch_shift(s: &mut Sample, semitones: f32) {
    let k = 2f32.powf(semitones / 12.0);
    let rate = s.c5;
    let n = s.frames();
    time_stretch(s, k);
    // Back to its length: a rate `k` times lower, played at the old one.
    let target = (s.c5 as f32 / k).round() as u32;
    resample(s, target);
    s.c5 = rate;
    // Rounding can leave a frame or so over.
    let c = ch(s);
    Arc::make_mut(&mut s.wave).pcm.truncate(n * c);
    let frames = s.frames() as u32;
    s.looped = s.looped.clamped(frames);
    s.sustain = s.sustain.clamped(frames);
}

/// Where the beats are: each place the sound's energy (in 10 ms
/// windows) jumps over its recent average, by more the lower
/// `sensitivity` (0 to 1) is, at least 50 ms apart. The first is
/// always 0.
pub fn find_slices(s: &Sample, sensitivity: f32) -> Vec<u32> {
    let n = s.frames();
    let c = ch(s);
    let win = ((s.c5 as f32 * 0.01) as usize).clamp(32, 2048);
    let gap = (s.c5 as f32 * 0.05) as usize;
    let energy: Vec<f32> = (0..n / win)
        .map(|w| {
            let mut e = 0.0;
            for i in w * win..(w + 1) * win {
                for k in 0..c {
                    let v = f32::from(s.wave.pcm[i * c + k]) / 32768.0;
                    e += v * v;
                }
            }
            (e / (win * c) as f32).sqrt()
        })
        .collect();
    let threshold = 1.2 + (1.0 - sensitivity.clamp(0.0, 1.0)) * 3.0;
    let mut out = vec![0u32];
    let mut avg = energy.first().copied().unwrap_or(0.0);
    for (w, &e) in energy.iter().enumerate().skip(1) {
        let at = w * win;
        if e > avg * threshold && e > 0.02 && at - *out.last().unwrap() as usize >= gap {
            out.push(at as u32);
        }
        avg = avg * 0.8 + e * 0.2;
    }
    out
}

/// A sample's slices made samples of their own (added at the end of the
/// list) and an instrument that plays them a key each up from `first`
/// (as Renoise's slices play). Returns the new instrument's index.
pub fn slices_to_instrument(m: &mut Module, sample: usize, first: u8) -> Result<usize, String> {
    let s = m.samples.get(sample).ok_or("There's no such sample.")?.clone();
    let mut cuts = s.slices.clone();
    cuts.retain(|&x| (x as usize) < s.frames());
    if !cuts.contains(&0) {
        cuts.insert(0, 0);
    }
    cuts.sort_unstable();
    cuts.dedup();
    if cuts.len() < 2 {
        return Err("Slice it first: it has no slices.".into());
    }
    if m.samples.len() + cuts.len() > 4000 {
        return Err("That's too many samples for one song.".into());
    }
    let mut keymap: Vec<(u8, u16)> = (0..120).map(|n| (n as u8, 0)).collect();
    for (k, w) in cuts.iter().enumerate() {
        let end = cuts.get(k + 1).map_or(s.frames(), |&e| e as usize);
        let mut part = Sample { name: format!("{} {}", s.name, k + 1).chars().take(32).collect(), wave: Arc::new(copy(&s, *w as usize, end)), looped: Loop::default(), sustain: Loop::default(), slices: Vec::new(), synth: None, ..s.clone() };
        part.name = part.name.trim().to_string();
        m.samples.push(part);
        let key = usize::from(first.saturating_sub(1)) + k;
        if key < 120 {
            // Each key plays its slice at the sample's own pitch (as C-5).
            keymap[key] = (60, m.samples.len() as u16);
        }
    }
    if !m.flags.instruments {
        m.instruments = m.samples.iter().enumerate().take(m.samples.len() - cuts.len()).map(|(i, x)| Instrument::for_sample(&x.name, (i + 1) as u16)).collect();
        m.flags.instruments = true;
    }
    let mut ins = Instrument::for_sample(&format!("{} (sliced)", s.name).chars().take(32).collect::<String>(), 0);
    ins.keymap = keymap;
    m.instruments.push(ins);
    Ok(m.instruments.len() - 1)
}

/// The rate a synth's cycle plays middle C at: its cycle is this many
/// frames, so C-5 (261.63 Hz) is `CYCLE` × that.
pub const SYNTH_CYCLE: usize = neumetik_synth::cycle::CYCLE;

/// A synth's waveform (made by neumetik-synth: `cycle::render`) as a
/// sample, looped, at the rate that makes it middle C at C-5.
pub fn synth(y: &Synth) -> Sample {
    let pts = neumetik_synth::cycle::render(y);
    let len = pts.len();
    let pcm: Vec<i16> = pts.iter().map(|v| (v * 30_000.0) as i16).collect();
    Sample {
        name: format!("Synth: {}", neumetik_synth::cycle::NAMES.get(usize::from(y.wave)).unwrap_or(&"Sine")),
        wave: Arc::new(Wave::new(16, 1, pcm)),
        c5: (261.6256 * SYNTH_CYCLE as f64).round() as u32,
        looped: Loop::forward(0, len as u32),
        synth: Some(*y),
        ..Sample::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Sample {
        Sample { wave: Arc::new(Wave::new(16, 1, (0..n).map(|i| (i * 10) as i16).collect())), looped: Loop::forward(50, 90), c5: 8000, ..Sample::empty() }
    }

    /// The pitch heard: cycles a frame, by upward zero crossings.
    fn cycles(s: &Sample) -> f32 {
        let p = &s.wave.pcm;
        let ups = p.windows(2).filter(|w| w[0] < 0 && w[1] >= 0).count();
        ups as f32 / p.len() as f32
    }

    #[test]
    fn cut_paste_and_crop_keep_the_loop_where_the_sound_is() {
        let mut s = ramp(100);
        cut(&mut s, 10, 20);
        assert_eq!(s.frames(), 90);
        assert_eq!((s.looped.start, s.looped.end), (40, 80));
        assert_eq!(s.wave.pcm[10], 200);
        let w = Wave::new(16, 1, vec![7; 5]);
        paste(&mut s, 0, &w);
        assert_eq!((s.looped.start, s.looped.end), (45, 85));
        crop(&mut s, 45, 85);
        assert_eq!(s.frames(), 40);
        assert_eq!((s.looped.start, s.looped.end), (0, 40));
        let mut s = ramp(10);
        set_format(&mut s, 8, 2);
        assert_eq!((s.wave.channels, s.wave.bits, s.frames()), (2, 8, 10));
        assert!(s.wave.pcm.iter().all(|v| v & 0xFF == 0));
    }

    #[test]
    fn levels_and_dc() {
        let mut s = Sample { wave: Arc::new(Wave::new(16, 1, vec![1000; 100])), ..Sample::empty() };
        remove_dc(&mut s, 0, 100);
        assert!(s.wave.pcm.iter().all(|&v| v == 0));
        let mut s = ramp(100);
        fade(&mut s, 0, 100, false);
        assert_eq!(s.wave.pcm[0], 0);
        assert!(s.wave.pcm[99] < 20);
        gain(&mut s, 0, 100, 0.0);
        assert!(s.wave.pcm.iter().all(|&v| v == 0));
    }

    #[test]
    fn resampling_and_stretching_keep_pitch_and_length_as_they_should() {
        // A 100 Hz sine at 8 kHz, 1 second.
        let sine = |n: usize| Sample { wave: Arc::new(Wave::new(16, 1, (0..n).map(|i| ((i as f32 * 100.0 / 8000.0 * std::f32::consts::TAU).sin() * 20000.0) as i16).collect())), c5: 8000, ..Sample::empty() };
        let mut s = sine(8000);
        let before = cycles(&s);
        resample(&mut s, 16_000);
        assert_eq!((s.frames(), s.c5), (16_000, 16_000));
        assert!((cycles(&s) * 2.0 - before).abs() < 0.0005);
        let mut s = sine(8000);
        time_stretch(&mut s, 1.5);
        assert!((s.frames() as i64 - 12_000).abs() < 2);
        assert!((cycles(&s) - before).abs() < 0.001, "{} {}", cycles(&s), before);
        let mut s = sine(8000);
        pitch_shift(&mut s, 12.0);
        assert_eq!(s.frames(), 8000);
        assert!((cycles(&s) / before - 2.0).abs() < 0.1, "{}", cycles(&s) / before);
    }

    #[test]
    fn slices_find_the_beats_and_make_an_instrument() {
        // Four clicks, a quarter second apart, at 8 kHz.
        let mut pcm = vec![0i16; 8000];
        for b in 0..4 {
            for i in 0..400 {
                pcm[b * 2000 + i] = if i % 2 == 0 { 20000 } else { -20000 };
            }
        }
        let mut s = Sample { wave: Arc::new(Wave::new(16, 1, pcm)), c5: 8000, ..Sample::empty() };
        s.slices = find_slices(&s, 0.5);
        assert_eq!(s.slices.len(), 4, "{:?}", s.slices);
        assert!(s.slices[1].abs_diff(2000) < 100);
        let mut m = Module::new(1);
        m.samples = vec![s];
        let i = slices_to_instrument(&mut m, 0, 61).unwrap();
        assert_eq!(m.samples.len(), 5);
        assert_eq!(m.instruments[i].map(62), (61, 3));
        assert!(m.flags.instruments);
    }

    #[test]
    fn a_synth_is_middle_c_and_loops() {
        for wave in 0..6 {
            let s = synth(&Synth { wave, width: 50, ratio: 2, depth: 40, beat: 0, soften: 0 });
            assert!(s.looped.on() && s.frames() > 0);
            if wave != 4 {
                assert_eq!(s.frames(), SYNTH_CYCLE);
                assert!((f64::from(s.c5) / SYNTH_CYCLE as f64 - 261.63).abs() < 0.1);
            }
        }
        let s = synth(&Synth { wave: 1, beat: 3, ..Default::default() });
        assert_eq!(s.frames(), SYNTH_CYCLE * 12);
    }

    #[test]
    fn a_smooth_loop_end_is_found_where_the_wave_matches() {
        // A sine of period 100: the loop from 0 should end on a period.
        let pcm: Vec<i16> = (0..1000).map(|i| ((i as f32 / 100.0 * std::f32::consts::TAU).sin() * 20000.0) as i16).collect();
        let s = Sample { wave: Arc::new(Wave::new(16, 1, pcm)), looped: Loop::forward(100, 480), ..Sample::empty() };
        let e = smooth_loop_end(&s, 40).unwrap();
        assert_eq!(e % 100, 0, "{e}");
    }
}
