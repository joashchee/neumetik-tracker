//! Musicline Editor (.ml, "MLEDMODL"): chunks of tunes (each channel
//! its own sequence of parts), parts, arpeggio tables, instruments and
//! samples (delta-packed or not), read in order as the replayer reads
//! them, length fields or not. The song plays by its own replayer,
//! neumetik-synth's (`neumetik_synth::musicline`), since each channel
//! keeps its own time; the patterns lay the song out as it plays, for
//! the screen, from a run of that replayer. Described by NostalgicPlayer's
//! player (MIT), read for the format, not copied.

use std::collections::HashMap;
use std::sync::Arc;

use super::common::amiga_pan;
use crate::io::{bad, Reader, Result};
use crate::model::*;
use neumetik_synth::musicline::{self as ml, Machine, RowRead};

pub fn probe_ml(d: &[u8]) -> bool {
    d.len() >= 20 && &d[..8] == b"MLEDMODL"
}

/// Reads a run of zero-ended strings' next one.
fn cstring(r: &mut Reader) -> Result<String> {
    let mut s = Vec::new();
    loop {
        let c = r.u8()?;
        if c == 0 {
            break;
        }
        s.push(c);
    }
    Ok(crate::io::text(&s))
}

/// The replayer's delta packing: a run starts at the command byte with
/// its first point and a count, each byte after two 4-bit steps.
fn unpack(packed: &[u8], command: u8, out_len: usize) -> Vec<i8> {
    let mut out = Vec::with_capacity(out_len);
    let mut i = 0;
    while i < packed.len() && out.len() < out_len {
        let b = packed[i];
        i += 1;
        if b != command {
            out.push(b as i8);
            continue;
        }
        let Some(h) = packed.get(i..i + 3) else { break };
        i += 3;
        let mut x = h[0] as i8;
        let mut n = usize::from(h[1]) << 8 | usize::from(h[2]);
        out.push(x);
        let step = |v: u8| ((v << 4) as i8) >> 4;
        while n > 0 && i < packed.len() {
            let d = packed[i];
            i += 1;
            x = x.wrapping_add(step(d >> 4));
            out.push(x);
            n -= 1;
            if n > 0 {
                x = x.wrapping_add(step(d & 0x0F));
                out.push(x);
                n -= 1;
            }
        }
    }
    out.resize(out_len, 0);
    out
}

/// The song, as the replayer reads it; its title, author and comments.
fn read_song(d: &[u8]) -> Result<(ml::Song, Vec<String>)> {
    let mut r = Reader::at(d, 8);
    // Some old files have no length after MODL and the VERS chunk next.
    let len = r.u32be()?;
    if len != u32::from_be_bytes(*b"VERS") {
        r.skip(len as usize)?;
        if &r.tag()? != b"VERS" {
            return bad("its version chunk is missing");
        }
    }
    let len = r.u32be()? as usize;
    r.skip(len)?;
    let mut song = ml::Song::new();
    let mut info = Vec::new();
    const TAGS: [&[u8; 4]; 6] = [b"TUNE", b"PART", b"ARPG", b"INST", b"SMPL", b"INFO"];
    while r.left() >= 8 {
        // Some files are a few bytes off between chunks (a part's pad
        // left out, a byte after a sample, a packed sample's length
        // over): the nearest chunk is taken.
        let here = r.len() - r.left();
        let tag_at = |at: usize| TAGS.iter().any(|t| d.get(at..).is_some_and(|x| x.starts_with(&t[..])));
        if !tag_at(here) {
            if let Some(at) = (1..=8).flat_map(|k| [here + k, here.wrapping_sub(k)]).find(|&at| at < d.len() && tag_at(at)) {
                r.seek(at)?;
            }
        }
        let tag = r.tag()?;
        let len = r.u32be()? as usize;
        match &tag {
            b"TUNE" => {
                let title = r.text(32)?;
                let tempo = r.u16be()?;
                let speed = r.u8()?;
                let groove = r.u8()?;
                let volume = r.u16be()?;
                let mixer = r.u8()? != 0;
                let nch = usize::from(r.u8()?);
                // A tune of no channels is passed over (Fletch's dances have many).
                if nch == 0 {
                    continue;
                }
                let sizes: Vec<usize> = (0..nch).map(|_| r.u32be().map(|v| v as usize)).collect::<Result<_>>()?;
                let mut sequences = Vec::with_capacity(nch);
                for size in sizes {
                    let words = r.bytes(size)?;
                    let mut seq: Vec<u16> = words.chunks_exact(2).take(256).map(|w| u16::from_be_bytes([w[0], w[1]])).collect();
                    // Past its end, a sequence goes round part 0 (the
                    // replayer's own; NostalgicPlayer stops it instead).
                    seq.resize(256, 0x0010);
                    sequences.push(seq);
                }
                song.tunes.push(ml::Tune { title, tempo, speed, groove, volume, mixer, sequences });
            }
            b"PART" => {
                let num = usize::from(r.u16be()?);
                let mut part = ml::Part::default();
                let mut line = 0;
                loop {
                    let flag = r.u8()?;
                    if flag == 0xFF {
                        break;
                    }
                    if line == 128 {
                        return bad("a part is damaged");
                    }
                    let l = &mut part.lines[line];
                    line += 1;
                    if flag & 1 != 0 {
                        l.note = r.u8()?;
                        l.instr = r.u8()?;
                    }
                    for i in 0..5 {
                        if flag & (2 << i) != 0 {
                            l.fx[i] = (r.u8()?, r.u8()?);
                        }
                    }
                }
                if let Some(p) = song.parts.get_mut(num) {
                    *p = part;
                }
                // Parts end on an even byte (most of them).
                let at = r.len() - r.left();
                if at & 1 != 0 && !TAGS.iter().any(|t| d[at..].starts_with(&t[..])) {
                    r.skip(1)?;
                }
            }
            b"ARPG" => {
                let num = usize::from(r.u16be()?);
                let body = r.bytes(len.saturating_sub(2) / 6 * 6)?;
                let mut arp = ml::Arpeggio::default();
                for (l, b) in arp.lines.iter_mut().zip(body.chunks_exact(6)) {
                    *l = ml::ArpLine { transpose: b[0] as i8, sample: b[1], fx: [(b[2], b[3]), (b[4], b[5])] };
                }
                if let Some(a) = song.arpeggios.get_mut(num) {
                    *a = arp;
                }
            }
            b"INST" => {
                let raw = r.bytes(ml::INSTRUMENT_BYTES)?;
                if song.instruments.len() < 256 {
                    song.instruments.push(ml::Instrument::new(raw, false));
                }
            }
            b"SMPL" => {
                let original = r.u32be()? as usize;
                let command = r.u8()?;
                r.skip(1)?;
                let name = r.text(32)?;
                r.skip(1)?;
                let kind = r.u8()?;
                let start = r.u32be()?;
                let mut length = r.u16be()?;
                let repeat = r.u32be()?;
                let mut repeat_length = r.u16be()?;
                let finetune = r.i16be()?;
                let semitone = r.i16be()?;
                let stored = len.checked_sub(50).ok_or_else(|| crate::io::Error::Bad("a sample is damaged".into()))?;
                if original > 4 << 20 {
                    return bad("a sample is damaged");
                }
                let body = r.bytes_upto(stored);
                let mut data: Vec<i8> = if stored != original { unpack(body, command, original) } else { body.iter().map(|&b| b as i8).collect() };
                data.resize(original, 0);
                if original == 256 {
                    data = ml::Wave::octaves(&data);
                }
                let number = song.samples.len() as u8;
                // The instruments read so far that name it take it.
                for inst in song.instruments.iter_mut().skip(1) {
                    if inst.sample() == number {
                        inst.bound = true;
                    }
                }
                let offset = repeat.saturating_sub(start).min(i32::MAX as u32);
                if usize::from(length) * 2 > data.len() {
                    length = (data.len() / 2) as u16;
                }
                if offset as usize + usize::from(repeat_length) * 2 > data.len() {
                    repeat_length = (data.len().saturating_sub(offset as usize) / 2) as u16;
                }
                if song.samples.len() < 256 {
                    song.samples.push(ml::Wave { name, kind, length, repeat: offset, repeat_length, finetune, semitone, data, stored: original as u32 });
                }
            }
            b"INFO" => {
                // A cut-off file keeps what's there of it.
                for _ in 0..9 {
                    match cstring(&mut r) {
                        Ok(s) => info.push(s),
                        Err(_) => break,
                    }
                }
            }
            _ => return bad("it has a chunk Musicline doesn't"),
        }
    }
    if song.tunes.is_empty() {
        return bad("it has no tunes");
    }
    Ok((song, info))
}

pub fn load_ml(d: &[u8]) -> Result<Module> {
    let (song, info) = read_song(d)?;
    let nch = ml::channels_of(&song).max(1);
    let mut m = Module {
        format: "ml".into(),
        format_name: "Musicline Editor".into(),
        tracker: "Musicline Editor".into(),
        kind: Kind::Mod,
        speed: song.tunes[0].speed.max(1),
        tempo: song.tunes[0].tempo.clamp(32, 255),
        global_volume: 128,
        mix_volume: 32,
        separation: 128,
        channels: (0..nch).map(|i| Channel { pan: amiga_pan(i), ..Default::default() }).collect(),
        ..Default::default()
    };
    m.title = info.first().cloned().filter(|s| !s.is_empty()).unwrap_or_else(|| song.tunes[0].title.clone());
    m.artist = info.get(1).cloned().unwrap_or_default();
    let comments: Vec<&str> = info.iter().skip(4).map(|s| s.as_str()).collect();
    m.message = comments.join("\n").trim_end().to_string();
    m.samples = song.samples.iter().skip(1).map(module_sample).collect();
    m.instruments = song
        .instruments
        .iter()
        .skip(1)
        .map(|inst| {
            let s = u16::from(inst.sample());
            let s = if usize::from(s) <= m.samples.len() { s } else { 0 };
            Instrument { name: inst.name(), keymap: (0..120).map(|n| (n as u8, s)).collect(), global_volume: 128, ..Default::default() }
        })
        .collect();
    m.flags.instruments = true;
    let song = Arc::new(song);
    let mut sequences = Vec::new();
    for t in 0..song.tunes.len() {
        let seq = lay_out(&song, t, nch, &mut m.patterns);
        sequences.push(Sequence { name: song.tunes[t].title.clone(), ..seq });
    }
    m.orders = sequences[0].orders.clone();
    m.restart = sequences[0].restart;
    if sequences.len() > 1 {
        m.sequences = sequences;
    }
    m.musicline = Some(song);
    Ok(m)
}

/// A sample as the tracker keeps it: a wave's first 256 points, a
/// sample's whole; its loop only where it ends at the sample's end, so
/// the repository's cut at a loop's end takes nothing the replayer reads.
fn module_sample(w: &ml::Wave) -> Sample {
    let pts = w.points();
    let frames = pts.len() as u32;
    let looped = if w.kind != ml::SAMPLE && frames == 256 {
        Loop::forward(0, 256)
    } else if w.repeat_length > 0 && w.repeat + u32::from(w.repeat_length) * 2 == frames {
        Loop::forward(w.repeat, frames)
    } else {
        Loop::default()
    };
    // C-5 is the replayer's C-4, period 424, tuned by the sample.
    let c5 = 3_546_895.0 / f64::from(ml::tables::period(37 * 32)) * 2f64.powf((f64::from(w.semitone) * 32.0 + f64::from(w.finetune)) / 384.0);
    Sample { name: w.name.clone(), wave: Arc::new(Wave::new(8, 1, pts.iter().map(|&x| i16::from(x) << 8).collect())), looped, c5: c5.round().clamp(1.0, 2_000_000.0) as u32, ..Sample::empty() }
}

/// The longest a song is laid out for.
const MOST_SECONDS: f64 = 30.0 * 60.0;

/// A tune laid out in patterns as it plays: a row wherever any channel
/// reads one, each row as long as the ticks to the next (its speed), a
/// new pattern where the first channel starts a part. Its restart is
/// where the song comes round to (a channel's loop back, the longest
/// of them), or its start when every channel stops.
fn lay_out(song: &ml::Song, tune: usize, nch: usize, patterns: &mut Vec<Arc<Pattern>>) -> Sequence {
    let mut mach = Machine::new(song, tune);
    let chans = mach.channels().min(nch);
    let mut rows: Vec<(u64, Vec<Option<RowRead>>)> = Vec::new();
    let mut seen: Vec<HashMap<u8, u64>> = vec![HashMap::new(); chans];
    let mut loops: Vec<Option<u64>> = vec![None; chans];
    let mut ended = vec![false; chans];
    let (mut t, mut secs) = (0u64, 0.0);
    loop {
        mach.tick(song);
        let reads: Vec<Option<RowRead>> = (0..chans).map(|c| mach.row(c)).collect();
        for c in 0..chans {
            if let Some(r) = reads[c] {
                if !ended[c] && mach.looped(c) && loops[c].is_none() {
                    loops[c] = Some(t - seen[c].get(&r.position).copied().unwrap_or(0));
                }
                seen[c].entry(r.position).or_insert(t);
            }
            ended[c] = mach.reached_end(c);
        }
        if t == 0 || reads.iter().any(Option::is_some) {
            rows.push((t, reads));
        }
        // The tick it comes round on (or stops) starts it over: the song ends before it.
        if t > 0 && (!mach.playing() || (0..chans).all(|c| mach.reached_end(c)) || secs > MOST_SECONDS) {
            break;
        }
        t += 1;
        secs += mach.tick_seconds();
    }
    let end = t;
    rows.retain(|r| r.0 < end);
    let stops = !mach.playing() || (0..chans).all(|c| mach.stopped(c));
    let restart_tick = if stops { 0 } else { end.saturating_sub(loops.iter().flatten().copied().max().unwrap_or(end)) };
    // A row at the restart, and none longer than a speed can say.
    if !rows.iter().any(|r| r.0 == restart_tick) {
        let at = rows.partition_point(|r| r.0 < restart_tick);
        rows.insert(at, (restart_tick, vec![None; chans]));
    }
    let mut i = 0;
    while i < rows.len() {
        let next = rows.get(i + 1).map_or(end, |r| r.0);
        if next - rows[i].0 > 255 {
            rows.insert(i + 1, (rows[i].0 + 255, vec![None; chans]));
        }
        i += 1;
    }
    let lead = (0..chans).find(|&c| rows.iter().any(|r| r.1[c].is_some_and(|x| !x.wait))).unwrap_or(0);
    let mut orders = Vec::new();
    let mut restart = 0u16;
    let mut instr_now = vec![0u8; chans];
    let mut speed_now = 0u64;
    let mut start = 0;
    while start < rows.len() {
        // Where this pattern ends: the lead's next part, the restart, or 256 rows.
        let mut stop = start + 1;
        while stop < rows.len() && stop - start < 256 && rows[stop].0 != restart_tick && !rows[stop].1[lead].is_some_and(|r| !r.wait && r.line == 0) {
            stop += 1;
        }
        if rows[start].0 == restart_tick {
            restart = orders.len() as u16;
        }
        let mut p = Pattern::new(stop - start, nch);
        for (k, (tick, reads)) in rows[start..stop].iter().enumerate() {
            for (c, read) in reads.iter().enumerate() {
                let Some(r) = read.filter(|r| !r.wait) else { continue };
                let line = song.parts[usize::from(r.part)].lines[usize::from(r.line)];
                let made = cell(song, &line, r.transpose, &mut instr_now[c]);
                if let Some(x) = p.cell_mut(k, c) {
                    *x = made;
                }
            }
            let next = rows.get(start + k + 1).map_or(end, |r| r.0);
            let len = (next - tick).clamp(1, 255);
            if len != speed_now || k == 0 {
                speed_now = len;
                let c = (0..nch).find(|&c| p.cell(k, c).fx == Fx::None).unwrap_or(0);
                if let Some(x) = p.cell_mut(k, c) {
                    x.fx = Fx::Speed;
                    x.param = len as u8;
                }
            }
        }
        orders.push(patterns.len() as u16);
        patterns.push(Arc::new(p));
        start = stop;
    }
    Sequence { name: String::new(), orders, restart }
}

/// A part line as a cell: its note (transposed as the replayer would,
/// the replayer's C-4 as C-5), its instrument, its first effect as near
/// as the tracker's commands come (the replayer plays the line itself).
fn cell(song: &ml::Song, line: &ml::Line, transpose: i8, instr_now: &mut u8) -> Cell {
    let mut c = Cell { instr: line.instr, ..Default::default() };
    if line.instr != 0 {
        *instr_now = line.instr;
    }
    if (1..=60).contains(&line.note) {
        let inst = song.instruments.get(usize::from(*instr_now));
        let t = if line.instr != 0 && inst.is_some_and(|i| i.raw[60] == 0) { 0 } else { i32::from(transpose) };
        c.note = (i32::from(line.note) + t + 24).clamp(i32::from(NOTE_MIN), i32::from(NOTE_MAX)) as u8;
    }
    for &(e, a) in &line.fx {
        let (fx, param) = match e {
            0x01 | 0xE1 => (Fx::PortaUp, a),
            0x02 | 0xE2 => (Fx::PortaDown, a),
            0x03 | 0xE3 => (Fx::TonePorta, a),
            0x08 | 0x09 | 0xEA => (Fx::Vibrato, a),
            0xE8 => (Fx::Tremolo, a),
            0x10 => (Fx::Volume, a.min(64)),
            0x11 | 0xE6 => (Fx::VolSlide, (a.min(15)) << 4),
            0x12 | 0xE7 => (Fx::VolSlide, a.min(15)),
            0x20 => (Fx::ChannelVolume, a.min(64)),
            0x30 => (Fx::GlobalVolume, a.min(64) * 2),
            0x48 => (Fx::Offset, a),
            _ => continue,
        };
        c.fx = fx;
        c.param = param;
        break;
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]) {
        out.extend_from_slice(tag);
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(body);
    }

    /// A song of two channels: a square-wave sample, a note on each at
    /// speed 3 on one, 6 on the other, then a loop back.
    fn file() -> Vec<u8> {
        let mut d = b"MLEDMODL".to_vec();
        d.extend_from_slice(&0u32.to_be_bytes());
        chunk(&mut d, b"VERS", b"\x01\x05");
        let mut tune = vec![0u8; 32];
        tune[..4].copy_from_slice(b"Test");
        tune.extend_from_slice(&[0, 125, 3, 0, 0, 64, 0, 2]);
        tune.extend_from_slice(&4u32.to_be_bytes());
        tune.extend_from_slice(&4u32.to_be_bytes());
        // Channel 0: part 0, then a jump back to it. Channel 1: part 1, the same.
        tune.extend_from_slice(&[0x00, 0x10, 0x00, 0xA0]);
        tune.extend_from_slice(&[0x01, 0x10, 0x00, 0xA0]);
        chunk(&mut d, b"TUNE", &tune);
        for (num, speed) in [(0u16, 3u8), (1, 6)] {
            let mut part = num.to_be_bytes().to_vec();
            // A note and a speed for this channel, then the part's end on line 4.
            part.extend_from_slice(&[0x03, 25, 1, 0x40, speed, 0, 0, 0, 0, 0, 0, 0x01, 61, 0, 0xFF]);
            if part.len() & 1 != 0 {
                part.push(0);
            }
            chunk(&mut d, b"PART", &part);
        }
        let mut inst = vec![0u8; 206];
        inst[..4].copy_from_slice(b"sq  ");
        inst[32] = 1;
        inst[38..40].copy_from_slice(&50u16.to_be_bytes());
        inst[44..46].copy_from_slice(&50u16.to_be_bytes());
        inst[58..60].copy_from_slice(&64u16.to_be_bytes());
        inst[62] = 0x80;
        chunk(&mut d, b"INST", &inst);
        let mut smpl = 100u32.to_be_bytes().to_vec();
        smpl.extend_from_slice(&[0, 0]);
        smpl.extend_from_slice(b"square\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
        smpl.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 50, 0, 0, 0, 0, 0, 50, 0, 0, 0, 0]);
        smpl.extend((0..100).map(|i| if i % 20 < 10 { 100u8 } else { 156 }));
        d.extend_from_slice(b"SMPL");
        d.extend_from_slice(&((smpl.len() - 6) as u32).to_be_bytes());
        d.extend_from_slice(&smpl);
        d
    }

    #[test]
    fn a_song_reads_lays_out_by_channel_and_plays_through_umf() {
        let d = file();
        let m = crate::formats::load(&d, "t.ml").expect("reads");
        assert_eq!((m.format.as_str(), m.channels.len(), m.samples.len(), m.instruments.len()), ("ml", 2, 1, 1));
        // Rows wherever either channel reads one: every 3 ticks.
        let p = &m.patterns[usize::from(m.orders[0])];
        assert_eq!((p.cell(0, 0).note, p.cell(0, 1).note, p.cell(0, 0).fx, p.cell(0, 0).param), (49, 49, Fx::Speed, 3));
        assert!(m.musicline.is_some());
        let audio = crate::player::render(Arc::new(m.clone()), 22_050, 2.0, 0.0);
        let rms = (audio.iter().map(|v| v * v).sum::<f32>() / audio.len() as f32).sqrt();
        assert!(rms > 0.05, "it plays: {rms}");
        // Its samples as the repository keeps them: cut to what's heard.
        use crate::canon::{canonical_wave, sample_id, wave_id};
        let refs: Vec<_> = m.samples.iter().map(|s| Some(crate::umf::SampleRef { sample: sample_id(wave_id(&canonical_wave(s)), s), wave: wave_id(&canonical_wave(s)) })).collect();
        let bytes = crate::umf::write(&m, &refs, &[], &Default::default(), true);
        let back = crate::umf::read(&bytes, &mut |_| None).expect("reads back").module;
        let (x, y) = (back.musicline.as_deref().unwrap(), m.musicline.as_deref().unwrap());
        assert_eq!((&x.tunes, &x.instruments, &x.samples), (&y.tunes, &y.instruments, &y.samples));
        assert!(x.parts == y.parts && x.arpeggios == y.arpeggios);
        assert_eq!(crate::player::render(Arc::new(back), 22_050, 2.0, 0.0), audio);
    }

    #[test]
    fn delta_packed_runs_unpack() {
        // A plain byte, then a run from 10: +1, -1, +7.
        let packed = [5u8, 0xAA, 10, 0, 3, 0x1F, 0x70];
        assert_eq!(unpack(&packed, 0xAA, 5), vec![5, 10, 11, 10, 17]);
    }
}
