/**
 * The tracker as the screen sees it: every type the session
 * (session/src/dto.rs) sends, and every command. The song lives in the
 * session, with its undo; the screen asks for a pattern at a time and
 * sends edits. Each platform runs the session its own way (lib/host.ts).
 */
import { host } from "./host";

async function call<T>(cmd: string, args?: unknown, data?: Uint8Array): Promise<T> {
  return (await (await host()).call<T>(cmd, args, data)).json;
}

/** A command that makes a file: what it says, and the file's bytes. */
async function file<T>(cmd: string, args?: unknown): Promise<[T, Uint8Array]> {
  const r = await (await host()).call<T>(cmd, args);
  return [r.json, r.bytes];
}

export interface Channel {
  name: string;
  pan: number;
  volume: number;
  muted: boolean;
  surround: boolean;
  /** The effect slot its sound goes through. */
  effect: number | null;
}

export interface Envelope {
  enabled: boolean;
  /** [tick, value]: volume 0 to 64, panning and pitch -32 to 32. */
  points: [number, number][];
  looped: [number, number] | null;
  sustain: [number, number] | null;
  carry: boolean;
  filter: boolean;
  /** OpenMPT's release node: the point it goes on from once the note's let go. */
  release: number | null;
}

export interface Sample {
  name: string;
  filename: string;
  frames: number;
  bits: number;
  channels: number;
  /** The rate middle C plays at, in hertz. */
  c5: number;
  volume: number;
  globalVolume: number;
  pan: number | null;
  /** 0 off, 1 forward, 2 ping-pong. */
  loopKind: number;
  loopStart: number;
  loopEnd: number;
  susKind: number;
  susStart: number;
  susEnd: number;
  /** Auto-vibrato: kind, speed, depth, sweep. */
  vibrato: [number, number, number, number];
  credit: string;
  /** Where it's sliced, in frames. */
  slices: number[];
  synth: Synth | null;
}

/** A synth made into a sample: 0 sine, 1 saw, 2 square, 3 triangle, 4 noise, 5 FM. */
export interface Synth {
  wave: number;
  width: number;
  ratio: number;
  depth: number;
  beat: number;
  soften: number;
}

export interface MidiOut {
  channel: number;
  program: number | null;
  bank: number | null;
}

export interface Phrase {
  cells: Cell[];
  looped: boolean;
  base: number;
}

export interface Instrument {
  name: string;
  filename: string;
  /** For each of 120 notes: [note played (0-based), 1-based sample]. */
  keymap: [number, number][];
  volumeEnv: Envelope;
  panEnv: Envelope;
  pitchEnv: Envelope;
  fadeout: number;
  globalVolume: number;
  pan: number | null;
  nna: number;
  dct: number;
  dca: number;
  pitchPanSeparation: number;
  pitchPanCenter: number;
  randomVolume: number;
  randomPan: number;
  cutoff: number | null;
  resonance: number | null;
  midi: MidiOut | null;
  effect: number | null;
  phrase: Phrase | null;
}

export interface Target {
  /** none, globalVolume, tempo, channelVolume, channelPan, effect. */
  kind: string;
  /** The channel, or the effect slot. */
  a: number;
  /** The effect's parameter. */
  b: number;
}

export interface Lane {
  target: Target;
  /** [row, value 0 to 1]. */
  points: [number, number][];
}

export interface EffectSlot {
  kind: number;
  name: string;
  params: number[];
  mix: number;
  bypass: boolean;
  output: number | null;
}

export interface Bookmark {
  name: string;
  order: number | null;
  pattern: number | null;
  row: number;
}

export interface Macros {
  parametered: string[];
  fixed: string[];
}

export interface Doc {
  name: string;
  /** The file it was opened from or saved to, where the platform has paths. */
  path: string | null;
  dirty: boolean;
  title: string;
  artist: string;
  message: string;
  format: string;
  formatName: string;
  tracker: string;
  kind: string;
  linear: boolean;
  instrumentsMode: boolean;
  speed: number;
  tempo: number;
  globalVolume: number;
  mixVolume: number;
  restart: number;
  channels: Channel[];
  orders: number[];
  patterns: { rows: number; name: string; time: [number, number] | null; swing: number | null; lanes: number }[];
  samples: Sample[];
  instruments: Instrument[];
  canUndo: boolean;
  canRedo: boolean;
  missing: number;
  /** 0 classic, 1 alternative, 2 modern. */
  tempoMode: number;
  rowsPerBeat: number;
  rowsPerMeasure: number;
  swing: number;
  sequences: string[];
  sequence: number;
  bookmarks: Bookmark[];
  /** [order, channel] slots of the matrix muted. */
  slotMutes: [number, number][];
  effects: EffectSlot[];
  macros: Macros | null;
}

/** A pattern: six numbers a cell (note, instrument, volume kind and value, effect and parameter), row by row. */
export interface Pattern {
  index: number;
  rows: number;
  channels: number;
  name: string;
  cells: number[];
  rowsPerBeat: number;
  rowsPerMeasure: number;
  lanes: Lane[];
}

export type Cell = [number, number, number, number, number, number];

export interface Opened {
  doc: Doc;
  /** Samples whose waveforms weren't in the file (a UMF saved without them). */
  missing: number;
}

export interface Edited {
  doc: Doc;
  pattern: Pattern | null;
}

export interface Status {
  playing: boolean;
  order: number;
  row: number;
  levels: number[];
}

export interface Letters {
  /** [code, letter, what it does]. */
  effects: [number, string, string][];
  volumes: [number, string][];
  /** [id, name, extensions]. */
  formats: [string, string, string[]][];
  /** The effect slots' kinds: name, and each parameter's [name, default]. */
  effectKinds: [string, [string, number][]][];
  sampleReads: string[];
  /** [extension, name]. */
  sampleWrites: [string, string][];
}

export type Edit =
  | { op: "cells"; pattern: number; row: number; channel: number; width: number; cells: Cell[] }
  | { op: "clear"; pattern: number; row: number; channel: number; rows: number; channels: number; mask: number }
  | { op: "transpose"; pattern: number; row: number; channel: number; rows: number; channels: number; semitones: number }
  | { op: "setInstrument"; pattern: number; row: number; channel: number; rows: number; channels: number; instr: number }
  | { op: "insertRows"; pattern: number; row: number; channel: number; channels: number; count: number }
  | { op: "deleteRows"; pattern: number; row: number; channel: number; channels: number; count: number }
  | { op: "patternRows"; pattern: number; rows: number }
  | { op: "patternName"; pattern: number; name: string }
  | { op: "addPattern" }
  | { op: "clonePattern"; pattern: number }
  | { op: "orders"; orders: number[] }
  | { op: "song"; title: string; artist: string; message: string; speed: number; tempo: number; globalVolume: number; mixVolume: number; restart: number; linear: boolean }
  | { op: "channel"; index: number; name: string; pan: number; volume: number; surround: boolean }
  | { op: "addChannel" }
  | { op: "removeChannel"; index: number }
  | ({ op: "sample"; index: number } & Omit<Sample, "filename" | "frames" | "bits" | "channels" | "slices" | "synth">)
  | { op: "addSample" }
  | { op: "clearSample"; index: number }
  | { op: "normalize"; index: number }
  | { op: "reverse"; index: number }
  | { op: "trimToLoop"; index: number }
  | { op: "instrument"; index: number; name: string; globalVolume: number; fadeout: number; pan: number | null; nna: number; volumeEnv: Envelope; panEnv: Envelope; pitchEnv: Envelope; more: InstrumentMore }
  | { op: "keymapAll"; index: number; sample: number }
  | { op: "keymapRange"; index: number; from: number; to: number; sample: number }
  | { op: "addInstrument" }
  | { op: "instrumentsMode"; on: boolean }
  | { op: "keymapNote"; index: number; note: number; toNote: number; sample: number }
  | { op: "interpolate"; pattern: number; row: number; channel: number; rows: number; channels: number; effect: boolean }
  | { op: "amplify"; pattern: number; row: number; channel: number; rows: number; channels: number; from: number; to: number }
  | { op: "paste"; pattern: number; row: number; channel: number; width: number; cells: Cell[]; mode: PasteMode }
  | { op: "replaceInstrument"; from: number; to: number; pattern: number | null; row: number; channel: number; rows: number; channels: number }
  | { op: "swapInstruments"; a: number; b: number }
  | { op: "patternTime"; pattern: number; time: [number, number] | null; swing: number | null }
  | { op: "timing"; tempoMode: number; rowsPerBeat: number; rowsPerMeasure: number; swing: number }
  | { op: "addSequence"; name: string; copy: boolean }
  | { op: "removeSequence"; index: number }
  | { op: "switchSequence"; index: number }
  | { op: "renameSequence"; index: number; name: string }
  | { op: "bookmarks"; bookmarks: Bookmark[] }
  | { op: "slotMute"; order: number; channel: number; muted: boolean }
  | { op: "lanes"; pattern: number; lanes: Lane[] }
  | { op: "effects"; effects: EffectSlot[] }
  | { op: "channelEffect"; index: number; effect: number | null }
  | { op: "macros"; macros: Macros | null }
  | { op: "sampleEdit"; index: number; edit: SampleEditOp; from: number; to: number; value: number }
  | { op: "sampleFormat"; index: number; bits: number; channels: number }
  | { op: "sampleDraw"; index: number; at: number; points: number[] }
  | { op: "sampleSlices"; index: number; slices: number[] }
  | { op: "slicesToInstrument"; index: number; first: number }
  | { op: "synth"; index: number; synth: Synth }
  | { op: "cleanup"; patterns: boolean; samples: boolean; instruments: boolean };

export type PasteMode = "overwrite" | "mix" | "mixCells" | "flood" | "insert";

export type SampleEditOp = "cut" | "crop" | "silence" | "fadeIn" | "fadeOut" | "gain" | "normalize" | "reverse" | "dc" | "crossfade" | "smoothLoop" | "stretch" | "pitch" | "resample" | "slices" | "clearSlices";

export interface InstrumentMore {
  dct: number;
  dca: number;
  pitchPanSeparation: number;
  pitchPanCenter: number;
  randomVolume: number;
  randomPan: number;
  cutoff: number | null;
  resonance: number | null;
  midi: MidiOut | null;
  effect: number | null;
  phrase: Phrase | null;
}

export const moreOf = (i: Instrument): InstrumentMore => ({
  dct: i.dct,
  dca: i.dca,
  pitchPanSeparation: i.pitchPanSeparation,
  pitchPanCenter: i.pitchPanCenter,
  randomVolume: i.randomVolume,
  randomPan: i.randomPan,
  cutoff: i.cutoff,
  resonance: i.resonance,
  midi: i.midi,
  effect: i.effect,
  phrase: i.phrase,
});

/** What find looks for: each part given must match. */
export interface Query {
  notes?: [number, number] | null;
  instr?: number | null;
  vol?: number | null;
  volp?: [number, number] | null;
  fx?: number | null;
  param?: [number, number] | null;
}

export interface Replacement {
  note?: number | null;
  transpose?: number;
  instr?: number | null;
  vol?: [number, number] | null;
  fx?: [number, number] | null;
  clear?: boolean;
}

export interface Scope {
  pattern: number | null;
  block: [number, number, number, number] | null;
}

export interface Stats {
  seconds: number;
  patterns: number;
  orders: number;
  notes: number;
  unusedPatterns: number[];
  emptyPatterns: number[];
  unusedSamples: number[];
  unusedInstruments: number[];
  unusedBytes: number;
}

export const letters = () => call<Letters>("letters");
/** A module or UMF opened from its bytes; `companions` are the files beside it (a MED song's instruments, StarTrekker's .NT). */
export async function openBytes(name: string, bytes: Uint8Array, companions: { name: string; bytes: Uint8Array }[] = []): Promise<Opened> {
  for (const c of companions) await call("companion", { name: c.name }, c.bytes);
  return call<Opened>("open", { name }, bytes);
}
/** A module opened by its path, the files beside it found too (the desktop's). */
export async function openPath(path: string): Promise<Opened> {
  const h = await host();
  if (!h.openPath) throw new Error("Files are opened by their contents here.");
  return (await h.openPath(path)).json as Opened;
}
export const fresh = (channels: number) => call<Doc>("new", { channels });
export const doc = () => call<Doc | null>("doc");
export const pattern = (index: number) => call<Pattern | null>("pattern", { index });
/** The song as an NTT (a UMF with its samples), named `name`; the song as it is once saved. */
export const save = (name: string, path: string | null) => file<Doc>("save", { name, path });
/** The song as an NTT, nothing changed; `saved` once it's written. */
export const snapshot = () => file<string>("snapshot");
export const saved = (name: string, path: string | null) => call<Doc>("saved", { name, path });
export const exportWav = () => file<null>("exportWav");
/** Each channel as a WAV file, in a zip; the files' names. */
export const exportStems = () => file<string[]>("exportStems");
/** The song as MOD, S3M, XM or IT, and what the format couldn't keep, with how often. */
export const exportModule = (format: string) => file<[string, number][]>("exportModule", { format });
export const edit = (edits: Edit[], show: number | null) => call<Edited>("edit", { edits, show });
export const undo = (redo: boolean, show: number | null) => call<Edited>("undo", { redo, show });
export const play = (order: number, row: number, hold: boolean) => call<null>("play", { order, row, hold });
export const stop = () => call<null>("stop");
export const jam = (note: number, instr: number) => call<null>("jam", { note, instr });
export const mute = (channel: number, muted: boolean) => call<null>("mute", { channel, muted });
export const status = () => call<Status>("status");
export const samplePeaks = (index: number, width: number) => call<[number, number][]>("samplePeaks", { index, width });
export const samplePeaksRange = (index: number, width: number, from: number, to: number) => call<[number, number][]>("samplePeaks", { index, width, from, to });
export const loadSample = (index: number, name: string, bytes: Uint8Array, show: number | null) =>
  call<{ edited: Edited; samples: number; instrument: number | null }>("loadSample", { index, name, show }, bytes);
/** A sample as a file of the kind `ext` names, and what that kind couldn't keep. */
export const saveSample = (index: number, ext: string) => file<string[]>("saveSample", { index, ext });
export const saveInstrument = (index: number) => file<null>("saveInstrument", { index });
export const sampleCopy = (index: number, from: number, to: number, cut: boolean, show: number | null) => call<Edited | null>("sampleCopy", { index, from, to, cut, show });
export const samplePaste = (index: number, at: number, show: number | null) => call<Edited>("samplePaste", { index, at, show });
export const sampleAudition = (index: number, from: number, to: number | null) => call<null>("sampleAudition", { index, from, to });
export const playRow = (order: number, row: number) => call<null>("playRow", { order, row });
export const fader = (channel: number, gain: number, pan: number | null) => call<null>("fader", { channel, gain, pan });
export const playback = (interp: number, amiga: boolean) => call<null>("playback", { interp, amiga });
export const find = (query: Query, scope: Scope) => call<[number, number, number][]>("find", { query, scope });
export const replace = (query: Query, withThis: Replacement, scope: Scope, show: number | null) => call<{ count: number; edited: Edited }>("replace", { query, with: withThis, scope, show });
export const stats = () => call<Stats>("stats");

// ---- Notes and effects as trackers write them ----

export const NOTE_OFF = 255;
export const NOTE_CUT = 254;
export const NOTE_FADE = 253;
const NAMES = ["C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-"];
const SPOKEN = ["C", "C sharp", "D", "D sharp", "E", "F", "F sharp", "G", "G sharp", "A", "A sharp", "B"];

/** A note as trackers write it: C-5 is middle C (61). */
export function noteText(n: number): string {
  if (n === 0) return "···";
  if (n === NOTE_OFF) return "===";
  if (n === NOTE_CUT) return "^^^";
  if (n === NOTE_FADE) return "~~~";
  if (n >= 1 && n <= 120) return `${NAMES[(n - 1) % 12]}${Math.floor((n - 1) / 12)}`;
  return "???";
}

export function noteWords(n: number): string {
  if (n === 0) return "no note";
  if (n === NOTE_OFF) return "note off";
  if (n === NOTE_CUT) return "note cut";
  if (n === NOTE_FADE) return "note fade";
  return `${SPOKEN[(n - 1) % 12]} ${Math.floor((n - 1) / 12)}`;
}

export const hex2 = (v: number) => v.toString(16).toUpperCase().padStart(2, "0");

/** The piano on the computer's keyboard, as FastTracker and Impulse Tracker lay it out: semitones up from the octave chosen. */
export const PIANO: Record<string, number> = {
  KeyZ: 0, KeyS: 1, KeyX: 2, KeyD: 3, KeyC: 4, KeyV: 5, KeyG: 6, KeyB: 7, KeyH: 8, KeyN: 9, KeyJ: 10, KeyM: 11,
  Comma: 12, KeyL: 13, Period: 14, Semicolon: 15, Slash: 16,
  KeyQ: 12, Digit2: 13, KeyW: 14, Digit3: 15, KeyE: 16, KeyR: 17, Digit5: 18, KeyT: 19, Digit6: 20, KeyY: 21, Digit7: 22, KeyU: 23,
  KeyI: 24, Digit9: 25, KeyO: 26, Digit0: 27, KeyP: 28,
};

export function bytes(n: number): string {
  if (n < 1024) return `${n} bytes`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10240 ? 1 : 0)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 / 1024).toFixed(n < 10 * 1024 * 1024 ? 2 : 1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}

export function seconds(s: number): string {
  if (s < 1) return `${Math.round(s * 1000)} ms`;
  if (s < 60) return `${s.toFixed(s < 10 ? 2 : 1)} s`;
  return `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
}

/** What the panel remembers between launches (a convenience: it starts at the defaults without it). */
const KEY = "neumetik-tracker.settings";

export interface Remembered {
  octave: number;
  step: number;
  follow: boolean;
  /** 0 none, 1 linear, 2 cubic, 3 sinc. */
  interp: number;
  amiga: boolean;
  /** Chords typed across channels (FastTracker 2's multichannel record). */
  spread: boolean;
  /** A MIDI note's velocity to the volume column. */
  velocity: boolean;
  /** The chord each key writes: semitones over the note, none for one note. */
  chord: number[];
  /** Notes written with the template's volume and effect (OpenMPT's). */
  template: Cell | null;
  /** The MIDI keyboard chosen, where the platform has MIDI. */
  midiIn: string | null;
}

export const DEFAULTS: Remembered = { octave: 4, step: 1, follow: true, interp: 2, amiga: false, spread: false, velocity: true, chord: [], template: null, midiIn: null };

/** Chords a key can write across channels. */
export const CHORDS: [string, number[]][] = [
  ["One note", []],
  ["Major", [4, 7]],
  ["Minor", [3, 7]],
  ["Seventh", [4, 7, 10]],
  ["Major seventh", [4, 7, 11]],
  ["Minor seventh", [3, 7, 10]],
  ["Suspended", [5, 7]],
  ["Power", [7, 12]],
  ["Octave", [12]],
];

export const INTERPS = ["None (the Amiga's)", "Linear (FastTracker 2's)", "Cubic (Impulse Tracker's)", "Windowed sinc (OpenMPT's)"];
export const TEMPO_MODES = ["Classic", "Alternative", "Modern"];

export function recall(): Remembered {
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? { ...DEFAULTS, ...(JSON.parse(raw) as Partial<Remembered>) } : DEFAULTS;
  } catch {
    return DEFAULTS;
  }
}

export function remember(r: Remembered) {
  try {    localStorage.setItem(KEY, JSON.stringify(r));
  } catch {
    // Not remembered; it still works.
  }
}
