// Copyright 2026 ansiapps. Neumetik Tracker: MIT licensed, see the LICENSE at the repository root.

//! **The tracker's session**: the song open in the editor, its undo, and
//! every command the app's screen sends, the same on every platform. The
//! desktop and mobile apps (`../src-tauri`) and the web build (`../web`)
//! each wrap a [`Session`] around their own sound output ([`Audio`]),
//! and pass it commands in one small binary format ([`Session::call_wire`]).
//!
//! The commands, and what each takes (JSON) and gives back:
//!
//! | Command | Takes | Gives |
//! | --- | --- | --- |
//! | `letters` | | the effect letters, formats, effect kinds, sample files |
//! | `companion` | `name`, the file's bytes | (a file kept for the next `open`) |
//! | `open` | `name`, `path`?, the file's bytes | `Opened` |
//! | `new` | `channels` | the song |
//! | `doc`, `pattern` | `index` | the song, a pattern |
//! | `save` | `name` | the song as an NTT (bytes) |
//! | `snapshot` | | the song as an NTT (bytes), nothing else changed |
//! | `saved` | `name`, `path`? | the song, marked saved (after a `snapshot` was written) |
//! | `exportWav`, `exportStems`, `exportModule` | `format` | a WAV, a zip of WAVs, a module (bytes) |
//! | `edit`, `undo` | `edits`, `show`; `redo`, `show` | `Edited` |
//! | `play`, `stop`, `jam`, `mute`, `playRow`, `fader`, `playback` | | |
//! | `find`, `replace`, `stats` | | |
//! | `samplePeaks`, `sampleAudition`, `sampleCopy`, `samplePaste` | | |
//! | `loadSample`, `saveSample`, `saveInstrument` | | |
//! | `status` | | where the song is, each channel's level |

pub mod dto;
pub mod engine;
mod zip;

use std::sync::Arc;

use neumetik_tracker::edit as ed;
use neumetik_tracker::player::Interp;
use neumetik_tracker::{self as tk, sample_edit as se, umf, Fx, Module, Player, Vol};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub use dto::Status;
pub use engine::{Cmd, Engine};

use dto::*;

/// The most undo steps kept.
const UNDO: usize = 200;
/// The rate songs are exported at.
const EXPORT_RATE: u32 = 44_100;
/// The longest a song is played for when exported or measured, in seconds.
const MOST: f64 = 3600.0;

/// Where the session's sound goes: the platform's output.
pub trait Audio {
    /// The output's rate, once there's sound out.
    fn rate(&self) -> Option<u32>;
    fn send(&mut self, cmd: Cmd);
    fn status(&mut self) -> Status;
}

/// The song open in the editor.
pub(crate) struct Doc {
    pub(crate) module: Module,
    /// The file it was opened from or saved to, where there are paths.
    pub(crate) path: Option<String>,
    pub(crate) name: String,
    pub(crate) dirty: bool,
    /// Samples whose waveforms weren't found when it was opened (a UMF
    /// that refers to a samples repository it doesn't carry).
    pub(crate) missing: usize,
    pub(crate) undo: Vec<Module>,
    pub(crate) redo: Vec<Module>,
}

/// What a command gives back: JSON, and bytes for the ones that make a file.
#[derive(Debug, Default)]
pub struct Reply {
    pub json: Value,
    pub bytes: Vec<u8>,
}

impl Reply {
    fn of(v: impl Serialize) -> Result<Reply, String> {
        Ok(Reply { json: serde_json::to_value(v).map_err(|e| e.to_string())?, bytes: Vec::new() })
    }

    fn file(v: impl Serialize, bytes: Vec<u8>) -> Result<Reply, String> {
        Ok(Reply { bytes, ..Reply::of(v)? })
    }
}

/// A command that takes a while (playing the whole song through), done
/// where it holds nothing up: what it needs is taken from the session first.
pub type Job = Box<dyn FnOnce() -> Result<Reply, String> + Send>;

/// A command, done or ready to be.
pub enum Prepared {
    Done(Result<Reply, String>),
    Job(Job),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    doc: DocSummary,
    missing: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Edited {
    doc: DocSummary,
    /// The pattern asked for, as it is now.
    pattern: Option<PatternDto>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Replaced {
    count: usize,
    edited: Edited,
}

pub struct Session<A: Audio> {
    audio: A,
    doc: Option<Doc>,
    /// How samples are read, and whether a MOD has the Amiga's filters.
    playback: (Interp, bool),
    /// A stretch of waveform copied in the sample editor.
    clip: Option<tk::Wave>,
    /// Files given for the next module opened, for the formats that keep
    /// their samples in a file beside the module.
    companions: Vec<(String, Vec<u8>)>,
}

fn args<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("The app sent a command that couldn't be read. ({e})"))
}

fn no_song() -> String {
    "There's no song open.".into()
}

fn stem(name: &str) -> String {
    let s = name.rsplit_once('.').map_or(name, |(a, _)| a).trim();
    if s.is_empty() { "song".into() } else { s.into() }
}

impl<A: Audio> Session<A> {
    pub fn new(audio: A) -> Session<A> {
        Session { audio, doc: None, playback: (Interp::Cubic, false), clip: None, companions: Vec::new() }
    }

    pub fn audio(&mut self) -> &mut A {
        &mut self.audio
    }

    /// The sound output started (or started again, at another rate): the
    /// song is given to it.
    pub fn audio_started(&mut self) {
        if let Some(d) = &self.doc {
            let m = d.module.clone();
            self.load_player(&m);
        }
    }

    fn doc(&self) -> Result<&Doc, String> {
        self.doc.as_ref().ok_or_else(no_song)
    }

    fn doc_mut(&mut self) -> Result<&mut Doc, String> {
        self.doc.as_mut().ok_or_else(no_song)
    }

    /// The song to the sound output: a new player, idle until Play, reading
    /// samples as the playback settings say.
    fn load_player(&mut self, module: &Module) {
        if let Some(rate) = self.audio.rate() {
            let mut p = Player::idle(Arc::new(module.clone()), rate);
            (p.interp, p.amiga) = self.playback;
            self.audio.send(Cmd::Load(Some(Box::new(p))));
        }
    }

    /// The song changed: to the player, with its effect slots made ready
    /// here when they're not the ones it had.
    fn module_changed(&mut self, before: Option<&Module>) {
        let Some(d) = &self.doc else { return };
        let kinds = |m: &Module| m.effects.iter().map(|e| e.kind).collect::<Vec<_>>();
        let rack = match (before, self.audio.rate()) {
            (Some(b), Some(rate)) if kinds(b) != kinds(&d.module) => Some(Player::rack_for(&d.module, rate)),
            _ => None,
        };
        let m = Arc::new(d.module.clone());
        self.audio.send(Cmd::Module(m, rack));
    }

    fn edited(&self, show: Option<usize>) -> Result<Edited, String> {
        let d = self.doc()?;
        Ok(Edited { doc: summary(d), pattern: show.and_then(|p| pattern_dto(&d.module, p)) })
    }

    /// A new module for the song, as one undo step.
    fn commit(&mut self, next: Module, show: Option<usize>) -> Result<Edited, String> {
        let d = self.doc_mut()?;
        let before = std::mem::replace(&mut d.module, next);
        d.redo.clear();
        d.dirty = true;
        self.module_changed(Some(&before));
        let d = self.doc_mut()?;
        d.undo.push(before);
        if d.undo.len() > UNDO {
            d.undo.remove(0);
        }
        self.edited(show)
    }

    /// Opens a module (any format read) or a UMF; `companion` gives the
    /// files beside it, by name, for the formats that keep samples there.
    pub fn open(&mut self, data: &[u8], name: &str, path: Option<String>, companion: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Opened, String> {
        let given = std::mem::take(&mut self.companions);
        let near = |n: &str| given.iter().find(|(f, _)| f.eq_ignore_ascii_case(n)).map(|(_, b)| b.clone()).or_else(|| companion(n));
        let (module, missing) = if umf::probe(data) {
            let loaded = umf::read(data, &mut |_| None).map_err(|e| format!("{name} couldn't be opened: {e}."))?;
            (loaded.module, loaded.missing.len())
        } else {
            (tk::load_with(data, name, &near).map_err(|e| format!("{name} couldn't be opened: {e}."))?, 0)
        };
        self.load_player(&module);
        let doc = Doc { name: name.into(), module, path, dirty: false, missing, undo: Vec::new(), redo: Vec::new() };
        let opened = Opened { doc: summary(&doc), missing };
        self.doc = Some(doc);
        Ok(opened)
    }

    /// A command by name, its arguments as JSON and any bytes it takes
    /// (a file), done here whatever it costs.
    pub fn call(&mut self, cmd: &str, a: Value, data: Vec<u8>) -> Result<Reply, String> {
        match self.prepare(cmd, a, data) {
            Prepared::Done(r) => r,
            Prepared::Job(job) => job(),
        }
    }

    /// `call` in the wire format both platforms speak: in, the command's
    /// name and its JSON, each after its length (u32, little-endian), then
    /// the bytes it takes; out, 0 (done) or 1 (failed), the JSON (or the
    /// failure's words) after its length, then the bytes it gives.
    pub fn call_wire(&mut self, msg: &[u8]) -> Vec<u8> {
        match wire::request(msg) {
            Ok((cmd, a, data)) => wire::reply(self.call(&cmd, a, data)),
            Err(e) => wire::reply(Err(e)),
        }
    }

    /// A command, done now unless it takes a while: then what it needs is
    /// taken, to be done elsewhere.
    pub fn prepare(&mut self, cmd: &str, a: Value, data: Vec<u8>) -> Prepared {
        match cmd {
            "exportWav" | "exportStems" | "stats" => match self.doc() {
                Ok(d) => {
                    let module = d.module.clone();
                    let name = d.name.clone();
                    let cmd = cmd.to_string();
                    Prepared::Job(Box::new(move || heavy(&cmd, module, &name)))
                }
                Err(e) => Prepared::Done(Err(e)),
            },
            _ => Prepared::Done(self.command(cmd, a, data)),
        }
    }

    fn command(&mut self, cmd: &str, a: Value, data: Vec<u8>) -> Result<Reply, String> {
        match cmd {
            "letters" => Reply::of(letters()),
            "status" => Reply::of(self.audio.status()),
            "companion" => {
                #[derive(Deserialize)]
                struct A {
                    name: String,
                }
                let a: A = args(a)?;
                self.companions.push((a.name, data));
                Reply::of(())
            }
            "open" => {
                #[derive(Deserialize)]
                struct A {
                    name: String,
                    #[serde(default)]
                    path: Option<String>,
                }
                let a: A = args(a)?;
                Reply::of(self.open(&data, &a.name, a.path, &|_| None)?)
            }
            "new" => {
                #[derive(Deserialize)]
                struct A {
                    channels: usize,
                }
                let a: A = args(a)?;
                let mut module = Module::new(a.channels.clamp(1, 64));
                module.title = "New song".into();
                module.samples = (0..8).map(|_| tk::Sample::empty()).collect();
                self.load_player(&module);
                let doc = Doc { name: "New song".into(), module, path: None, dirty: false, missing: 0, undo: Vec::new(), redo: Vec::new() };
                let s = summary(&doc);
                self.doc = Some(doc);
                Reply::of(s)
            }
            "doc" => Reply::of(self.doc.as_ref().map(summary)),
            "pattern" => {
                #[derive(Deserialize)]
                struct A {
                    index: usize,
                }
                let a: A = args(a)?;
                Reply::of(self.doc.as_ref().and_then(|d| pattern_dto(&d.module, a.index)))
            }
            // The song as an NTT (a UMF with its samples): everything it is, kept.
            "save" => {
                #[derive(Deserialize)]
                struct A {
                    name: String,
                    #[serde(default)]
                    path: Option<String>,
                }
                let a: A = args(a)?;
                let d = self.doc_mut()?;
                let bytes = umf::write_ntt(&d.module, &umf::About { bundled: true, ..Default::default() });
                d.dirty = false;
                d.name = a.name;
                if a.path.is_some() {
                    d.path = a.path;
                }
                Reply::file(summary(d), bytes)
            }
            // The song written to a file (as `snapshot` gave it): named for it, and not changed since.
            "saved" => {
                #[derive(Deserialize)]
                struct A {
                    name: String,
                    #[serde(default)]
                    path: Option<String>,
                }
                let a: A = args(a)?;
                let d = self.doc_mut()?;
                d.dirty = false;
                d.name = a.name;
                if a.path.is_some() {
                    d.path = a.path;
                }
                Reply::of(summary(d))
            }
            "snapshot" => {
                let d = self.doc()?;
                Reply::file(&d.name, umf::write_ntt(&d.module, &umf::About { bundled: true, ..Default::default() }))
            }
            // The song written as MOD, S3M, XM or IT, and what the format couldn't keep.
            "exportModule" => {
                #[derive(Deserialize)]
                struct A {
                    format: String,
                }
                let a: A = args(a)?;
                let f = tk::export::Format::from_name(&a.format).ok_or("That format isn't written.")?;
                let e = tk::export::export(&self.doc()?.module, f);
                Reply::file(e.lost, e.bytes)
            }
            "edit" => {
                #[derive(Deserialize)]
                struct A {
                    edits: Vec<Edit>,
                    show: Option<usize>,
                }
                let a: A = args(a)?;
                let mut next = self.doc()?.module.clone();
                for e in a.edits {
                    apply(&mut next, e)?;
                }
                next.tidy();
                Reply::of(self.commit(next, a.show)?)
            }
            "undo" => {
                #[derive(Deserialize)]
                struct A {
                    redo: bool,
                    show: Option<usize>,
                }
                let a: A = args(a)?;
                let d = self.doc_mut()?;
                let (from, to) = if a.redo { (&mut d.redo, &mut d.undo) } else { (&mut d.undo, &mut d.redo) };
                let mut before = None;
                if let Some(m) = from.pop() {
                    let old = std::mem::replace(&mut d.module, m);
                    before = Some(old.clone());
                    to.push(old);
                    d.dirty = true;
                }
                self.module_changed(before.as_ref());
                Reply::of(self.edited(a.show)?)
            }
            "play" => {
                #[derive(Deserialize)]
                struct A {
                    order: usize,
                    row: usize,
                    hold: bool,
                }
                let a: A = args(a)?;
                self.doc()?;
                self.audio.send(Cmd::Play { order: a.order, row: a.row, hold: a.hold });
                Reply::of(())
            }
            "stop" => {
                self.audio.send(Cmd::Stop);
                Reply::of(())
            }
            // A note from the keyboard (or a click on a sample), on the
            // instrument (or sample) given; `NOTE_OFF` (255) lets it go.
            "jam" => {
                #[derive(Deserialize)]
                struct A {
                    note: u8,
                    instr: u8,
                }
                let a: A = args(a)?;
                self.audio.send(Cmd::Jam(a.note, a.instr));
                Reply::of(())
            }
            "mute" => {
                #[derive(Deserialize)]
                struct A {
                    channel: usize,
                    muted: bool,
                }
                let a: A = args(a)?;
                self.audio.send(Cmd::Mute(a.channel, a.muted));
                Reply::of(())
            }
            // The row at an order played on its own (Impulse Tracker's 8).
            "playRow" => {
                #[derive(Deserialize)]
                struct A {
                    order: usize,
                    row: usize,
                }
                let a: A = args(a)?;
                self.audio.send(Cmd::PlayRow { order: a.order, row: a.row });
                Reply::of(())
            }
            // A channel's fader as the song plays: a gain (1 as written) and a panning (0 to 1), or the song's.
            "fader" => {
                #[derive(Deserialize)]
                struct A {
                    channel: usize,
                    gain: f32,
                    pan: Option<f32>,
                }
                let a: A = args(a)?;
                self.audio.send(Cmd::Fader(a.channel, a.gain, a.pan));
                Reply::of(())
            }
            // How samples are read (0 none, 1 linear, 2 cubic, 3 sinc), and the Amiga's filters over a MOD.
            "playback" => {
                #[derive(Deserialize)]
                struct A {
                    interp: u8,
                    amiga: bool,
                }
                let a: A = args(a)?;
                self.playback = (Interp::from_u8(a.interp), a.amiga);
                self.audio.send(Cmd::Playback(self.playback.0, a.amiga));
                Reply::of(())
            }
            // Every cell that matches: (pattern, row, channel).
            "find" => {
                #[derive(Deserialize)]
                struct A {
                    query: QueryDto,
                    scope: ScopeDto,
                }
                let a: A = args(a)?;
                Reply::of(ed::find(&self.doc()?.module, &a.query.to(), a.scope.to()))
            }
            // Every match replaced, one undo step.
            "replace" => {
                #[derive(Deserialize)]
                struct A {
                    query: QueryDto,
                    with: ReplacementDto,
                    scope: ScopeDto,
                    show: Option<usize>,
                }
                let a: A = args(a)?;
                let q = a.query.to();
                if q.is_empty() {
                    return Err("Say what to find first.".into());
                }
                let mut next = self.doc()?.module.clone();
                let w = a.with;
                let r = ed::Replacement { note: w.note, transpose: w.transpose, instr: w.instr, vol: w.vol.map(|(v, x)| (Vol::from_u8(v), x)), fx: w.fx.map(|(f, x)| (Fx::from_u8(f), x)), clear: w.clear };
                let count = ed::replace(&mut next, &q, &r, a.scope.to());
                let edited = if count == 0 { self.edited(a.show)? } else { self.commit(next, a.show)? };
                Reply::of(Replaced { count, edited })
            }
            // A sample's shape, all of it or (zoomed) frames `from` to `to`.
            "samplePeaks" => {
                #[derive(Deserialize)]
                struct A {
                    index: usize,
                    width: usize,
                    from: Option<usize>,
                    to: Option<usize>,
                }
                let a: A = args(a)?;
                let Some(s) = self.doc()?.module.samples.get(a.index) else { return Reply::of(Vec::<(f32, f32)>::new()) };
                Reply::of(match (a.from, a.to) {
                    (Some(x), Some(y)) if x < y && y <= s.frames() && (x, y) != (0, s.frames()) => peaks(&se::copy(s, x, y), a.width),
                    _ => peaks(&s.wave, a.width),
                })
            }
            // A sample heard on its own from frame `from` (to `to`, or its
            // end, looping as it does when it plays to the end), at middle C.
            "sampleAudition" => {
                #[derive(Deserialize)]
                struct A {
                    index: usize,
                    from: usize,
                    to: Option<usize>,
                }
                let a: A = args(a)?;
                let mut sample = self.doc()?.module.samples.get(a.index).ok_or("There's no such sample.")?.clone();
                let n = sample.frames();
                let end = a.to.unwrap_or(n).min(n);
                if a.from >= end {
                    self.audio.send(Cmd::Preview(None));
                    return Reply::of(());
                }
                if (a.from, end) != (0, n) {
                    let (from, cut) = (a.from, a.to.is_some());
                    sample.wave = Arc::new(se::copy(&sample, from, end));
                    let shift = |l: tk::Loop| if cut { tk::Loop::default() } else { tk::Loop { start: l.start.saturating_sub(from as u32), end: l.end.saturating_sub(from as u32), ..l }.clamped((end - from) as u32) };
                    sample.looped = shift(sample.looped);
                    sample.sustain = shift(sample.sustain);
                }
                let rate = self.audio.rate().ok_or("There's no sound output.")?;
                let mut m = Module::new(1);
                m.kind = tk::Kind::It;
                m.samples = vec![sample];
                let mut p = Player::idle(Arc::new(m), rate);
                p.jam(tk::NOTE_MIDDLE_C, 1);
                self.audio.send(Cmd::Preview(Some(Box::new(p))));
                Reply::of(())
            }
            // Frames `from` to `to` of a sample copied (`cut` takes them out too, one undo step).
            "sampleCopy" => {
                #[derive(Deserialize)]
                struct A {
                    index: usize,
                    from: usize,
                    to: usize,
                    cut: bool,
                    show: Option<usize>,
                }
                let a: A = args(a)?;
                let s = self.doc()?.module.samples.get(a.index).ok_or("There's no such sample.")?;
                let w = se::copy(s, a.from, a.to);
                if w.is_empty() {
                    return Err("Mark some of the waveform first.".into());
                }
                self.clip = Some(w);
                if !a.cut {
                    return Reply::of(None::<Edited>);
                }
                let mut next = self.doc()?.module.clone();
                se::cut(&mut next.samples[a.index], a.from, a.to);
                Reply::of(Some(self.commit(next, a.show)?))
            }
            // What was copied put into a sample at `at` (into an empty one, it's all of it).
            "samplePaste" => {
                #[derive(Deserialize)]
                struct A {
                    index: usize,
                    at: usize,
                    show: Option<usize>,
                }
                let a: A = args(a)?;
                let clip = self.clip.clone().ok_or("Nothing's been copied from a waveform yet.")?;
                let mut next = self.doc()?.module.clone();
                se::paste(next.samples.get_mut(a.index).ok_or("There's no such sample.")?, a.at, &clip);
                Reply::of(self.commit(next, a.show)?)
            }
            "loadSample" => {
                #[derive(Deserialize)]
                struct A {
                    index: usize,
                    name: String,
                    show: Option<usize>,
                }
                let a: A = args(a)?;
                Reply::of(self.load_sample(a.index, &a.name, &data, a.show)?)
            }
            // A sample saved as a file of the kind `ext` names, and what that kind couldn't keep.
            "saveSample" => {
                #[derive(Deserialize)]
                struct A {
                    index: usize,
                    ext: String,
                }
                let a: A = args(a)?;
                let s = self.doc()?.module.samples.get(a.index).ok_or("There's no such sample.")?;
                if s.wave.is_empty() {
                    return Err("That sample is empty.".into());
                }
                let (bytes, lost) = tk::samplefile::write(s, &a.ext.to_ascii_lowercase());
                Reply::file(lost, bytes)
            }
            // An instrument and its samples saved as a FastTracker 2 XI file.
            "saveInstrument" => {
                #[derive(Deserialize)]
                struct A {
                    index: usize,
                }
                let a: A = args(a)?;
                let m = &self.doc()?.module;
                let ins = m.instruments.get(a.index).ok_or("There's no such instrument.")?;
                Reply::file((), tk::samplefile::write_xi(ins, &m.samples))
            }
            other => Err(format!("There's no command called {other}.")),
        }
    }

    /// A sample file (WAV, AIFF, 8SVX, FLAC, ITS, XI, PAT) into a sample
    /// slot, one undo step. An instrument file with more than one sample
    /// (XI, PAT) adds its samples at the end and an instrument that plays
    /// them, the song turned to instruments if it wasn't.
    fn load_sample(&mut self, index: usize, name: &str, data: &[u8], show: Option<usize>) -> Result<LoadedSample, String> {
        let loaded = tk::samplefile::read(data, name).map_err(|e| format!("{name} couldn't be read: {e}."))?;
        let mut next = self.doc()?.module.clone();
        let count = loaded.samples.len();
        let mut instrument = None;
        match loaded.instrument.filter(|_| count > 1) {
            Some(mut ins) => {
                let first = next.samples.len();
                if next.samples.len() + count > 4000 {
                    return Err("That's too many samples for one song.".into());
                }
                next.samples.extend(loaded.samples);
                for k in &mut ins.keymap {
                    if k.1 > 0 {
                        k.1 += first as u16;
                    }
                }
                if !next.flags.instruments {
                    next.instruments = next.samples.iter().take(first).enumerate().map(|(i, x)| tk::Instrument::for_sample(&x.name, (i + 1) as u16)).collect();
                    next.flags.instruments = true;
                }
                ins.filename = name.into();
                next.instruments.push(ins);
                instrument = Some(next.instruments.len() - 1);
            }
            None => {
                let slot = next.samples.get_mut(index).ok_or("There's no such sample.")?;
                let mut smp = loaded.samples.into_iter().next().ok_or("It has no sample in it.")?;
                smp.name = smp.name.chars().take(32).collect();
                smp.filename = name.chars().take(32).collect();
                *slot = smp;
            }
        }
        next.tidy();
        let edited = self.commit(next, show)?;
        Ok(LoadedSample { edited, samples: count, instrument })
    }
}

/// The commands that play the whole song through.
fn heavy(cmd: &str, module: Module, name: &str) -> Result<Reply, String> {
    match cmd {
        // The song as a WAV file at 44.1 kHz, played once through.
        "exportWav" => {
            let audio = tk::player::render(Arc::new(module), EXPORT_RATE, MOST, 1.0);
            Reply::file((), tk::wav::write(&audio, EXPORT_RATE))
        }
        // Each channel played on its own, a WAV file each, in a zip:
        // "<song> 01 <channel's name>.wav" and on.
        "exportStems" => {
            let stem = stem(name);
            let names: Vec<String> = module.channels.iter().map(|c| c.name.clone()).collect();
            let stems = tk::player::render_stems(Arc::new(module), EXPORT_RATE, MOST, 1.0, &mut |_| {});
            let mut files = Vec::new();
            for (ch, audio) in stems.iter().enumerate() {
                if audio.is_empty() {
                    continue;
                }
                let label: String = names[ch].trim().chars().filter(|c| !"/\\:*?\"<>|".contains(*c)).collect();
                let file = if label.is_empty() { format!("{stem} {:02}.wav", ch + 1) } else { format!("{stem} {:02} {label}.wav", ch + 1) };
                files.push((file, tk::wav::write(audio, EXPORT_RATE)));
            }
            let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
            Reply::file(&names, zip::store(&files))
        }
        "stats" => {
            let seconds = tk::player::length(Arc::new(module.clone()), MOST);
            let s = ed::stats(&module, seconds);
            Reply::of(StatsDto { seconds: s.seconds, patterns: s.patterns, orders: s.orders, notes: s.notes, unused_patterns: s.unused_patterns, empty_patterns: s.empty_patterns, unused_samples: s.unused_samples, unused_instruments: s.unused_instruments, unused_bytes: s.unused_bytes })
        }
        other => Err(format!("There's no command called {other}.")),
    }
}

/// The wire format (`Session::call_wire`).
pub mod wire {
    use super::*;

    fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8], String> {
        if b.len() < n {
            return Err("The app sent a command cut short.".into());
        }
        let (a, rest) = b.split_at(n);
        *b = rest;
        Ok(a)
    }

    fn len(b: &mut &[u8]) -> Result<usize, String> {
        Ok(u32::from_le_bytes(take(b, 4)?.try_into().expect("4 bytes")) as usize)
    }

    /// A request: its command, its arguments and its bytes.
    pub fn request(mut b: &[u8]) -> Result<(String, Value, Vec<u8>), String> {
        let n = len(&mut b)?;
        let cmd = String::from_utf8_lossy(take(&mut b, n)?).to_string();
        let n = len(&mut b)?;
        let a = take(&mut b, n)?;
        let a = if a.is_empty() { Value::Null } else { serde_json::from_slice(a).map_err(|e| format!("The app sent a command that couldn't be read. ({e})"))? };
        Ok((cmd, a, b.to_vec()))
    }

    /// A request made (what the platforms' tests send).
    pub fn encode(cmd: &str, a: &Value, data: &[u8]) -> Vec<u8> {
        let a = serde_json::to_vec(a).unwrap_or_default();
        let mut out = Vec::with_capacity(8 + cmd.len() + a.len() + data.len());
        out.extend_from_slice(&(cmd.len() as u32).to_le_bytes());
        out.extend_from_slice(cmd.as_bytes());
        out.extend_from_slice(&(a.len() as u32).to_le_bytes());
        out.extend_from_slice(&a);
        out.extend_from_slice(data);
        out
    }

    /// A reply, done or failed.
    pub fn reply(r: Result<Reply, String>) -> Vec<u8> {
        let (ok, json, bytes) = match r {
            Ok(r) => (0u8, serde_json::to_vec(&r.json).unwrap_or_else(|_| b"null".to_vec()), r.bytes),
            Err(e) => (1u8, serde_json::to_vec(&json!(e)).unwrap_or_default(), Vec::new()),
        };
        let mut out = Vec::with_capacity(5 + json.len() + bytes.len());
        out.push(ok);
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&json);
        out.extend_from_slice(&bytes);
        out
    }

    /// A reply read back: its JSON, or what failed, and its bytes.
    pub fn decode(mut b: &[u8]) -> Result<(Value, Vec<u8>), String> {
        let ok = take(&mut b, 1)?[0] == 0;
        let n = len(&mut b)?;
        let json: Value = serde_json::from_slice(take(&mut b, n)?).map_err(|e| e.to_string())?;
        if ok {
            Ok((json, b.to_vec()))
        } else {
            Err(json.as_str().unwrap_or("It failed.").to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sound that goes nowhere, at a rate, its commands counted.
    #[derive(Default)]
    struct Quiet {
        engine: Option<Engine>,
    }

    impl Audio for Quiet {
        fn rate(&self) -> Option<u32> {
            Some(44_100)
        }
        fn send(&mut self, cmd: Cmd) {
            self.engine.get_or_insert_with(Engine::new).apply(cmd);
        }
        fn status(&mut self) -> Status {
            let mut s = Status::default();
            self.engine.get_or_insert_with(Engine::new).status(&mut s);
            s
        }
    }

    fn call(s: &mut Session<Quiet>, cmd: &str, a: Value) -> (Value, Vec<u8>) {
        wire::decode(&s.call_wire(&wire::encode(cmd, &a, &[]))).unwrap_or_else(|e| panic!("{cmd}: {e}"))
    }

    #[test]
    fn a_new_song_is_edited_undone_saved_and_opened_again() {
        let mut s = Session::new(Quiet::default());
        let (doc, _) = call(&mut s, "new", json!({ "channels": 4 }));
        assert_eq!(doc["channels"].as_array().map(|c| c.len()), Some(4));
        let (e, _) = call(&mut s, "edit", json!({ "edits": [{ "op": "cells", "pattern": 0, "row": 2, "channel": 1, "width": 1, "cells": [[49, 1, 0, 0, 0, 0]] }], "show": 0 }));
        assert_eq!(e["pattern"]["cells"][(2 * 4 + 1) * 6], 49);
        assert_eq!(e["doc"]["canUndo"], true);
        let (u, _) = call(&mut s, "undo", json!({ "redo": false, "show": 0 }));
        assert_eq!(u["pattern"]["cells"][(2 * 4 + 1) * 6], 0);
        call(&mut s, "undo", json!({ "redo": true, "show": 0 }));
        let (saved, ntt) = call(&mut s, "save", json!({ "name": "tune.ntt" }));
        assert_eq!(saved["dirty"], false);
        assert!(umf::probe(&ntt));
        let mut t = Session::new(Quiet::default());
        let back = wire::decode(&t.call_wire(&wire::encode("open", &json!({ "name": "tune.ntt" }), &ntt))).expect("opens");
        assert_eq!(back.0["doc"]["name"], "tune.ntt");
        let (p, _) = call(&mut t, "pattern", json!({ "index": 0 }));
        assert_eq!(p["cells"][(2 * 4 + 1) * 6], 49);
    }

    #[test]
    fn a_song_is_played_and_exported() {
        let mut s = Session::new(Quiet::default());
        call(&mut s, "new", json!({ "channels": 2 }));
        // A square wave in sample 1, and a note on it.
        let wave: Vec<u8> = (0..4000).flat_map(|i: i32| (if i % 100 < 50 { 12000i16 } else { -12000 }).to_le_bytes()).collect();
        let wav = tk::wav::write(&wave.chunks(2).flat_map(|b| { let v = f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0; [v, v] }).collect::<Vec<f32>>(), 22_050);
        let r = wire::decode(&s.call_wire(&wire::encode("loadSample", &json!({ "index": 0, "name": "square.wav", "show": 0 }), &wav))).expect("loads");
        assert!(r.0["edited"]["doc"]["samples"][0]["frames"].as_u64().unwrap_or(0) > 0);
        call(&mut s, "edit", json!({ "edits": [{ "op": "cells", "pattern": 0, "row": 0, "channel": 0, "width": 1, "cells": [[49, 1, 0, 0, 0, 0]] }], "show": 0 }));
        call(&mut s, "play", json!({ "order": 0, "row": 0, "hold": false }));
        let (st, _) = call(&mut s, "status", Value::Null);
        assert_eq!(st["playing"], true);
        let (_, wav) = call(&mut s, "exportWav", Value::Null);
        assert_eq!(&wav[..4], b"RIFF");
        let (names, zip) = call(&mut s, "exportStems", Value::Null);
        assert_eq!(names.as_array().map(|n| n.len()), Some(2), "a stem a channel");
        assert_eq!(&zip[..4], b"PK\x03\x04");
        let (lost, it) = call(&mut s, "exportModule", json!({ "format": "it" }));
        assert!(lost.is_array());
        assert_eq!(&it[..4], b"IMPM");
        let (stats, _) = call(&mut s, "stats", Value::Null);
        assert_eq!(stats["notes"], 1);
    }

    #[test]
    fn a_failure_is_said_in_words() {
        let mut s = Session::new(Quiet::default());
        let e = wire::decode(&s.call_wire(&wire::encode("pattern", &json!({}), &[]))).unwrap_err();
        assert!(e.contains("couldn't be read"), "{e}");
        assert_eq!(wire::decode(&s.call_wire(&wire::encode("edit", &json!({ "edits": [], "show": null }), &[]))).unwrap_err(), "There's no song open.");
        assert!(wire::decode(&s.call_wire(&[1, 2])).is_err());
    }
}
