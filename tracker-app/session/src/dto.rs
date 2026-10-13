// Copyright 2026 ansiapps. Neumetik Tracker: MIT licensed, see the LICENSE at the repository root.

//! What the app sees of a song and sends back: the song summed up, a
//! pattern, the effect letters, and every edit (`Edit`, `apply`).

use std::sync::Arc;

use neumetik_tracker::edit::{self as ed, Block};
use neumetik_tracker::{self as tk, sample_edit as se, Cell, Fx, Module, Vol};
use serde::{Deserialize, Serialize};

use crate::{Doc, Edited};

// ---- What the app sees ----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDto {
    pub(crate) name: String,
    pub(crate) pan: u8,
    pub(crate) volume: u8,
    pub(crate) muted: bool,
    pub(crate) surround: bool,
    /// The effect slot its sound goes through.
    pub(crate) effect: Option<u8>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternInfo {
    pub(crate) rows: usize,
    pub(crate) name: String,
    /// Its own rows a beat and a measure, and swing, over the song's.
    pub(crate) time: Option<(u16, u16)>,
    pub(crate) swing: Option<u8>,
    pub(crate) lanes: usize,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct EnvDto {
    pub(crate) enabled: bool,
    pub(crate) points: Vec<(u16, i8)>,
    pub(crate) looped: Option<(u8, u8)>,
    pub(crate) sustain: Option<(u8, u8)>,
    pub(crate) carry: bool,
    pub(crate) filter: bool,
    #[serde(default)]
    pub(crate) release: Option<u8>,
}

impl EnvDto {
    fn of(e: &tk::Envelope) -> EnvDto {
        EnvDto { enabled: e.enabled, points: e.points.iter().map(|p| (p.tick, p.value)).collect(), looped: e.looped, sustain: e.sustain, carry: e.carry, filter: e.filter, release: e.release }
    }

    fn to(&self) -> tk::Envelope {
        let mut points: Vec<tk::EnvPoint> = self.points.iter().map(|&(tick, value)| tk::EnvPoint { tick, value }).collect();
        points.sort_by_key(|p| p.tick);
        let n = points.len() as u8;
        let ok = |p: Option<(u8, u8)>| p.filter(|&(a, b)| a <= b && b < n);
        tk::Envelope { enabled: self.enabled && n > 0, looped: ok(self.looped), sustain: ok(self.sustain), carry: self.carry, filter: self.filter, release: self.release.filter(|&r| r < n), points }
    }
}

/// What an automation lane moves: `kind` and its channel or slot (`a`) and parameter (`b`).
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TargetDto {
    pub(crate) kind: String,
    pub(crate) a: u8,
    pub(crate) b: u8,
}

impl TargetDto {
    fn of(t: Option<tk::Target>) -> TargetDto {
        let (kind, a, b) = match t {
            None => ("none", 0, 0),
            Some(tk::Target::GlobalVolume) => ("globalVolume", 0, 0),
            Some(tk::Target::Tempo) => ("tempo", 0, 0),
            Some(tk::Target::ChannelVolume(c)) => ("channelVolume", c, 0),
            Some(tk::Target::ChannelPan(c)) => ("channelPan", c, 0),
            Some(tk::Target::Effect { slot, param }) => ("effect", slot, param),
        };
        TargetDto { kind: kind.into(), a, b }
    }

    fn to(self) -> Option<tk::Target> {
        Some(match self.kind.as_str() {
            "globalVolume" => tk::Target::GlobalVolume,
            "tempo" => tk::Target::Tempo,
            "channelVolume" => tk::Target::ChannelVolume(self.a),
            "channelPan" => tk::Target::ChannelPan(self.a),
            "effect" => tk::Target::Effect { slot: self.a, param: self.b },
            _ => return None,
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct LaneDto {
    pub(crate) target: TargetDto,
    pub(crate) points: Vec<(u16, f32)>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct EffectDto {
    pub(crate) kind: u8,
    pub(crate) name: String,
    pub(crate) params: Vec<f32>,
    pub(crate) mix: f32,
    pub(crate) bypass: bool,
    pub(crate) output: Option<u8>,
}

impl EffectDto {
    fn of(e: &tk::EffectSlot) -> EffectDto {
        EffectDto { kind: e.kind as u8, name: e.name.clone(), params: (0..e.kind.params().len()).map(|i| e.param(i)).collect(), mix: e.mix, bypass: e.bypass, output: e.output }
    }

    fn to(&self) -> tk::EffectSlot {
        let kind = tk::EffectKind::from_u8(self.kind);
        tk::EffectSlot { kind, name: self.name.clone(), params: self.params.iter().map(|p| p.clamp(0.0, 1.0)).collect(), mix: self.mix.clamp(0.0, 1.0), bypass: self.bypass, output: self.output }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "camelCase")]
pub struct MidiOutDto {
    pub(crate) channel: u8,
    pub(crate) program: Option<u8>,
    pub(crate) bank: Option<u16>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct PhraseDto {
    /// Six numbers a cell, as a pattern's.
    pub(crate) cells: Vec<[u8; 6]>,
    pub(crate) looped: bool,
    pub(crate) base: u8,
}

#[derive(Serialize, Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "camelCase")]
pub struct SynthDto {
    pub(crate) wave: u8,
    pub(crate) width: u8,
    pub(crate) ratio: u8,
    pub(crate) depth: u8,
    pub(crate) beat: u8,
    pub(crate) soften: u8,
}

impl SynthDto {
    fn of(y: &tk::Synth) -> SynthDto {
        SynthDto { wave: y.wave, width: y.width, ratio: y.ratio, depth: y.depth, beat: y.beat, soften: y.soften }
    }

    fn to(self) -> tk::Synth {
        tk::Synth { wave: self.wave.min(5), width: self.width.clamp(1, 99), ratio: self.ratio.clamp(1, 16), depth: self.depth.min(100), beat: self.beat.min(64), soften: self.soften.min(100) }
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkDto {
    pub(crate) name: String,
    pub(crate) order: Option<u16>,
    pub(crate) pattern: Option<u16>,
    pub(crate) row: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleDto {
    pub(crate) name: String,
    pub(crate) filename: String,
    pub(crate) frames: usize,
    pub(crate) bits: u8,
    pub(crate) channels: u8,
    pub(crate) c5: u32,
    pub(crate) volume: u8,
    pub(crate) global_volume: u8,
    pub(crate) pan: Option<u8>,
    /// 0 off, 1 forward, 2 ping-pong.
    pub(crate) loop_kind: u8,
    pub(crate) loop_start: u32,
    pub(crate) loop_end: u32,
    pub(crate) sus_kind: u8,
    pub(crate) sus_start: u32,
    pub(crate) sus_end: u32,
    /// Auto-vibrato: kind, speed, depth, sweep.
    pub(crate) vibrato: [u8; 4],
    pub(crate) credit: String,
    pub(crate) slices: Vec<u32>,
    pub(crate) synth: Option<SynthDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentDto {
    pub(crate) name: String,
    pub(crate) filename: String,
    /// For each note, the note played and the 1-based sample.
    pub(crate) keymap: Vec<(u8, u16)>,
    pub(crate) volume_env: EnvDto,
    pub(crate) pan_env: EnvDto,
    pub(crate) pitch_env: EnvDto,
    pub(crate) fadeout: u32,
    pub(crate) global_volume: u8,
    pub(crate) pan: Option<u8>,
    pub(crate) nna: u8,
    pub(crate) dct: u8,
    pub(crate) dca: u8,
    pub(crate) pitch_pan_separation: i8,
    pub(crate) pitch_pan_center: u8,
    pub(crate) random_volume: u8,
    pub(crate) random_pan: u8,
    pub(crate) cutoff: Option<u8>,
    pub(crate) resonance: Option<u8>,
    pub(crate) midi: Option<MidiOutDto>,
    pub(crate) effect: Option<u8>,
    pub(crate) phrase: Option<PhraseDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocSummary {
    pub(crate) name: String,
    /// The file it was opened from or last saved to, where the platform has paths.
    pub(crate) path: Option<String>,
    pub(crate) dirty: bool,
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) message: String,
    pub(crate) format: String,
    pub(crate) format_name: String,
    pub(crate) tracker: String,
    /// Whose effect rules it follows: mod, s3m, xm or it.
    pub(crate) kind: String,
    pub(crate) linear: bool,
    pub(crate) instruments_mode: bool,
    pub(crate) speed: u8,
    pub(crate) tempo: u16,
    pub(crate) global_volume: u8,
    pub(crate) mix_volume: u8,
    pub(crate) restart: u16,
    pub(crate) channels: Vec<ChannelDto>,
    pub(crate) orders: Vec<u16>,
    pub(crate) patterns: Vec<PatternInfo>,
    pub(crate) samples: Vec<SampleDto>,
    pub(crate) instruments: Vec<InstrumentDto>,
    pub(crate) can_undo: bool,
    pub(crate) can_redo: bool,
    /// Samples whose waveforms weren't found (a UMF whose samples aren't
    /// in this repository).
    pub(crate) missing: usize,
    /// 0 classic, 1 alternative, 2 modern.
    pub(crate) tempo_mode: u8,
    pub(crate) rows_per_beat: u16,
    pub(crate) rows_per_measure: u16,
    pub(crate) swing: u8,
    /// Every order list's name (one when there's one), and which is being edited.
    pub(crate) sequences: Vec<String>,
    pub(crate) sequence: usize,
    pub(crate) bookmarks: Vec<BookmarkDto>,
    pub(crate) slot_mutes: Vec<(u16, u16)>,
    pub(crate) effects: Vec<EffectDto>,
    pub(crate) macros: Option<MacrosDto>,
    /// How the song likes to be seen (a visualizer settings line), empty
    /// for none; and whether its cover is its own rather than painted.
    pub(crate) visual: String,
    pub(crate) cover_own: bool,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct MacrosDto {
    pub(crate) parametered: Vec<String>,
    pub(crate) fixed: Vec<String>,
}

fn loop_parts(l: &tk::Loop) -> (u8, u32, u32) {
    let kind = match l.kind {
        tk::LoopKind::Off => 0,
        tk::LoopKind::Forward => 1,
        tk::LoopKind::PingPong => 2,
    };
    (kind, l.start, l.end)
}

pub(crate) fn summary(doc: &Doc) -> DocSummary {
    let m = &doc.module;
    DocSummary {
        name: doc.name.clone(),
        path: doc.path.clone(),
        dirty: doc.dirty,
        title: m.title.clone(),
        artist: m.artist.clone(),
        message: m.message.clone(),
        format: m.format.clone(),
        format_name: m.format_name.clone(),
        tracker: m.tracker.clone(),
        kind: m.kind.name().into(),
        linear: m.flags.linear,
        instruments_mode: m.flags.instruments,
        speed: m.speed,
        tempo: m.tempo,
        global_volume: m.global_volume,
        mix_volume: m.mix_volume,
        restart: m.restart,
        channels: m.channels.iter().map(|c| ChannelDto { name: c.name.clone(), pan: c.pan, volume: c.volume, muted: c.muted, surround: c.surround, effect: c.effect }).collect(),
        orders: m.orders.clone(),
        patterns: m.patterns.iter().map(|p| PatternInfo { rows: p.rows, name: p.name.clone(), time: p.time.map(|t| (t.rows_per_beat, t.rows_per_measure)), swing: p.swing, lanes: p.automation.len() }).collect(),
        samples: m
            .samples
            .iter()
            .map(|s| {
                let (loop_kind, loop_start, loop_end) = loop_parts(&s.looped);
                let (sus_kind, sus_start, sus_end) = loop_parts(&s.sustain);
                SampleDto {
                    name: s.name.clone(),
                    filename: s.filename.clone(),
                    frames: s.frames(),
                    bits: s.wave.bits,
                    channels: s.wave.channels,
                    c5: s.c5,
                    volume: s.volume,
                    global_volume: s.global_volume,
                    pan: s.pan,
                    loop_kind,
                    loop_start,
                    loop_end,
                    sus_kind,
                    sus_start,
                    sus_end,
                    vibrato: [s.vibrato.kind, s.vibrato.speed, s.vibrato.depth, s.vibrato.sweep],
                    credit: s.credit.clone(),
                    slices: s.slices.clone(),
                    synth: s.synth.as_ref().map(SynthDto::of),
                }
            })
            .collect(),
        instruments: m
            .instruments
            .iter()
            .map(|i| InstrumentDto {
                name: i.name.clone(),
                filename: i.filename.clone(),
                keymap: i.keymap.clone(),
                volume_env: EnvDto::of(&i.volume_env),
                pan_env: EnvDto::of(&i.pan_env),
                pitch_env: EnvDto::of(&i.pitch_env),
                fadeout: i.fadeout,
                global_volume: i.global_volume,
                pan: i.pan,
                nna: i.nna as u8,
                dct: i.dct,
                dca: i.dca,
                pitch_pan_separation: i.pitch_pan_separation,
                pitch_pan_center: i.pitch_pan_center,
                random_volume: i.random_volume,
                random_pan: i.random_pan,
                cutoff: i.cutoff,
                resonance: i.resonance,
                midi: i.midi.map(|o| MidiOutDto { channel: o.channel, program: o.program, bank: o.bank }),
                effect: i.effect,
                phrase: i.phrase.as_ref().map(|p| PhraseDto { cells: p.cells.iter().map(cell_bytes).collect(), looped: p.looped, base: p.base }),
            })
            .collect(),
        can_undo: !doc.undo.is_empty(),
        can_redo: !doc.redo.is_empty(),
        missing: doc.missing,
        tempo_mode: m.tempo_mode as u8,
        rows_per_beat: m.time.rows_per_beat,
        rows_per_measure: m.time.rows_per_measure,
        swing: m.swing,
        sequences: m.all_sequences().iter().map(|q| q.name.clone()).collect(),
        sequence: m.sequence.min(m.all_sequences().len().saturating_sub(1)),
        bookmarks: m.bookmarks.iter().map(|b| BookmarkDto { name: b.name.clone(), order: b.order, pattern: b.pattern, row: b.row }).collect(),
        slot_mutes: m.slot_mutes.clone(),
        effects: m.effects.iter().map(EffectDto::of).collect(),
        macros: m.macros.as_ref().map(|x| MacrosDto { parametered: x.parametered.clone(), fixed: x.fixed.clone() }),
        visual: m.visual.clone(),
        cover_own: m.cover.is_some(),
    }
}

/// A pattern for the panel: its cells as six numbers each (note,
/// instrument, volume column kind and value, effect and parameter), row
/// by row.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternDto {
    pub(crate) index: usize,
    pub(crate) rows: usize,
    pub(crate) channels: usize,
    pub(crate) name: String,
    pub(crate) cells: Vec<u8>,
    /// Rows a beat and a measure, its own or the song's.
    pub(crate) rows_per_beat: u16,
    pub(crate) rows_per_measure: u16,
    pub(crate) lanes: Vec<LaneDto>,
}

fn cell_bytes(c: &Cell) -> [u8; 6] {
    [c.note, c.instr, c.vol as u8, c.volp, c.fx as u8, c.param]
}

pub(crate) fn pattern_dto(m: &Module, index: usize) -> Option<PatternDto> {
    let p = m.pattern(index)?;
    let mut cells = Vec::with_capacity(p.cells.len() * 6);
    for c in &p.cells {
        cells.extend_from_slice(&cell_bytes(c));
    }
    let t = m.time_of(index);
    let lanes = p.automation.iter().map(|l| LaneDto { target: TargetDto::of(l.target), points: l.points.clone() }).collect();
    Some(PatternDto { index, rows: p.rows, channels: p.channels, name: p.name.clone(), cells, rows_per_beat: t.rows_per_beat, rows_per_measure: t.rows_per_measure, lanes })
}

pub(crate) fn cell_of(v: &[u8; 6]) -> Cell {
    Cell { note: v[0], instr: v[1], vol: Vol::from_u8(v[2]), volp: v[3], fx: Fx::from_u8(v[4]), param: v[5] }
}

/// The effect letters and volume column letters, for the panel to
/// show and type, and each effect's description.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Letters {
    pub(crate) effects: Vec<(u8, char, &'static str)>,
    pub(crate) volumes: Vec<(u8, char)>,
    pub(crate) formats: Vec<(&'static str, &'static str, Vec<&'static str>)>,
    /// The effect slots' kinds: name, and each parameter's name and default.
    pub(crate) effect_kinds: Vec<(&'static str, Vec<(&'static str, f32)>)>,
    /// Sample files read (extensions) and written (extension, name).
    pub(crate) sample_reads: Vec<&'static str>,
    pub(crate) sample_writes: Vec<(&'static str, &'static str)>,
}

pub fn letters() -> Letters {
    Letters {
        effects: Fx::ALL.iter().filter(|f| **f != Fx::None).map(|f| (*f as u8, f.letter(), f.describe())).collect(),
        volumes: Vol::ALL.iter().filter(|v| **v != Vol::None).map(|v| (*v as u8, v.letter())).collect(),
        formats: tk::formats::FORMATS.iter().map(|f| (f.id, f.name, f.extensions.to_vec())).collect(),
        effect_kinds: tk::EffectKind::ALL.iter().map(|k| (k.name(), k.params().to_vec())).collect(),
        sample_reads: tk::samplefile::EXTENSIONS.to_vec(),
        sample_writes: tk::samplefile::KINDS.to_vec(),
    }
}

// ---- Editing ----

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Edit {
    /// Cells put down from (row, channel), `width` channels a row.
    #[serde(rename_all = "camelCase")]
    Cells { pattern: usize, row: usize, channel: usize, width: usize, cells: Vec<[u8; 6]> },
    /// A block's notes, instruments, volumes and effects (bits 1, 2, 4, 8) cleared.
    #[serde(rename_all = "camelCase")]
    Clear { pattern: usize, row: usize, channel: usize, rows: usize, channels: usize, mask: u8 },
    #[serde(rename_all = "camelCase")]
    Transpose { pattern: usize, row: usize, channel: usize, rows: usize, channels: usize, semitones: i8 },
    /// A block's instruments set to one.
    #[serde(rename_all = "camelCase")]
    SetInstrument { pattern: usize, row: usize, channel: usize, rows: usize, channels: usize, instr: u8 },
    /// Rows put in (or taken out) at `row`, in the channels given; the
    /// rest move down (or up).
    #[serde(rename_all = "camelCase")]
    InsertRows { pattern: usize, row: usize, channel: usize, channels: usize, count: usize },
    #[serde(rename_all = "camelCase")]
    DeleteRows { pattern: usize, row: usize, channel: usize, channels: usize, count: usize },
    #[serde(rename_all = "camelCase")]
    PatternRows { pattern: usize, rows: usize },
    #[serde(rename_all = "camelCase")]
    PatternName { pattern: usize, name: String },
    AddPattern,
    #[serde(rename_all = "camelCase")]
    ClonePattern { pattern: usize },
    Orders { orders: Vec<u16> },
    #[serde(rename_all = "camelCase")]
    Song { title: String, artist: String, message: String, speed: u8, tempo: u16, global_volume: u8, mix_volume: u8, restart: u16, linear: bool },
    #[serde(rename_all = "camelCase")]
    Channel { index: usize, name: String, pan: u8, volume: u8, surround: bool },
    AddChannel,
    RemoveChannel { index: usize },
    #[serde(rename_all = "camelCase")]
    Sample { index: usize, name: String, volume: u8, global_volume: u8, c5: u32, pan: Option<u8>, loop_kind: u8, loop_start: u32, loop_end: u32, sus_kind: u8, sus_start: u32, sus_end: u32, vibrato: [u8; 4], #[serde(default)] credit: Option<String> },
    AddSample,
    ClearSample { index: usize },
    /// The waveform brought to full scale.
    Normalize { index: usize },
    Reverse { index: usize },
    /// The waveform cut to its loop's end.
    TrimToLoop { index: usize },
    #[serde(rename_all = "camelCase")]
    Instrument { index: usize, name: String, global_volume: u8, fadeout: u32, pan: Option<u8>, nna: u8, volume_env: EnvDto, pan_env: EnvDto, pitch_env: EnvDto, #[serde(default)] more: Option<InstrumentMore> },
    /// Every note of an instrument mapped to one sample (1-based; 0 none).
    #[serde(rename_all = "camelCase")]
    KeymapAll { index: usize, sample: u16 },
    /// A range of notes mapped to a sample.
    #[serde(rename_all = "camelCase")]
    KeymapRange { index: usize, from: u8, to: u8, sample: u16 },
    AddInstrument,
    /// The song uses instruments (keymaps, envelopes) or plays samples directly.
    #[serde(rename_all = "camelCase")]
    InstrumentsMode { on: bool },
    /// One note of a keymap: the note it plays and the sample (1-based).
    #[serde(rename_all = "camelCase")]
    KeymapNote { index: usize, note: u8, to_note: u8, sample: u16 },
    /// A block's volumes or effect values from its first row's to its last's.
    #[serde(rename_all = "camelCase")]
    Interpolate { pattern: usize, row: usize, channel: usize, rows: usize, channels: usize, effect: bool },
    /// A block's volumes made `from` percent at its first row to `to` at its last.
    #[serde(rename_all = "camelCase")]
    Amplify { pattern: usize, row: usize, channel: usize, rows: usize, channels: usize, from: f32, to: f32 },
    /// Cells pasted: overwrite, mix, mixCells, flood or insert.
    #[serde(rename_all = "camelCase")]
    Paste { pattern: usize, row: usize, channel: usize, width: usize, cells: Vec<[u8; 6]>, mode: String },
    /// An instrument made another in a block, or (with no pattern) the song.
    #[serde(rename_all = "camelCase")]
    ReplaceInstrument { from: u8, to: u8, pattern: Option<usize>, row: usize, channel: usize, rows: usize, channels: usize },
    #[serde(rename_all = "camelCase")]
    SwapInstruments { a: u8, b: u8 },
    /// A pattern's own rows a beat and a measure, and swing (None: the song's).
    #[serde(rename_all = "camelCase")]
    PatternTime { pattern: usize, time: Option<(u16, u16)>, swing: Option<u8> },
    #[serde(rename_all = "camelCase")]
    Timing { tempo_mode: u8, rows_per_beat: u16, rows_per_measure: u16, swing: u8 },
    /// A new order list, empty or a copy of the one being edited, made the one edited.
    #[serde(rename_all = "camelCase")]
    AddSequence { name: String, copy: bool },
    #[serde(rename_all = "camelCase")]
    RemoveSequence { index: usize },
    #[serde(rename_all = "camelCase")]
    SwitchSequence { index: usize },
    #[serde(rename_all = "camelCase")]
    RenameSequence { index: usize, name: String },
    #[serde(rename_all = "camelCase")]
    Bookmarks { bookmarks: Vec<BookmarkDto> },
    /// A slot of the pattern matrix muted or played.
    #[serde(rename_all = "camelCase")]
    SlotMute { order: u16, channel: u16, muted: bool },
    #[serde(rename_all = "camelCase")]
    Lanes { pattern: usize, lanes: Vec<LaneDto> },
    #[serde(rename_all = "camelCase")]
    Effects { effects: Vec<EffectDto> },
    #[serde(rename_all = "camelCase")]
    ChannelEffect { index: usize, effect: Option<u8> },
    #[serde(rename_all = "camelCase")]
    Macros { macros: Option<MacrosDto> },
    /// A range of a sample's waveform edited: cut, crop, silence, fadeIn,
    /// fadeOut, gain (value in percent), normalize, reverse, dc,
    /// crossfade (value the frames), smoothLoop (value the reach),
    /// stretch (value the ratio), pitch (value semitones), resample
    /// (value the rate), slices (value the sensitivity), clearSlices.
    #[serde(rename_all = "camelCase")]
    SampleEdit { index: usize, edit: String, from: usize, to: usize, value: f32 },
    #[serde(rename_all = "camelCase")]
    SampleFormat { index: usize, bits: u8, channels: u8 },
    #[serde(rename_all = "camelCase")]
    SampleDraw { index: usize, at: usize, points: Vec<i16> },
    #[serde(rename_all = "camelCase")]
    SampleSlices { index: usize, slices: Vec<u32> },
    /// A sample's slices made samples, and an instrument playing them from `first`.
    #[serde(rename_all = "camelCase")]
    SlicesToInstrument { index: usize, first: u8 },
    /// A synth made into the sample.
    #[serde(rename_all = "camelCase")]
    Synth { index: usize, synth: SynthDto },
    /// OpenMPT's Cleanup: what nothing plays taken out.
    #[serde(rename_all = "camelCase")]
    Cleanup { patterns: bool, samples: bool, instruments: bool },
}

/// An instrument's settings beyond its first ones.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentMore {
    pub(crate) dct: u8,
    pub(crate) dca: u8,
    pub(crate) pitch_pan_separation: i8,
    pub(crate) pitch_pan_center: u8,
    pub(crate) random_volume: u8,
    pub(crate) random_pan: u8,
    pub(crate) cutoff: Option<u8>,
    pub(crate) resonance: Option<u8>,
    pub(crate) midi: Option<MidiOutDto>,
    pub(crate) effect: Option<u8>,
    pub(crate) phrase: Option<PhraseDto>,
}

fn pat<'a>(m: &'a mut Module, index: usize) -> Result<&'a mut tk::Pattern, String> {
    m.patterns.get_mut(index).map(Arc::make_mut).ok_or_else(|| format!("There's no pattern {index}."))
}

fn loop_of(kind: u8, start: u32, end: u32, frames: usize) -> tk::Loop {
    tk::Loop { kind: tk::LoopKind::from_u8(kind), start, end }.clamped(frames as u32)
}

pub(crate) fn apply(m: &mut Module, e: Edit) -> Result<(), String> {
    match e {
        Edit::Cells { pattern, row, channel, width, cells } => {
            let p = pat(m, pattern)?;
            let width = width.max(1);
            for (i, v) in cells.iter().enumerate() {
                if let Some(c) = p.cell_mut(row + i / width, channel + i % width) {
                    *c = cell_of(v);
                }
            }
        }
        Edit::Clear { pattern, row, channel, rows, channels, mask } => {
            let p = pat(m, pattern)?;
            for r in row..row + rows {
                for ch in channel..channel + channels {
                    if let Some(c) = p.cell_mut(r, ch) {
                        if mask & 1 != 0 {
                            c.note = 0;
                        }
                        if mask & 2 != 0 {
                            c.instr = 0;
                        }
                        if mask & 4 != 0 {
                            c.vol = Vol::None;
                            c.volp = 0;
                        }
                        if mask & 8 != 0 {
                            c.fx = Fx::None;
                            c.param = 0;
                        }
                    }
                }
            }
        }
        Edit::Transpose { pattern, row, channel, rows, channels, semitones } => {
            let p = pat(m, pattern)?;
            for r in row..row + rows {
                for ch in channel..channel + channels {
                    if let Some(c) = p.cell_mut(r, ch) {
                        if (tk::NOTE_MIN..=tk::NOTE_MAX).contains(&c.note) {
                            c.note = (i16::from(c.note) + i16::from(semitones)).clamp(i16::from(tk::NOTE_MIN), i16::from(tk::NOTE_MAX)) as u8;
                        }
                    }
                }
            }
        }
        Edit::SetInstrument { pattern, row, channel, rows, channels, instr } => {
            let p = pat(m, pattern)?;
            for r in row..row + rows {
                for ch in channel..channel + channels {
                    if let Some(c) = p.cell_mut(r, ch) {
                        if c.instr != 0 || c.note != 0 && c.note <= tk::NOTE_MAX {
                            c.instr = instr;
                        }
                    }
                }
            }
        }
        Edit::InsertRows { pattern, row, channel, channels, count } => {
            let p = pat(m, pattern)?;
            for ch in channel..(channel + channels).min(p.channels) {
                for r in (row..p.rows).rev() {
                    let v = if r >= row + count { p.cell(r - count, ch) } else { Cell::default() };
                    if let Some(c) = p.cell_mut(r, ch) {
                        *c = v;
                    }
                }
            }
        }
        Edit::DeleteRows { pattern, row, channel, channels, count } => {
            let p = pat(m, pattern)?;
            for ch in channel..(channel + channels).min(p.channels) {
                for r in row..p.rows {
                    let v = p.cell(r + count, ch);
                    if let Some(c) = p.cell_mut(r, ch) {
                        *c = v;
                    }
                }
            }
        }
        Edit::PatternRows { pattern, rows } => {
            let p = pat(m, pattern)?;
            *p = p.with_rows(rows.clamp(1, 1024));
        }
        Edit::PatternName { pattern, name } => pat(m, pattern)?.name = name,
        Edit::AddPattern => {
            if m.patterns.len() >= 999 {
                return Err("A song can have 999 patterns at most.".into());
            }
            m.patterns.push(Arc::new(tk::Pattern::new(64, m.channels.len())));
        }
        Edit::ClonePattern { pattern } => {
            let p = m.pattern(pattern).ok_or("There's no such pattern.")?.clone();
            m.patterns.push(Arc::new(p));
        }
        Edit::Orders { orders } => {
            let n = m.patterns.len() as u16;
            m.orders = orders.into_iter().filter(|&o| o >= tk::ORDER_SKIP || o < n).collect();
            if m.orders.is_empty() {
                m.orders.push(0);
            }
        }
        Edit::Song { title, artist, message, speed, tempo, global_volume, mix_volume, restart, linear } => {
            m.title = title;
            m.artist = artist;
            m.message = message;
            m.speed = speed.max(1);
            m.tempo = tempo.clamp(32, 999);
            m.global_volume = global_volume.min(128);
            m.mix_volume = mix_volume.clamp(1, 128);
            m.restart = restart.min(m.orders.len().saturating_sub(1) as u16);
            m.flags.linear = linear;
        }
        Edit::Channel { index, name, pan, volume, surround } => {
            let c = m.channels.get_mut(index).ok_or("There's no such channel.")?;
            c.name = name;
            c.pan = pan;
            c.volume = volume.min(64);
            c.surround = surround;
        }
        Edit::AddChannel => {
            if m.channels.len() >= 64 {
                return Err("A song can have 64 channels at most.".into());
            }
            let pan = if m.channels.len() % 2 == 0 { 64 } else { 192 };
            m.channels.push(tk::Channel { pan, ..Default::default() });
            let n = m.channels.len();
            for p in &mut m.patterns {
                *p = Arc::new(p.with_channels(n));
            }
        }
        Edit::RemoveChannel { index } => {
            if m.channels.len() <= 1 || index >= m.channels.len() {
                return Err("The last channel can't be removed.".into());
            }
            m.channels.remove(index);
            let n = m.channels.len();
            for p in &mut m.patterns {
                let mut np = tk::Pattern::new(p.rows, n);
                np.name = p.name.clone();
                for r in 0..p.rows {
                    let mut k = 0;
                    for ch in 0..p.channels {
                        if ch != index {
                            np.cells[r * n + k] = p.cell(r, ch);
                            k += 1;
                        }
                    }
                }
                *p = Arc::new(np);
            }
        }
        Edit::Sample { index, name, volume, global_volume, c5, pan, loop_kind, loop_start, loop_end, sus_kind, sus_start, sus_end, vibrato, credit } => {
            let s = m.samples.get_mut(index).ok_or("There's no such sample.")?;
            let frames = s.frames();
            s.name = name;
            if let Some(c) = credit {
                s.credit = c;
            }
            s.volume = volume.min(64);
            s.global_volume = global_volume.min(64);
            s.c5 = c5.clamp(100, 2_000_000);
            s.pan = pan;
            s.looped = loop_of(loop_kind, loop_start, loop_end, frames);
            s.sustain = loop_of(sus_kind, sus_start, sus_end, frames);
            s.vibrato = tk::AutoVibrato { kind: vibrato[0].min(4), speed: vibrato[1], depth: vibrato[2], sweep: vibrato[3] };
        }
        Edit::AddSample => {
            if m.samples.len() >= 255 && !m.flags.instruments {
                return Err("A song without instruments can have 255 samples at most.".into());
            }
            m.samples.push(tk::Sample::empty());
        }
        Edit::ClearSample { index } => {
            let s = m.samples.get_mut(index).ok_or("There's no such sample.")?;
            *s = tk::Sample::empty();
        }
        Edit::Normalize { index } => {
            let s = m.samples.get_mut(index).ok_or("There's no such sample.")?;
            let peak = s.wave.pcm.iter().map(|v| i32::from(*v).abs()).max().unwrap_or(0);
            if peak > 0 {
                let w = Arc::make_mut(&mut s.wave);
                let k = if w.bits == 8 { 32512.0 } else { 32767.0 } / peak as f32;
                for v in &mut w.pcm {
                    let x = (f32::from(*v) * k).round() as i32;
                    *v = if w.bits == 8 { ((x >> 8).clamp(-128, 127) << 8) as i16 } else { x.clamp(-32768, 32767) as i16 };
                }
            }
        }
        Edit::Reverse { index } => {
            let s = m.samples.get_mut(index).ok_or("There's no such sample.")?;
            let ch = usize::from(s.wave.channels.max(1));
            let w = Arc::make_mut(&mut s.wave);
            let frames: Vec<Vec<i16>> = w.pcm.chunks(ch).rev().map(|c| c.to_vec()).collect();
            w.pcm = frames.concat();
        }
        Edit::TrimToLoop { index } => {
            let s = m.samples.get_mut(index).ok_or("There's no such sample.")?;
            if s.looped.on() {
                let end = s.looped.end as usize * usize::from(s.wave.channels.max(1));
                Arc::make_mut(&mut s.wave).pcm.truncate(end);
            }
        }
        Edit::Instrument { index, name, global_volume, fadeout, pan, nna, volume_env, pan_env, pitch_env, more } => {
            let i = m.instruments.get_mut(index).ok_or("There's no such instrument.")?;
            if let Some(x) = more {
                i.dct = x.dct.min(3);
                i.dca = x.dca.min(2);
                i.pitch_pan_separation = x.pitch_pan_separation.clamp(-32, 32);
                i.pitch_pan_center = x.pitch_pan_center.min(119);
                i.random_volume = x.random_volume.min(100);
                i.random_pan = x.random_pan.min(64);
                i.cutoff = x.cutoff.map(|c| c.min(127));
                i.resonance = x.resonance.map(|r| r.min(127));
                i.midi = x.midi.map(|o| tk::MidiOut { channel: o.channel.min(15), program: o.program.map(|p| p.min(127)), bank: o.bank.map(|b| b.min(16383)) });
                i.effect = x.effect;
                i.phrase = x.phrase.map(|p| tk::Phrase { cells: p.cells.iter().map(cell_of).collect(), looped: p.looped, base: p.base.clamp(tk::NOTE_MIN, tk::NOTE_MAX) });
            }
            i.name = name;
            i.global_volume = global_volume.min(128);
            i.fadeout = fadeout.min(65536);
            i.pan = pan;
            i.nna = tk::Nna::from_u8(nna);
            i.volume_env = volume_env.to();
            i.pan_env = pan_env.to();
            i.pitch_env = pitch_env.to();
        }
        Edit::KeymapAll { index, sample } => {
            let i = m.instruments.get_mut(index).ok_or("There's no such instrument.")?;
            i.keymap = (0..120).map(|n| (n as u8, sample)).collect();
        }
        Edit::KeymapRange { index, from, to, sample } => {
            let i = m.instruments.get_mut(index).ok_or("There's no such instrument.")?;
            for n in from.min(119)..=to.min(119) {
                i.keymap[usize::from(n)] = (n, sample);
            }
        }
        Edit::AddInstrument => {
            if m.instruments.len() >= 255 {
                return Err("A song can have 255 instruments at most.".into());
            }
            let n = (m.instruments.len() + 1) as u16;
            m.instruments.push(tk::Instrument::for_sample("", if usize::from(n) <= m.samples.len() { n } else { 0 }));
        }
        Edit::InstrumentsMode { on } => {
            if on && m.instruments.is_empty() {
                // An instrument a sample, so the song sounds as it did.
                m.instruments = m.samples.iter().enumerate().map(|(i, s)| tk::Instrument::for_sample(&s.name, (i + 1) as u16)).collect();
            }
            m.flags.instruments = on;
        }
        Edit::KeymapNote { index, note, to_note, sample } => {
            let i = m.instruments.get_mut(index).ok_or("There's no such instrument.")?;
            if let Some(k) = i.keymap.get_mut(usize::from(note)) {
                *k = (to_note.min(119), sample);
            }
        }
        Edit::Interpolate { pattern, row, channel, rows, channels, effect } => {
            let p = pat(m, pattern)?;
            ed::interpolate(p, Block { row, channel, rows, channels }, if effect { ed::Column::Effect } else { ed::Column::Volume });
        }
        Edit::Amplify { pattern, row, channel, rows, channels, from, to } => {
            let samples: Vec<u8> = m.samples.iter().map(|s| s.volume).collect();
            let keyed: Vec<u16> = if m.flags.instruments { m.instruments.iter().map(|i| i.map(tk::NOTE_MIDDLE_C).1).collect() } else { Vec::new() };
            let instruments = m.flags.instruments;
            let volume = move |instr: u8| -> u8 {
                let s = if instruments { keyed.get(usize::from(instr).wrapping_sub(1)).copied().unwrap_or(0) } else { u16::from(instr) };
                samples.get(usize::from(s).wrapping_sub(1)).copied().unwrap_or(64)
            };
            let p = pat(m, pattern)?;
            ed::amplify(p, Block { row, channel, rows, channels }, from.clamp(0.0, 800.0), to.clamp(0.0, 800.0), &volume);
        }
        Edit::Paste { pattern, row, channel, width, cells, mode } => {
            let mode = match mode.as_str() {
                "mix" => ed::PasteMode::Mix,
                "mixCells" => ed::PasteMode::MixCells,
                "flood" => ed::PasteMode::Flood,
                "insert" => ed::PasteMode::Insert,
                _ => ed::PasteMode::Overwrite,
            };
            let cells: Vec<Cell> = cells.iter().map(cell_of).collect();
            ed::paste(pat(m, pattern)?, row, channel, width, &cells, mode);
        }
        Edit::ReplaceInstrument { from, to, pattern, row, channel, rows, channels } => {
            ed::replace_instrument(m, from, to, pattern.map(|p| (p, Block { row, channel, rows, channels })));
        }
        Edit::SwapInstruments { a, b } => ed::swap_instruments(m, a, b)?,
        Edit::PatternTime { pattern, time, swing } => {
            let p = pat(m, pattern)?;
            p.time = time.map(|(b, me)| tk::TimeSig { rows_per_beat: b.clamp(1, 64), rows_per_measure: me.clamp(1, 256) });
            p.swing = swing.map(|s| s.min(100));
        }
        Edit::Timing { tempo_mode, rows_per_beat, rows_per_measure, swing } => {
            m.tempo_mode = tk::TempoMode::from_u8(tempo_mode);
            m.time = tk::TimeSig { rows_per_beat: rows_per_beat.clamp(1, 64), rows_per_measure: rows_per_measure.clamp(1, 256) };
            m.swing = swing.min(100);
        }
        Edit::AddSequence { name, copy } => {
            let mut all = m.all_sequences();
            if all.len() >= 64 {
                return Err("A song can have 64 order lists at most.".into());
            }
            let now = m.sequence.min(all.len() - 1);
            let fresh = if copy { tk::Sequence { name: name.clone(), ..all[now].clone() } } else { tk::Sequence { name: name.clone(), orders: vec![0], restart: 0 } };
            all.push(fresh);
            m.sequences = all;
            m.switch_sequence(m.sequences.len() - 1);
        }
        Edit::RemoveSequence { index } => {
            let mut all = m.all_sequences();
            if all.len() <= 1 || index >= all.len() {
                return Err("The last order list can't be removed.".into());
            }
            all.remove(index);
            let cur = m.sequence.min(all.len());
            let to = if index < cur { cur - 1 } else { cur.min(all.len() - 1) };
            m.orders = all[to].orders.clone();
            m.restart = all[to].restart;
            if all.len() == 1 {
                m.sequences.clear();
                m.sequence = 0;
            } else {
                m.sequences = all;
                m.sequence = to;
            }
        }
        Edit::SwitchSequence { index } => m.switch_sequence(index),
        Edit::RenameSequence { index, name } => {
            let mut all = m.all_sequences();
            if let Some(q) = all.get_mut(index) {
                q.name = name;
            }
            m.sequences = all;
        }
        Edit::Bookmarks { bookmarks } => {
            m.bookmarks = bookmarks.into_iter().map(|b| tk::Bookmark { name: b.name, order: b.order, pattern: b.pattern, row: b.row }).collect();
        }
        Edit::SlotMute { order, channel, muted } => {
            m.slot_mutes.retain(|&x| x != (order, channel));
            if muted {
                m.slot_mutes.push((order, channel));
                m.slot_mutes.sort_unstable();
            }
        }
        Edit::Lanes { pattern, lanes } => {
            let p = pat(m, pattern)?;
            p.automation = lanes
                .into_iter()
                .map(|l| {
                    let mut points: Vec<(u16, f32)> = l.points.into_iter().map(|(r, v)| (r, v.clamp(0.0, 1.0))).collect();
                    points.sort_by_key(|p| p.0);
                    points.dedup_by_key(|p| p.0);
                    tk::Lane { target: l.target.to(), points }
                })
                .collect();
        }
        Edit::Effects { effects } => {
            if effects.len() > 32 {
                return Err("A song can have 32 effect slots at most.".into());
            }
            m.effects = effects.iter().map(EffectDto::to).collect();
            let n = m.effects.len() as u8;
            for c in &mut m.channels {
                c.effect = c.effect.filter(|&e| e < n);
            }
            for i in &mut m.instruments {
                i.effect = i.effect.filter(|&e| e < n);
            }
        }
        Edit::ChannelEffect { index, effect } => {
            let n = m.effects.len() as u8;
            let c = m.channels.get_mut(index).ok_or("There's no such channel.")?;
            c.effect = effect.filter(|&e| e < n);
        }
        Edit::Macros { macros } => {
            m.macros = macros.map(|x| {
                let fit = |mut v: Vec<String>, n: usize| {
                    v.resize(n, String::new());
                    v.truncate(n);
                    v
                };
                tk::MidiMacros { parametered: fit(x.parametered, 16), fixed: fit(x.fixed, 128) }
            });
        }
        Edit::SampleEdit { index, edit: op, from, to, value } => {
            let s = m.samples.get_mut(index).ok_or("There's no such sample.")?;
            if s.wave.is_empty() {
                return Err("That sample is empty.".into());
            }
            match op.as_str() {
                "cut" => se::cut(s, from, to),
                "crop" => se::crop(s, from, to),
                "silence" => se::silence(s, from, to),
                "fadeIn" => se::fade(s, from, to, true),
                "fadeOut" => se::fade(s, from, to, false),
                "gain" => se::gain(s, from, to, value),
                "normalize" => se::normalize(s, from, to),
                "reverse" => se::reverse(s, from, to),
                "dc" => se::remove_dc(s, from, to),
                "crossfade" => se::crossfade_loop(s, value.max(0.0) as usize),
                "smoothLoop" => {
                    let end = se::smooth_loop_end(s, value.max(1.0) as usize).ok_or("It has no loop to move.")?;
                    s.looped.end = end;
                }
                "stretch" => se::time_stretch(s, value),
                "pitch" => se::pitch_shift(s, value.clamp(-24.0, 24.0)),
                "resample" => se::resample(s, value.round().clamp(1000.0, 192_000.0) as u32),
                "slices" => s.slices = se::find_slices(s, value),
                "clearSlices" => s.slices.clear(),
                other => return Err(format!("There's no sample edit called {other}.")),
            }
            if s.frames() == 0 {
                *s = tk::Sample { name: s.name.clone(), ..tk::Sample::empty() };
            }
        }
        Edit::SampleFormat { index, bits, channels } => {
            let s = m.samples.get_mut(index).ok_or("There's no such sample.")?;
            se::set_format(s, if bits <= 8 { 8 } else { 16 }, channels.clamp(1, 2));
        }
        Edit::SampleDraw { index, at, points } => {
            let s = m.samples.get_mut(index).ok_or("There's no such sample.")?;
            se::draw(s, at, &points);
        }
        Edit::SampleSlices { index, slices } => {
            let s = m.samples.get_mut(index).ok_or("There's no such sample.")?;
            let n = s.frames() as u32;
            let mut v: Vec<u32> = slices.into_iter().filter(|&x| x < n).collect();
            v.sort_unstable();
            v.dedup();
            s.slices = v;
        }
        Edit::SlicesToInstrument { index, first } => {
            se::slices_to_instrument(m, index, first.clamp(tk::NOTE_MIN, tk::NOTE_MAX))?;
        }
        Edit::Synth { index, synth } => {
            let s = m.samples.get_mut(index).ok_or("There's no such sample.")?;
            let made = se::synth(&synth.to());
            let name = if s.name.trim().is_empty() || s.synth.is_some() { made.name.clone() } else { s.name.clone() };
            *s = tk::Sample { name, volume: s.volume, pan: s.pan, ..made };
        }
        Edit::Cleanup { patterns, samples, instruments } => {
            if patterns {
                ed::remove_unused_patterns(m);
            }
            if instruments {
                ed::remove_unused_instruments(m);
            }
            if samples {
                ed::remove_unused_samples(m, true);
            }
        }
    }
    Ok(())
}

// ---- Find and replace ----

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueryDto {
    pub(crate) notes: Option<(u8, u8)>,
    pub(crate) instr: Option<u8>,
    pub(crate) vol: Option<u8>,
    pub(crate) volp: Option<(u8, u8)>,
    pub(crate) fx: Option<u8>,
    pub(crate) param: Option<(u8, u8)>,
}

impl QueryDto {
    pub(crate) fn to(&self) -> ed::Query {
        ed::Query { notes: self.notes, instr: self.instr, vol: self.vol.map(Vol::from_u8), volp: self.volp, fx: self.fx.map(Fx::from_u8), param: self.param }
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReplacementDto {
    pub(crate) note: Option<u8>,
    #[serde(default)]
    pub(crate) transpose: i8,
    pub(crate) instr: Option<u8>,
    pub(crate) vol: Option<(u8, u8)>,
    pub(crate) fx: Option<(u8, u8)>,
    #[serde(default)]
    pub(crate) clear: bool,
}

/// Where find looks: the song, a pattern, or a block of one.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeDto {
    pub(crate) pattern: Option<usize>,
    pub(crate) block: Option<(usize, usize, usize, usize)>,
}

impl ScopeDto {
    pub(crate) fn to(&self) -> ed::Scope {
        match (self.pattern, self.block) {
            (Some(p), Some((row, channel, rows, channels))) => ed::Scope::Block(p, Block { row, channel, rows, channels }),
            (Some(p), None) => ed::Scope::Pattern(p),
            _ => ed::Scope::Song,
        }
    }
}



#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsDto {
    pub(crate) seconds: f64,
    pub(crate) patterns: usize,
    pub(crate) orders: usize,
    pub(crate) notes: usize,
    pub(crate) unused_patterns: Vec<usize>,
    pub(crate) empty_patterns: Vec<usize>,
    pub(crate) unused_samples: Vec<usize>,
    pub(crate) unused_instruments: Vec<usize>,
    pub(crate) unused_bytes: u64,
}

/// Where the song is as it plays, and each channel's level (0 to 1).
#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub playing: bool,
    pub order: usize,
    pub row: usize,
    pub levels: Vec<f32>,
}

/// A waveform's shape: the lowest and highest point in each of `width`
/// columns, -1 to 1.
pub(crate) fn peaks(w: &tk::Wave, width: usize) -> Vec<(f32, f32)> {
    let frames = w.frames();
    let ch = usize::from(w.channels.max(1));
    let width = width.clamp(1, 4096);
    if frames == 0 {
        return Vec::new();
    }
    (0..width)
        .map(|i| {
            let a = i * frames / width;
            let b = ((i + 1) * frames / width).max(a + 1).min(frames);
            let (mut lo, mut hi) = (i16::MAX, i16::MIN);
            for f in a..b {
                let v = w.pcm[f * ch];
                lo = lo.min(v);
                hi = hi.max(v);
            }
            (f32::from(lo) / 32768.0, f32::from(hi) / 32768.0)
        })
        .collect()
}

/// A sample file (WAV, AIFF, 8SVX, FLAC, ITS, XI, PAT) into a sample
/// slot, one undo step. An instrument file with more than one sample
/// (XI, PAT) adds its samples at the end and an instrument that plays
/// them, the song turned to instruments if it wasn't.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedSample {
    pub(crate) edited: Edited,
    pub(crate) samples: usize,
    /// The instrument made, 0-based, when the file was one.
    pub(crate) instrument: Option<usize>,
}
