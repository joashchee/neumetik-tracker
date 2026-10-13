/**
 * **The tracker**: a module opened, played and edited.
 *
 * - Open… reads a module in any format neumetik-tracker reads (MOD, S3M,
 *   XM, IT and some eighty more) or a song saved here. Save… saves the
 *   song as an NTT (a UMF with its samples): everything it is, kept.
 *   Export… writes a WAV, each channel's WAV, or MOD, S3M, XM or IT.
 * - The editor works as FastTracker 2 and Impulse Tracker did: the order
 *   list, the pattern (a row a step, a column a channel: note,
 *   instrument, volume column, effect), samples and instruments.
 *
 * The pattern's keys (the grid has the focus):
 *
 * | Keys | |
 * | --- | --- |
 * | Arrows, Tab, Page Up/Down, Home/End | move |
 * | Space | edit on or off (off, the piano keys only play) |
 * | Z S X D C V G B H N J M, Q 2 W 3 E R 5 T 6 Y 7 U I 9 O 0 P | notes, two octaves from the octave chosen (a chord, or across channels, as Settings say) |
 * | ` (Shift: fade), 1 | note off, note cut |
 * | 4, 8 (on the note column) | play the note under the cursor, the row (Impulse Tracker's) |
 * | 0-9 A-F | instrument, volume and effect values; letters the volume command and effect (Shift for the lowercase ones) |
 * | [ and ] | the octave down and up |
 * | Backspace, Delete | clears what's under the cursor (or the block) |
 * | Shift+Return, Shift+Backspace | puts in a row, takes one out (with Cmd, in every channel) |
 * | Shift+arrows, Cmd+A | marks a block (Cmd+A again: the pattern) |
 * | Cmd+C, Cmd+X, Cmd+V | copy, cut, paste (Cmd+Shift+V mixes it in, Cmd+Alt+V pushes the rest down) |
 * | Cmd+Up/Down (Shift: an octave) | transposes the note or the block |
 * | Alt+K, Alt+X, Alt+J | the block's volumes or effect values interpolated; its volumes amplified or faded |
 * | Cmd+E, Cmd+T, Cmd+F, Cmd+B | the effect in words; the cell as the template; find and replace; a bookmark here |
 * | Cmd+Z, Cmd+Shift+Z | undo, redo |
 * | F5 F6 F7 F8 (Shift+F7: in the pattern) | play the song, the pattern, from the cursor; stop |
 *
 * A MIDI keyboard (Settings…, where the platform has MIDI) plays and
 * writes as the computer's does; on a phone, the keys under the pattern
 * (TouchKeys.tsx) do.
 *
 * The song itself is the session (session/src/lib.rs), through lib/tracker.ts.
 */
import { useCallback, useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import type { ActivityUpdate } from "../lib/activity";
import { host } from "../lib/host";
import * as midi from "../lib/midi";
import * as tk from "../lib/tracker";
import { Dialog } from "./Dialog";
import { AmplifyDialog, EffectDialog, ExportDialog, FindDialog, MacrosDialog, SettingsDialog, StatsDialog, effectWords } from "./TrackerDialogs";
import { InstrumentPane } from "./TrackerInstruments";
import { AutomationPane, MixPane } from "./TrackerMixer";
import { SamplePane } from "./TrackerSamples";
import { SongPane } from "./TrackerSong";
import { TouchKeys } from "./TouchKeys";

interface TrackerPanelProps {
  runActivity: <T>(label: string, task: (update: ActivityUpdate) => Promise<T>, key?: string) => Promise<T>;
  isBusy: (key: string) => boolean;
  showStatus: (message: string) => void;
  showError: (message: string) => void;
  audioReady: boolean;
  /** The panel is showing: only then does it take the keyboard. */
  active: boolean;
  /** Whether its song is playing. */
  onPlaying: (playing: boolean) => void;
  /** A phone or tablet: the keys under the pattern are shown. */
  touch: boolean;
}

/** Rows the grid shows at once, and the cursor's columns in a cell. */
const VISIBLE = 28;
const COLS = 9;
/** Where each cursor column is in a cell's text ("C-5 01 v40 A0F"). */
const COL_AT = [0, 4, 5, 7, 8, 9, 11, 12, 13];
const COL_WIDTH = [3, 1, 1, 1, 1, 1, 1, 1, 1];
/** Which field each cursor column edits: 0 note, 1 instrument, 2 volume, 3 effect. */
const FIELD = [0, 1, 1, 2, 2, 2, 3, 3, 3];
const FIELD_MASK = [1 | 2, 2, 4, 8];

type Cursor = { row: number; ch: number; col: number };
type Block = { row: number; ch: number; rows: number; channels: number };
type Ask = null | { discard: () => void } | "export" | "find" | "settings" | "amplify" | "effect" | "stats" | "macros";
type Side = "samples" | "instruments" | "song" | "mix" | "automation";

const EMPTY: tk.Cell = [0, 0, 0, 0, 0, 0];

export function TrackerPanel({ runActivity, isBusy, showStatus, showError, audioReady, active, onPlaying, touch }: TrackerPanelProps) {
  const id = useId();
  const [remembered, setRemembered] = useState<tk.Remembered>(tk.recall);
  const { octave, step, follow, interp, amiga, spread, chord, template, velocity } = remembered;
  const choose = useCallback((patch: Partial<tk.Remembered>) => setRemembered((r) => ({ ...r, ...patch })), []);
  useEffect(() => tk.remember(remembered), [remembered]);

  const [letters, setLetters] = useState<tk.Letters | null>(null);
  const [doc, setDoc] = useState<tk.Doc | null>(null);
  const [pat, setPat] = useState<tk.Pattern | null>(null);
  const [orderPos, setOrderPos] = useState(0);
  const [patIndex, setPatIndex] = useState(0);
  const [cursor, setCursor] = useState<Cursor>({ row: 0, ch: 0, col: 0 });
  const [anchor, setAnchor] = useState<{ row: number; ch: number } | null>(null);
  const [editing, setEditing] = useState(true);
  const [instrument, setInstrument] = useState(1);
  const [status, setStatus] = useState<tk.Status | null>(null);
  const [muted, setMuted] = useState<Set<number>>(new Set());
  const [side, setSide] = useState<Side>("samples");
  const [matrix, setMatrix] = useState(false);
  const [jump, setJump] = useState("");
  const [sample, setSample] = useState(0);
  const [ins, setIns] = useState(0);
  const [ask, setAsk] = useState<Ask>(null);
  const [top, setTop] = useState(0);
  const [said, setSaid] = useState("");
  const clip = useRef<{ width: number; cells: tk.Cell[] } | null>(null);
  const grid = useRef<HTMLDivElement>(null);
  const held = useRef(new Set<string>());
  /** Keys (and MIDI notes) held, in the order they went down: where each wrote its note. */
  const chordHeld = useRef(new Map<string, number>());

  // How samples are read, to the player whenever it's chosen (and at the start).
  useEffect(() => {
    if (audioReady) tk.playback(interp, amiga).catch(() => {});
  }, [audioReady, interp, amiga]);
  // The MIDI keyboard chosen, listened to (where the platform has MIDI).
  const midiIn = remembered.midiIn;
  useEffect(() => {
    void midi.listenTo(midiIn, (n) => midiRef.current(n));
    return () => void midi.listenTo(null, () => {});
  }, [midiIn]);

  const fail = useCallback((e: unknown) => showError(String(e)), [showError]);
  useEffect(() => {
    tk.letters().then(setLetters, fail);
    tk.doc().then((d) => d && setDoc(d), fail);
  }, [fail]);

  const playing = status?.playing ?? false;
  useEffect(() => onPlaying(playing), [playing, onPlaying]);

  const fxLetter = useMemo(() => new Map((letters?.effects ?? []).map(([c, l]) => [c, l])), [letters]);
  const fxCode = useMemo(() => new Map((letters?.effects ?? []).map(([c, l]) => [l, c])), [letters]);
  const fxWords = useMemo(() => new Map((letters?.effects ?? []).map(([c, , w]) => [c, w])), [letters]);
  const volLetter = useMemo(() => new Map((letters?.volumes ?? []).map(([c, l]) => [c, l])), [letters]);
  const volCode = useMemo(() => new Map((letters?.volumes ?? []).map(([c, l]) => [l, c])), [letters]);

  // The pattern shown follows the order chosen.
  const shownPattern = doc ? (doc.orders[orderPos] !== undefined && doc.orders[orderPos] < doc.patterns.length ? doc.orders[orderPos] : patIndex) : 0;
  useEffect(() => {
    if (!doc) {
      setPat(null);
      return;
    }
    tk.pattern(shownPattern).then(setPat, fail);
  }, [doc, shownPattern, fail]);
  useEffect(() => setPatIndex(shownPattern), [shownPattern]);

  const rows = pat?.rows ?? 64;
  const channels = pat?.channels ?? doc?.channels.length ?? 4;
  const cellAt = useCallback((row: number, ch: number): tk.Cell => {
    if (!pat || row < 0 || row >= pat.rows || ch < 0 || ch >= pat.channels) return EMPTY;
    const at = (row * pat.channels + ch) * 6;
    return pat.cells.slice(at, at + 6) as tk.Cell;
  }, [pat]);

  // ---- Status: where the song is, each channel's level ----
  // Hidden, it still looks (less often) while the song plays, so the
  // tab's green dot goes out when the song ends.
  useEffect(() => {
    if (!(active || playing) || !audioReady || !doc) return;
    let stop = false;
    const look = () => {
      if (stop) return;
      tk.status().then((s) => {
        if (!stop) setStatus(s);
      }, () => {});
    };
    look();
    const timer = window.setInterval(look, playing && active ? 40 : 250);
    return () => {
      stop = true;
      window.clearInterval(timer);
    };
  }, [active, audioReady, doc, playing]);

  // Following the song: its order, its row in view.
  useEffect(() => {
    if (!follow || !playing || !status) return;
    setOrderPos(status.order);
    setTop((t) => (status.row < t || status.row >= t + VISIBLE ? Math.max(0, status.row - Math.floor(VISIBLE / 3)) : t));
  }, [follow, playing, status]);

  // The cursor kept in view.
  useEffect(() => {
    setTop((t) => (cursor.row < t ? cursor.row : cursor.row >= t + VISIBLE ? cursor.row - VISIBLE + 1 : t));
  }, [cursor.row]);
  useEffect(() => {
    setCursor((c) => ({ ...c, row: Math.min(c.row, rows - 1), ch: Math.min(c.ch, channels - 1) }));
  }, [rows, channels]);

  // ---- Opening, new, saving ----

  function loaded(d: tk.Doc) {
    setDoc(d);
    setOrderPos(0);
    setCursor({ row: 0, ch: 0, col: 0 });
    setAnchor(null);
    setTop(0);
    setMuted(new Set());
    setSample(0);
    setIns(0);
    setInstrument(1);
  }

  const unlessDirty = (then: () => void) => (doc?.dirty ? setAsk({ discard: then }) : then());

  function opened(o: tk.Opened, name: string) {
    loaded(o.doc);
    if (o.missing > 0) showError(`Opened ${name}, but ${o.missing} of its samples weren't saved in it: they're silent.`);
    else showStatus(`Opened ${name} (${o.doc.formatName}${o.doc.tracker ? `, ${o.doc.tracker}` : ""}).`);
  }

  async function chooseAndOpen() {
    const exts = Array.from(new Set((letters?.formats ?? []).flatMap(([, , e]) => e)));
    try {
      const h = await host();
      const picked = await h.pick([{ name: "Tracker modules", extensions: exts }], !h.openPath);
      if (picked.length === 0) return;
      // Picked with the files beside it (a MED song's instruments, StarTrekker's .NT): the module is the one a format knows.
      const known = (n: string) => exts.some((e) => n.toLowerCase().endsWith(`.${e}`) || n.toLowerCase().startsWith(`${e}.`));
      const main = picked.find((f) => known(f.name)) ?? picked[0];
      const o = await runActivity(`Opening ${main.name}…`, async () => {
        if (main.path && h.openPath) return tk.openPath(main.path);
        const others = await Promise.all(picked.filter((f) => f !== main).map(async (f) => ({ name: f.name, bytes: await f.bytes() })));
        return tk.openBytes(main.name, await main.bytes(), others);
      }, "tracker-open");
      opened(o, main.name);
    } catch (e) {
      showError(String(e));
    }
  }

  function newSong() {
    unlessDirty(() => {
      tk.fresh(8).then((d) => {
        loaded(d);
        showStatus("A new song: eight channels, one pattern of 64 rows, eight empty samples. Load… in Samples fills a sample.");
      }, fail);
    });
  }

  const songStem = () => (doc?.name || "song").replace(/\.[^.]+$/, "");

  async function saveSong() {
    if (!doc) return;
    try {
      const name = `${songStem()}.ntt`;
      const [, bytes] = await runActivity(`Saving ${name}…`, () => tk.snapshot(), "tracker-save");
      const to = await (await host()).save(name, bytes, { name: "Neumetik Tracker song (NTT)", extensions: ["ntt"] });
      if (!to) return;
      setDoc(await tk.saved(to, null));
      showStatus(`Saved ${to}: the song with its samples, everything it is, to open here again or in any NTT player.`);
    } catch (e) {
      showError(String(e));
    }
  }

  async function exportWav() {
    if (!doc) return;
    setAsk(null);
    try {
      const [, bytes] = await runActivity("Exporting the song as WAV…", () => tk.exportWav(), "tracker-export");
      const to = await (await host()).save(`${songStem()}.wav`, bytes, { name: "WAV audio", extensions: ["wav"] });
      if (to) showStatus(`Exported ${to}: the song played once through, at 44.1 kHz.`);
    } catch (e) {
      showError(String(e));
    }
  }

  async function exportStems() {
    if (!doc) return;
    setAsk(null);
    try {
      const [files, bytes] = await runActivity("Exporting each channel…", () => tk.exportStems(), "tracker-export");
      const to = await (await host()).save(`${songStem()} channels.zip`, bytes, { name: "Zip of WAV files", extensions: ["zip"] });
      if (to) showStatus(`Exported ${files.length} ${files.length === 1 ? "channel" : "channels"} in ${to}: ${files.slice(0, 3).join(", ")}${files.length > 3 ? "…" : ""}.`);
    } catch (e) {
      showError(String(e));
    }
  }

  async function exportModule(format: string) {
    if (!doc) return;
    setAsk(null);
    try {
      const [lost, bytes] = await runActivity(`Exporting as ${format.toUpperCase()}…`, () => tk.exportModule(format), "tracker-export");
      const to = await (await host()).save(`${songStem()}.${format}`, bytes, { name: format.toUpperCase(), extensions: [format] });
      if (!to) return;
      if (lost.length === 0) showStatus(`Exported ${to}: everything kept.`);
      else showError(`Exported ${to}, but ${format.toUpperCase()} couldn't keep: ${lost.map(([w, n]) => (n > 1 ? `${w} (${n})` : w)).join("; ")}.`);
    } catch (e) {
      showError(String(e));
    }
  }

  // ---- Editing ----

  const run = useCallback(
    (edits: tk.Edit[], then?: (d: tk.Doc) => void) => {
      tk.edit(edits, patIndex).then((r) => {
        setDoc(r.doc);
        if (r.pattern) setPat(r.pattern);
        then?.(r.doc);
      }, fail);
    },
    [patIndex, fail],
  );

  const undoRedo = useCallback(
    (redo: boolean) => {
      tk.undo(redo, patIndex).then((r) => {
        setDoc(r.doc);
        if (r.pattern) setPat(r.pattern);
        showStatus(redo ? "Redone." : "Undone.");
      }, fail);
    },
    [patIndex, fail, showStatus],
  );

  const block: Block | null = anchor ? { row: Math.min(anchor.row, cursor.row), ch: Math.min(anchor.ch, cursor.ch), rows: Math.abs(anchor.row - cursor.row) + 1, channels: Math.abs(anchor.ch - cursor.ch) + 1 } : null;
  const inBlock = (row: number, ch: number) => !!block && row >= block.row && row < block.row + block.rows && ch >= block.ch && ch < block.ch + block.channels;

  function putCell(row: number, ch: number, cell: tk.Cell) {
    run([{ op: "cells", pattern: patIndex, row, channel: ch, width: 1, cells: [cell] }]);
  }

  function moveDown(by = step) {
    setCursor((c) => ({ ...c, row: Math.min(rows - 1, c.row + by) }));
  }

  function playNote(note: number, instr = instrument) {
    if (audioReady) tk.jam(note, instr).catch(fail);
  }

  /**
   * A note typed (or played on a MIDI keyboard, `key` its name): its chord
   * written across channels from the cursor's, with the template's volume
   * and effect, or the velocity's volume. Held with others and Chords
   * across channels on, it goes in the next channel along; the cursor
   * moves down once they're all let go.
   */
  function enterNote(key: string, note: number, vel: number | null) {
    if (!pat) return;
    const offset = spread ? chordHeld.current.size : 0;
    chordHeld.current.set(key, offset);
    const notes = [note, ...chord.map((c) => Math.min(120, note + c))];
    const cells: tk.Cell[] = [];
    let width = 0;
    for (let i = 0; i < notes.length; i++) {
      const ch = cursor.ch + offset + i;
      if (ch >= channels) break;
      const old = cellAt(cursor.row, ch);
      const c: tk.Cell = [notes[i], instrument, old[2], old[3], old[4], old[5]];
      if (template) {
        if (template[2]) [c[2], c[3]] = [template[2], template[3]];
        if (template[4]) [c[4], c[5]] = [template[4], template[5]];
      }
      if (vel !== null && velocity) [c[2], c[3]] = [1, Math.max(1, Math.round((vel * 64) / 127))];
      cells.push(c);
      width++;
    }
    if (width > 0) run([{ op: "cells", pattern: patIndex, row: cursor.row, channel: cursor.ch + offset, width, cells }]);
    if (!spread) moveDown();
  }

  /** A key (or MIDI note) let go: with Chords across channels, the row's done once none are held. */
  function leaveNote(key: string) {
    if (!chordHeld.current.delete(key)) return;
    if (spread && chordHeld.current.size === 0 && editing) moveDown();
  }

  function copyBlock(cut: boolean) {
    const b = block ?? { row: cursor.row, ch: cursor.ch, rows: 1, channels: 1 };
    const cells: tk.Cell[] = [];
    for (let r = b.row; r < b.row + b.rows; r++) for (let c = b.ch; c < b.ch + b.channels; c++) cells.push(cellAt(r, c));
    clip.current = { width: b.channels, cells };
    if (cut) run([{ op: "clear", pattern: patIndex, row: b.row, channel: b.ch, rows: b.rows, channels: b.channels, mask: 15 }]);
    showStatus(`${cut ? "Cut" : "Copied"} ${b.rows} ${b.rows === 1 ? "row" : "rows"} of ${b.channels} ${b.channels === 1 ? "channel" : "channels"}.`);
  }

  function paste() {
    const c = clip.current;
    if (!c) return showStatus("Nothing's been copied yet: mark a block (Shift+arrows) and Cmd+C.");
    run([{ op: "cells", pattern: patIndex, row: cursor.row, channel: cursor.ch, width: c.width, cells: c.cells }]);
  }

  function transpose(by: number) {
    const b = block ?? { row: cursor.row, ch: cursor.ch, rows: 1, channels: 1 };
    run([{ op: "transpose", pattern: patIndex, row: b.row, channel: b.ch, rows: b.rows, channels: b.channels, semitones: by }]);
  }

  function clearHere() {
    if (block) {
      run([{ op: "clear", pattern: patIndex, row: block.row, channel: block.ch, rows: block.rows, channels: block.channels, mask: 15 }]);
      return;
    }
    run([{ op: "clear", pattern: patIndex, row: cursor.row, channel: cursor.ch, rows: 1, channels: 1, mask: FIELD_MASK[FIELD[cursor.col]] }]);
    moveDown();
  }

  function play(kind: "song" | "pattern" | "cursor" | "cursorPattern") {
    if (!doc || !audioReady) return;
    const [order, row, hold] = kind === "song" ? [0, 0, false] : kind === "pattern" ? [orderPos, 0, true] : [orderPos, cursor.row, kind === "cursorPattern"];
    const said = { song: "Playing the song.", pattern: `Playing pattern ${patIndex}, over and over.`, cursor: `Playing from row ${cursor.row}.`, cursorPattern: `Playing pattern ${patIndex} from row ${cursor.row}, over and over.` }[kind];
    tk.play(order, row, hold).then(() => showStatus(touch ? said : `${said} F8 stops it.`), fail);
  }

  const blockOrCursor = (): Block => block ?? { row: cursor.row, ch: cursor.ch, rows: 1, channels: 1 };

  function interpolate(effect: boolean) {
    const b = blockOrCursor();
    if (b.rows < 3) return showStatus("Mark a block of three rows or more (Shift+arrows): its first and last rows are what's between.");
    run([{ op: "interpolate", pattern: patIndex, row: b.row, channel: b.ch, rows: b.rows, channels: b.channels, effect }]);
  }

  function pasteAs(mode: tk.PasteMode) {
    const c = clip.current;
    if (!c) return showStatus("Nothing's been copied yet: mark a block (Shift+arrows) and Cmd+C.");
    run([{ op: "paste", pattern: patIndex, row: cursor.row, channel: cursor.ch, width: c.width, cells: c.cells, mode }]);
  }

  function addBookmark() {
    if (!doc) return;
    const name = `Pattern ${patIndex}, row ${cursor.row}`;
    run([{ op: "bookmarks", bookmarks: [...doc.bookmarks, { name, order: orderPos, pattern: patIndex, row: cursor.row }] }]);
    showStatus(`Bookmarked ${name}.`);
  }

  const stop = useCallback(() => {
    tk.stop().catch(fail);
  }, [fail]);

  // Hidden, the song plays on (as MIDI does), but a note held from the
  // keyboard lets go: its key can come up unheard.
  useEffect(() => {
    if (!active && audioReady) tk.jam(tk.NOTE_OFF, 0).catch(() => {});
  }, [active, audioReady]);

  function onGridKey(e: KeyboardEvent<HTMLDivElement>) {
    if (!doc || !pat) return;
    const cmd = e.metaKey || e.ctrlKey;
    const key = e.key;
    const moveTo = (c: Partial<Cursor>, mark: boolean) => {
      if (mark) setAnchor((a) => a ?? { row: cursor.row, ch: cursor.ch });
      else setAnchor(null);
      setCursor((cur) => ({ ...cur, ...c }));
    };
    // Playing.
    if (key === "F5" || key === "F6" || key === "F7") {
      e.preventDefault();
      return play(key === "F5" ? "song" : key === "F6" ? "pattern" : e.shiftKey ? "cursorPattern" : "cursor");
    }
    // Impulse Tracker's Alt+K and Alt+X (interpolate), Alt+J (amplify).
    if (e.altKey && !cmd && (e.code === "KeyK" || e.code === "KeyX" || e.code === "KeyJ")) {
      e.preventDefault();
      if (e.code === "KeyJ") return setAsk("amplify");
      return interpolate(e.code === "KeyX");
    }
    if (key === "F8") {
      e.preventDefault();
      return stop();
    }
    if (cmd) {
      const k = key.toLowerCase();
      if (k === "z") {
        e.preventDefault();
        return undoRedo(e.shiftKey);
      }
      if (k === "c" || k === "x") {
        e.preventDefault();
        return copyBlock(k === "x");
      }
      if (k === "v" || e.code === "KeyV") {
        e.preventDefault();
        return e.shiftKey ? pasteAs("mix") : e.altKey ? pasteAs("insert") : paste();
      }
      if (k === "f") {
        e.preventDefault();
        return setAsk("find");
      }
      if (k === "e") {
        e.preventDefault();
        return setAsk("effect");
      }
      if (k === "t") {
        e.preventDefault();
        choose({ template: cellAt(cursor.row, cursor.ch) });
        return showStatus("The cell under the cursor is the template: notes typed get its volume and effect (Settings… clears it).");
      }
      if (k === "b") {
        e.preventDefault();
        return addBookmark();
      }
      if (k === "a") {
        e.preventDefault();
        const whole = block && block.rows === rows && block.channels === 1;
        setAnchor({ row: 0, ch: whole ? 0 : cursor.ch });
        setCursor((c) => ({ ...c, row: rows - 1, ch: whole ? channels - 1 : c.ch }));
        return;
      }
      if (key === "ArrowUp" || key === "ArrowDown") {
        e.preventDefault();
        return transpose((key === "ArrowUp" ? 1 : -1) * (e.shiftKey ? 12 : 1));
      }
      if (key === "Enter" && e.shiftKey) {
        e.preventDefault();
        return run([{ op: "insertRows", pattern: patIndex, row: cursor.row, channel: 0, channels, count: 1 }]);
      }
      if (key === "Backspace" && e.shiftKey) {
        e.preventDefault();
        return run([{ op: "deleteRows", pattern: patIndex, row: cursor.row, channel: 0, channels, count: 1 }]);
      }
      return;
    }
    switch (key) {
      case "ArrowUp":
        e.preventDefault();
        return moveTo({ row: (cursor.row - 1 + rows) % rows }, e.shiftKey);
      case "ArrowDown":
        e.preventDefault();
        return moveTo({ row: (cursor.row + 1) % rows }, e.shiftKey);
      case "ArrowLeft":
        e.preventDefault();
        if (e.shiftKey) return moveTo({ ch: Math.max(0, cursor.ch - 1) }, true);
        return moveTo(cursor.col > 0 ? { col: cursor.col - 1 } : cursor.ch > 0 ? { ch: cursor.ch - 1, col: COLS - 1 } : {}, false);
      case "ArrowRight":
        e.preventDefault();
        if (e.shiftKey) return moveTo({ ch: Math.min(channels - 1, cursor.ch + 1) }, true);
        return moveTo(cursor.col < COLS - 1 ? { col: cursor.col + 1 } : cursor.ch < channels - 1 ? { ch: cursor.ch + 1, col: 0 } : {}, false);
      case "Tab":
        e.preventDefault();
        return moveTo({ ch: (cursor.ch + (e.shiftKey ? channels - 1 : 1)) % channels, col: 0 }, false);
      case "PageUp":
        e.preventDefault();
        return moveTo({ row: Math.max(0, cursor.row - 16) }, e.shiftKey);
      case "PageDown":
        e.preventDefault();
        return moveTo({ row: Math.min(rows - 1, cursor.row + 16) }, e.shiftKey);
      case "Home":
        e.preventDefault();
        return moveTo({ row: 0 }, e.shiftKey);
      case "End":
        e.preventDefault();
        return moveTo({ row: rows - 1 }, e.shiftKey);
      case " ":
        e.preventDefault();
        setEditing((v) => !v);
        return;
      case "[":
      case "]":
        e.preventDefault();
        choose({ octave: Math.max(0, Math.min(8, octave + (key === "]" ? 1 : -1))) });
        return;
      case "Escape":
        setAnchor(null);
        return;
    }
    if (key === "Enter" && e.shiftKey) {
      e.preventDefault();
      return run([{ op: "insertRows", pattern: patIndex, row: cursor.row, channel: cursor.ch, channels: 1, count: 1 }]);
    }
    if (key === "Backspace" && e.shiftKey) {
      e.preventDefault();
      return run([{ op: "deleteRows", pattern: patIndex, row: cursor.row, channel: cursor.ch, channels: 1, count: 1 }]);
    }
    if (key === "Backspace" || key === "Delete") {
      e.preventDefault();
      if (editing) clearHere();
      return;
    }
    const cell = cellAt(cursor.row, cursor.ch);
    const field = FIELD[cursor.col];
    // The piano: notes on the note column (and played, edit on or off).
    if (field === 0 && tk.PIANO[e.code] !== undefined && !e.altKey) {
      e.preventDefault();
      if (e.repeat || held.current.has(e.code)) return;
      held.current.add(e.code);
      const note = Math.min(120, octave * 12 + tk.PIANO[e.code] + 1);
      playNote(note);
      if (editing) enterNote(e.code, note, null);
      return;
    }
    // Impulse Tracker's 4 and 8: the note under the cursor, the row.
    if (field === 0 && (e.code === "Digit4" || e.code === "Digit8") && !e.altKey) {
      e.preventDefault();
      if (e.repeat) return;
      if (e.code === "Digit8") {
        if (audioReady) tk.playRow(orderPos, cursor.row).catch(fail);
      } else if (cell[0] >= 1 && cell[0] <= 120) {
        held.current.add(e.code);
        playNote(cell[0], cell[1] || instrument);
      }
      return moveDown();
    }
    if (!editing) return;
    if (field === 0) {
      if (e.code === "Backquote") {
        e.preventDefault();
        putCell(cursor.row, cursor.ch, [e.shiftKey ? tk.NOTE_FADE : tk.NOTE_OFF, 0, cell[2], cell[3], cell[4], cell[5]]);
        return moveDown();
      }
      if (e.code === "Digit1") {
        e.preventDefault();
        putCell(cursor.row, cursor.ch, [tk.NOTE_CUT, 0, cell[2], cell[3], cell[4], cell[5]]);
        return moveDown();
      }
      return;
    }
    const digit = /^[0-9a-fA-F]$/.test(key) ? parseInt(key, 16) : null;
    const next = [...cell] as tk.Cell;
    const nibble = (v: number, hi: boolean, d: number) => (hi ? (d << 4) | (v & 15) : (v & 0xf0) | d);
    const advance = (second: boolean) => {
      if (second) {
        setCursor((c) => ({ ...c, col: c.col - 1, row: Math.min(rows - 1, c.row + step) }));
      } else {
        setCursor((c) => ({ ...c, col: c.col + 1 }));
      }
    };
    if (cursor.col === 1 || cursor.col === 2) {
      if (digit === null) return;
      e.preventDefault();
      next[1] = nibble(cell[1], cursor.col === 1, digit);
      putCell(cursor.row, cursor.ch, next);
      if (next[1] > 0) setInstrument(next[1]);
      return advance(cursor.col === 2);
    }
    if (cursor.col === 3) {
      const code = volCode.get(key.toLowerCase());
      if (code === undefined) return;
      e.preventDefault();
      next[2] = code;
      if (cell[2] === 0) next[3] = code === 1 ? 64 : code === 2 ? 32 : 0;
      putCell(cursor.row, cursor.ch, next);
      return setCursor((c) => ({ ...c, col: 4 }));
    }
    if (cursor.col === 4 || cursor.col === 5) {
      if (digit === null) return;
      e.preventDefault();
      if (next[2] === 0) next[2] = 1;
      next[3] = nibble(cell[3], cursor.col === 4, digit);
      if (next[2] === 1 || next[2] === 2) next[3] = Math.min(64, next[3]);
      putCell(cursor.row, cursor.ch, next);
      return advance(cursor.col === 5);
    }
    if (cursor.col === 6) {
      // Plain letters are the capital (IT) commands; Shift the lowercase ones.
      const letter = key.length === 1 ? (e.shiftKey ? key.toLowerCase() : key.toUpperCase()) : "";
      const code = fxCode.get(letter) ?? fxCode.get(key);
      if (code === undefined) return;
      e.preventDefault();
      next[4] = code;
      putCell(cursor.row, cursor.ch, next);
      return setCursor((c) => ({ ...c, col: 7 }));
    }
    if (cursor.col === 7 || cursor.col === 8) {
      if (digit === null) return;
      e.preventDefault();
      next[5] = nibble(cell[5], cursor.col === 7, digit);
      putCell(cursor.row, cursor.ch, next);
      return advance(cursor.col === 8);
    }
  }

  function onGridKeyUp(e: KeyboardEvent<HTMLDivElement>) {
    if (held.current.delete(e.code) && audioReady && held.current.size === 0) tk.jam(tk.NOTE_OFF, 0).catch(() => {});
    leaveNote(e.code);
  }

  // A MIDI keyboard: notes play, and (editing) are written as typed ones are.
  const midiRef = useRef<(n: { note: number; velocity: number; on: boolean }) => void>(() => {});
  midiRef.current = ({ note, velocity: vel, on }) => {
    const n = Math.min(120, note + 1);
    const key = `midi-${note}`;
    if (on) {
      playNote(n);
      if (editing && pat && FIELD[cursor.col] === 0) enterNote(key, n, vel);
    } else {
      if (audioReady) tk.jam(tk.NOTE_OFF, 0).catch(() => {});
      leaveNote(key);
    }
  };
  // A polite line for screen readers: where the cursor is, and what's there.
  useEffect(() => {
    if (!pat) return;
    const c = cellAt(cursor.row, cursor.ch);
    const parts = [tk.noteWords(c[0])];
    if (c[1]) parts.push(`instrument ${c[1]}`);
    if (c[2]) parts.push(`volume column ${volLetter.get(c[2]) ?? ""}${tk.hex2(c[3])}`);
    if (c[4]) parts.push(`${fxWords.get(c[4]) ?? "effect"}, ${tk.hex2(c[5])}`);
    setSaid(`Channel ${cursor.ch + 1}, row ${cursor.row}: ${parts.join(", ")}`);
  }, [cursor, pat, cellAt, volLetter, fxWords]);

  // ---- Orders ----

  function setOrders(orders: number[], pos = orderPos) {
    run([{ op: "orders", orders }], () => setOrderPos(Math.max(0, Math.min(pos, orders.length - 1))));
  }

  // ---- The text of a cell ----

  const cellText = useCallback(
    (c: tk.Cell) => {
      const note = tk.noteText(c[0]);
      const instr = c[1] ? tk.hex2(c[1]) : "··";
      const vol = c[2] ? `${volLetter.get(c[2]) ?? "?"}${tk.hex2(c[3])}` : "···";
      const fx = c[4] ? `${fxLetter.get(c[4]) ?? "?"}${tk.hex2(c[5])}` : "···";
      return `${note} ${instr} ${vol} ${fx}`;
    },
    [volLetter, fxLetter],
  );

  const playingRow = playing && status && doc && doc.orders[status.order] === patIndex ? status.row : -1;
  const visibleRows = Array.from({ length: Math.min(VISIBLE, rows - top) }, (_, i) => top + i);
  const sampleNames = doc?.samples ?? [];
  const instrumentNames = doc?.instrumentsMode ? doc.instruments.map((i) => i.name) : sampleNames.map((s) => s.name);

  const title = doc ? `${doc.name || "New song"}${doc.dirty ? " (changed)" : ""}` : "No song open";

  return (
    <section className="panel tracker" aria-label="Tracker" data-testid="tracker-panel">
      <div className="tracker-bar">
        <button type="button" data-testid="tracker-open" disabled={isBusy("tracker-open")} onClick={() => unlessDirty(() => void chooseAndOpen())}>
          Open…
        </button>
        <button type="button" data-testid="tracker-new" onClick={newSong}>
          New
        </button>
        <button type="button" className="primary" data-testid="tracker-save" disabled={!doc || isBusy("tracker-save")} title="The song with its samples, as an NTT" onClick={() => void saveSong()}>
          Save…
        </button>
        <button type="button" data-testid="tracker-export" disabled={!doc || isBusy("tracker-export")} onClick={() => setAsk("export")}>
          Export…
        </button>
        <button type="button" data-testid="tracker-find" disabled={!doc} title="Find and replace (Cmd+F)" onClick={() => setAsk("find")}>
          Find…
        </button>
        <button type="button" data-testid="tracker-settings-open" title="Interpolation, the Amiga's filters, a MIDI keyboard, chords" onClick={() => setAsk("settings")}>
          Settings…
        </button>
        <span className="tracker-sep" />
        <button type="button" data-testid="tracker-play-song" disabled={!doc || !audioReady} title="Play the song from the start (F5)" onClick={() => play("song")}>
          ▶ Song
        </button>
        <button type="button" data-testid="tracker-play-pattern" disabled={!doc || !audioReady} title="Play this pattern over and over (F6)" onClick={() => play("pattern")}>
          ▶ Pattern
        </button>
        <button type="button" data-testid="tracker-play-cursor" disabled={!doc || !audioReady} title="Play the song from the cursor's row (F7)" onClick={() => play("cursor")}>
          ▶ Cursor
        </button>
        <button type="button" data-testid="tracker-stop" disabled={!playing} title="Stop (F8)" onClick={stop}>
          ■ Stop
        </button>
        <span className="tracker-sep" />
        <button type="button" className="small" data-testid="tracker-undo" disabled={!doc?.canUndo} onClick={() => undoRedo(false)} title="Undo (Cmd+Z)">
          Undo
        </button>
        <button type="button" className="small" data-testid="tracker-redo" disabled={!doc?.canRedo} onClick={() => undoRedo(true)} title="Redo (Cmd+Shift+Z)">
          Redo
        </button>
        <span className="tracker-title" data-testid="tracker-title" title={doc?.path ?? undefined}>
          {title}
        </span>
      </div>

      {!doc ? (
        <div className="tracker-empty" data-testid="tracker-empty">
          <p>Open a tracker module (MOD, S3M, XM, IT, MTM, 669, MED, OKT, ULT, FAR, AHX and some eighty formats more) to play and edit it, or start a New song.</p>
          <p>Songs are saved as NTT, with their samples: everything they are, kept. Export… writes WAV, or MOD, S3M, XM and IT for other trackers.</p>
        </div>
      ) : (
        <div className="tracker-body">
          <div className="tracker-settings">
            <label htmlFor={`${id}-oct`}>Octave</label>
            <input id={`${id}-oct`} type="number" min={0} max={8} value={octave} data-testid="tracker-octave" onChange={(e) => choose({ octave: Math.max(0, Math.min(8, Number(e.currentTarget.value))) })} />
            <label htmlFor={`${id}-step`}>Step</label>
            <input id={`${id}-step`} type="number" min={0} max={16} value={step} data-testid="tracker-step" onChange={(e) => choose({ step: Math.max(0, Math.min(16, Number(e.currentTarget.value))) })} />
            <label htmlFor={`${id}-ins`}>{doc.instrumentsMode ? "Instrument" : "Sample"}</label>
            <select id={`${id}-ins`} value={instrument} data-testid="tracker-instrument" onChange={(e) => setInstrument(Number(e.currentTarget.value))}>
              {instrumentNames.map((n, i) => (
                <option key={i} value={i + 1}>{`${tk.hex2(i + 1)} ${n || "(unnamed)"}`}</option>
              ))}
            </select>
            <label className="form-check" title="Space turns editing on and off: off, the piano keys only play">
              <input type="checkbox" checked={editing} data-testid="tracker-edit" onChange={(e) => setEditing(e.currentTarget.checked)} />
              Edit
            </label>
            <label className="form-check" title="The pattern follows the song as it plays">
              <input type="checkbox" checked={follow} data-testid="tracker-follow" onChange={(e) => choose({ follow: e.currentTarget.checked })} />
              Follow
            </label>
            <span className="tracker-info" data-testid="tracker-info">
              {`${doc.formatName}${doc.tracker ? ` · ${doc.tracker}` : ""} · ${doc.channels.length} ch · speed ${doc.speed} · ${doc.tempo} bpm${playing && status ? ` · ▶ ${tk.hex2(status.order)}:${tk.hex2(status.row)}` : ""}`}
            </span>
          </div>
          <div className="tracker-main">
            <div className="tracker-orders">
              <div className="tracker-inline">
                <p className="tracker-head" title={doc.sequences.length > 1 ? "The order list being edited (the Song tab has them)" : undefined}>
                  {doc.sequences.length > 1 ? `Orders: ${doc.sequences[doc.sequence] || doc.sequence + 1}` : "Orders"}
                </p>
                <button type="button" className={`small tracker-matrix-toggle${matrix ? " selected" : ""}`} aria-pressed={matrix} title="The pattern matrix: a slot (an order's channel) clicked is muted there (Renoise's)" onClick={() => setMatrix((m) => !m)}>
                  ▦
                </button>
              </div>
              {matrix ? (
                <div className="tracker-matrix" data-testid="tracker-matrix" role="grid" aria-label="The pattern matrix: orders by channels, muted slots crossed">
                  {doc.orders.map((o, i) => (
                    <div key={i} role="row" className={`tracker-matrix-row${i === orderPos ? " selected" : ""}`}>
                      <button type="button" className="tracker-matrix-order" onClick={() => setOrderPos(i)}>
                        {tk.hex2(i)}
                      </button>
                      {doc.channels.map((_, ch) => {
                        const muted = doc.slotMutes.some(([a, b]) => a === i && b === ch);
                        return (
                          <button
                            key={ch}
                            type="button"
                            role="gridcell"
                            className={`tracker-slot${muted ? " muted" : ""}${o >= 0xfffe ? " skip" : ""}`}
                            aria-label={`Order ${i}, channel ${ch + 1}${muted ? ", muted" : ""}`}
                            aria-pressed={muted}
                            onClick={() => run([{ op: "slotMute", order: i, channel: ch, muted: !muted }])}
                          />
                        );
                      })}
                    </div>
                  ))}
                </div>
              ) : (
                <ol className="tracker-order-list" data-testid="tracker-orders" aria-label="The order the patterns play in">
                  {doc.orders.map((o, i) => (
                    <li key={i}>
                      <button
                        type="button"
                        className={`tracker-order${i === orderPos ? " selected" : ""}${playing && status?.order === i ? " playing" : ""}`}
                        aria-current={i === orderPos}
                        title={o < doc.patterns.length && doc.patterns[o].name ? doc.patterns[o].name : undefined}
                        onClick={() => setOrderPos(i)}
                      >
                        {`${tk.hex2(i)}: ${o === 0xfffe ? "+++" : String(o).padStart(3, "0")}${o < doc.patterns.length && doc.patterns[o].name ? ` ${doc.patterns[o].name}` : ""}${doc.slotMutes.some(([a]) => a === i) ? " ▦" : ""}`}
                      </button>
                    </li>
                  ))}
                </ol>
              )}
              <div className="tracker-order-buttons">
                <button type="button" className="small" title="The pattern number down" onClick={() => setOrders(doc.orders.map((o, i) => (i === orderPos && o > 0 && o < 0xfffe ? o - 1 : o)))}>
                  −
                </button>
                <button type="button" className="small" title="The pattern number up" onClick={() => setOrders(doc.orders.map((o, i) => (i === orderPos && o + 1 < doc.patterns.length ? o + 1 : o)))}>
                  +
                </button>
                <button type="button" className="small" title="Repeat this pattern after this position" onClick={() => setOrders([...doc.orders.slice(0, orderPos + 1), doc.orders[orderPos], ...doc.orders.slice(orderPos + 1)], orderPos + 1)}>
                  Ins
                </button>
                <button type="button" className="small" title="Take this position out" disabled={doc.orders.length <= 1} onClick={() => setOrders(doc.orders.filter((_, i) => i !== orderPos))}>
                  Del
                </button>
              </div>
              <button
                type="button"
                className="small tracker-wide"
                data-testid="tracker-new-pattern"
                title="A new, empty pattern after this position"
                onClick={() => run([{ op: "addPattern" }, { op: "orders", orders: [...doc.orders.slice(0, orderPos + 1), doc.patterns.length, ...doc.orders.slice(orderPos + 1)] }], () => setOrderPos(orderPos + 1))}
              >
                New Pattern
              </button>
              <button
                type="button"
                className="small tracker-wide"
                title="A copy of this pattern after this position"
                onClick={() => run([{ op: "clonePattern", pattern: patIndex }, { op: "orders", orders: [...doc.orders.slice(0, orderPos + 1), doc.patterns.length, ...doc.orders.slice(orderPos + 1)] }], () => setOrderPos(orderPos + 1))}
              >
                Copy Pattern
              </button>
              <label className="tracker-rows" htmlFor={`${id}-rows`}>
                Rows
              </label>
              <input id={`${id}-rows`} type="number" min={1} max={256} value={rows} data-testid="tracker-rows" onChange={(e) => run([{ op: "patternRows", pattern: patIndex, rows: Math.max(1, Math.min(256, Number(e.currentTarget.value) || 64)) }])} />
              <label className="tracker-rows" htmlFor={`${id}-pname`}>
                Name
              </label>
              <input
                key={`${patIndex}-${doc.patterns[patIndex]?.name ?? ""}`}
                id={`${id}-pname`}
                type="text"
                data-testid="tracker-pattern-name"
                defaultValue={doc.patterns[patIndex]?.name ?? ""}
                onBlur={(e) => e.currentTarget.value !== (doc.patterns[patIndex]?.name ?? "") && run([{ op: "patternName", pattern: patIndex, name: e.currentTarget.value }])}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
              />
              <label className="form-check tracker-rows" title="Its own rows a beat and a measure, and swing, over the song's">
                <input
                  type="checkbox"
                  checked={!!doc.patterns[patIndex]?.time || doc.patterns[patIndex]?.swing != null}
                  onChange={(e) => run([{ op: "patternTime", pattern: patIndex, time: e.currentTarget.checked ? [doc.rowsPerBeat, doc.rowsPerMeasure] : null, swing: e.currentTarget.checked ? doc.swing : null }])}
                />
                Own time
              </label>
              {(!!doc.patterns[patIndex]?.time || doc.patterns[patIndex]?.swing != null) && (
                <div className="tracker-own-time">
                  {(
                    [
                      ["Beat", 0, 1, 64],
                      ["Bar", 1, 1, 256],
                      ["Swing", 2, 0, 100],
                    ] as const
                  ).map(([n, k, lo, hi]) => {
                    const pt = doc.patterns[patIndex];
                    const vals = [pt.time?.[0] ?? doc.rowsPerBeat, pt.time?.[1] ?? doc.rowsPerMeasure, pt.swing ?? doc.swing];
                    return (
                      <label key={n} className="tracker-rows">
                        {n}
                        <input
                          type="number"
                          min={lo}
                          max={hi}
                          value={vals[k]}
                          onChange={(e) => {
                            const v = Math.max(lo, Math.min(hi, Number(e.currentTarget.value) || lo));
                            const next = vals.map((x, i) => (i === k ? v : x));
                            run([{ op: "patternTime", pattern: patIndex, time: [next[0], next[1]], swing: next[2] }]);
                          }}
                        />
                      </label>
                    );
                  })}
                </div>
              )}
              <label className="tracker-rows" htmlFor={`${id}-bm`}>
                Bookmarks
              </label>
              <select
                id={`${id}-bm`}
                value={jump}
                data-testid="tracker-bookmarks"
                onChange={(e) => {
                  const v = e.currentTarget.value;
                  setJump("");
                  if (v === "add") return addBookmark();
                  if (v.startsWith("del")) return run([{ op: "bookmarks", bookmarks: doc.bookmarks.filter((_, i) => i !== Number(v.slice(3))) }]);
                  const b = doc.bookmarks[Number(v)];
                  if (!b) return;
                  const o = b.order !== null && b.order < doc.orders.length ? b.order : doc.orders.indexOf(b.pattern ?? -1);
                  if (o >= 0) setOrderPos(o);
                  setCursor((c) => ({ ...c, row: b.row }));
                  grid.current?.focus();
                }}
              >
                <option value="">Go to…</option>
                {doc.bookmarks.map((b, i) => (
                  <option key={i} value={i}>
                    {b.name}
                  </option>
                ))}
                <option value="add">Bookmark here (Cmd+B)</option>
                {doc.bookmarks.map((b, i) => (
                  <option key={`d${i}`} value={`del${i}`}>{`Forget ${b.name}`}</option>
                ))}
              </select>
            </div>

            <div className="tracker-grid-wrap">
              <div className="tracker-grid-head" aria-hidden="true">
                <span className="tracker-rownum">{`P${String(patIndex).padStart(2, "0")}`}</span>
                {doc.channels.map((c, ch) => {
                  const level = status?.levels[ch] ?? 0;
                  const lit = Math.round(Math.min(1, level) * 6);
                  return (
                    <button
                      key={ch}
                      type="button"
                      tabIndex={-1}
                      className={`tracker-chan${muted.has(ch) ? " muted" : ""}`}
                      title={`${c.name || `Channel ${ch + 1}`}: click to mute, Alt+click to hear only it`}
                      onClick={(e) => {
                        const next = new Set(muted);
                        if (e.altKey) {
                          const solo = !(next.size === channels - 1 && !next.has(ch));
                          next.clear();
                          if (solo) for (let i = 0; i < channels; i++) if (i !== ch) next.add(i);
                        } else if (next.has(ch)) next.delete(ch);
                        else next.add(ch);
                        for (let i = 0; i < channels; i++) if (next.has(i) !== muted.has(i)) tk.mute(i, next.has(i)).catch(fail);
                        setMuted(next);
                      }}
                    >
                      {`${String(ch + 1).padStart(2, "0")} ${muted.has(ch) ? "mute " : ""}`}
                      <span className="tracker-vu">{"▮".repeat(lit)}</span>
                      <span className="tracker-vu-off">{"·".repeat(6 - lit)}</span>
                    </button>
                  );
                })}
              </div>
              <div
                ref={grid}
                className={`tracker-grid${editing ? " editing" : ""}`}
                role="grid"
                tabIndex={0}
                aria-label={`Pattern ${patIndex}, ${rows} rows, ${channels} channels${editing ? ", editing" : ""}`}
                aria-describedby={`${id}-said`}
                data-testid="tracker-grid"
                onKeyDown={onGridKey}
                onKeyUp={onGridKeyUp}
                onBlur={() => {
                  if (held.current.size > 0) {
                    held.current.clear();
                    tk.jam(tk.NOTE_OFF, 0).catch(() => {});
                  }
                }}
                onWheel={(e) => setTop((t) => Math.max(0, Math.min(Math.max(0, rows - VISIBLE), t + Math.sign(e.deltaY) * 2)))}
              >
                {visibleRows.map((row) => (
                  <div key={row} className={`tracker-line${row % (pat?.rowsPerMeasure || 16) === 0 ? " bar" : row % (pat?.rowsPerBeat || 4) === 0 ? " beat" : ""}${row === playingRow ? " now" : ""}`} role="row">
                    <span className="tracker-rownum">{String(row).padStart(3, "0")}</span>
                    {Array.from({ length: channels }, (_, ch) => {
                      const text = cellText(cellAt(row, ch));
                      const here = row === cursor.row && ch === cursor.ch;
                      return (
                        <span
                          key={ch}
                          role="gridcell"
                          className={`tracker-cell${inBlock(row, ch) ? " marked" : ""}${muted.has(ch) ? " muted" : ""}`}
                          onMouseDown={(e) => {
                            e.preventDefault();
                            grid.current?.focus();
                            if (e.shiftKey) setAnchor((a) => a ?? { row: cursor.row, ch: cursor.ch });
                            else setAnchor(null);
                            const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
                            const char = Math.floor(((e.clientX - r.left) / r.width) * 14);
                            const col = COL_AT.reduce((best, at, i) => (char >= at ? i : best), 0);
                            setCursor({ row, ch, col });
                          }}
                        >
                          {here ? (
                            <>
                              {text.slice(0, COL_AT[cursor.col])}
                              <span className="tracker-cursor">{text.slice(COL_AT[cursor.col], COL_AT[cursor.col] + COL_WIDTH[cursor.col])}</span>
                              {text.slice(COL_AT[cursor.col] + COL_WIDTH[cursor.col])}
                            </>
                          ) : (
                            text
                          )}
                        </span>
                      );
                    })}
                  </div>
                ))}
              </div>
              <p className="sr-only" id={`${id}-said`} aria-live="polite">
                {said}
              </p>
              <div className="tracker-inline tracker-under-grid">
                <select
                  aria-label="The block"
                  value=""
                  data-testid="tracker-block-menu"
                  title="What to do with the block marked (Shift+arrows), or the cell under the cursor"
                  onChange={(e) => {
                    const v = e.currentTarget.value;
                    grid.current?.focus();
                    if (v === "mix" || v === "mixCells" || v === "flood" || v === "insert") return pasteAs(v);
                    if (v === "slideVol" || v === "slideFx") return interpolate(v === "slideFx");
                    if (v === "amplify") return setAsk("amplify");
                    if (v === "replace") {
                      const b = blockOrCursor();
                      const from = cellAt(cursor.row, cursor.ch)[1];
                      if (!from) return showStatus("Put the cursor on a note with the instrument to replace.");
                      run([{ op: "replaceInstrument", from, to: instrument, pattern: block ? patIndex : null, row: b.row, channel: b.ch, rows: b.rows, channels: b.channels }]);
                      return showStatus(`Instrument ${tk.hex2(from)} made ${tk.hex2(instrument)}${block ? " in the block" : " through the song"}.`);
                    }
                    if (v === "swap") {
                      const a = cellAt(cursor.row, cursor.ch)[1];
                      if (!a || a === instrument) return showStatus("Put the cursor on a note with another instrument than the current one.");
                      run([{ op: "swapInstruments", a, b: instrument }]);
                      return showStatus(`Instruments ${tk.hex2(a)} and ${tk.hex2(instrument)} swapped.`);
                    }
                    if (v === "effect") return setAsk("effect");
                  }}
                >
                  <option value="">Block…</option>
                  <option value="mix">Mix paste: into empty fields (Cmd+Shift+V)</option>
                  <option value="mixCells">Fill paste: into empty cells</option>
                  <option value="flood">Flood paste: to the pattern's end</option>
                  <option value="insert">Insert paste: the rest pushed down (Cmd+Alt+V)</option>
                  <option value="slideVol">Slide volumes, first row to last (Alt+K)</option>
                  <option value="slideFx">Slide effect values, first row to last (Alt+X)</option>
                  <option value="amplify">Amplify or fade the volumes… (Alt+J)</option>
                  <option value="replace">Its instrument made the current one</option>
                  <option value="swap">Swap its instrument with the current one</option>
                  <option value="effect">The effect here… (Cmd+E)</option>
                </select>
                <span className="tracker-effect-words" data-testid="tracker-effect-words">
                  {(() => {
                    const c = cellAt(cursor.row, cursor.ch);
                    return c[4] ? effectWords(c[4], c[5], letters) : "No effect here";
                  })()}
                </span>
              </div>
              {touch ? (
                <TouchKeys
                  octave={octave}
                  onOctave={(o) => choose({ octave: o })}
                  editing={editing}
                  onEditing={setEditing}
                  onKey={(key, code, shift = false) => {
                    grid.current?.focus();
                    onGridKey({ key, code, shiftKey: shift, metaKey: false, ctrlKey: false, altKey: false, repeat: false, preventDefault() {} } as unknown as KeyboardEvent<HTMLDivElement>);
                  }}
                  onNoteDown={(semitone, key) => {
                    const note = Math.min(120, octave * 12 + semitone + 1);
                    playNote(note);
                    if (editing && FIELD[cursor.col] === 0) enterNote(key, note, null);
                  }}
                  onNoteUp={(key) => {
                    if (audioReady) tk.jam(tk.NOTE_OFF, 0).catch(() => {});
                    leaveNote(key);
                  }}
                  onEffect={() => setAsk("effect")}
                />
              ) : (
                              <p className="tracker-keys">
                  {editing ? "Editing" : "Not editing (Space)"} · octave {octave} (with [ and ]) · ZSXDCV… and Q2W3ER… notes · ` off, 1 cut · 4 note, 8 row · F5 song F6 pattern F7 from cursor (Shift: in the pattern) F8 stop
                </p>
              )}
            </div>

            <div className="tracker-side">
              <div className="tracker-tabs" role="tablist" aria-label="Samples, instruments, the song, the mix and automation">
                {(
                  [
                    ["samples", "Samples"],
                    ["instruments", "Instr."],
                    ["song", "Song"],
                    ["mix", "Mix"],
                    ["automation", "Lanes"],
                  ] as const
                ).map(([k, n]) => (
                  <button key={k} type="button" role="tab" aria-selected={side === k} className={side === k ? "selected" : ""} data-testid={`tracker-tab-${k}`} onClick={() => setSide(k)}>
                    {n}
                  </button>
                ))}
              </div>
              {side === "samples" && (
                <SamplePane
                  doc={doc}
                  letters={letters}
                  index={sample}
                  onIndex={setSample}
                  run={run}
                  patIndex={patIndex}
                  setDoc={setDoc}
                  playNote={(n) => playNote(n, doc.instrumentsMode ? 0 : sample + 1)}
                  showStatus={showStatus}
                  showError={showError}
                  onInstrument={(i) => {
                    setIns(i);
                    setSide("instruments");
                  }}
                  audioReady={audioReady}
                />
              )}
              {side === "instruments" && <InstrumentPane doc={doc} letters={letters} index={ins} onIndex={setIns} run={run} playNote={(n) => playNote(n, ins + 1)} showStatus={showStatus} showError={showError} />}
              {side === "song" && <SongPane doc={doc} letters={letters} run={run} onStats={() => setAsk("stats")} onMacros={() => setAsk("macros")} />}
              {side === "mix" && <MixPane doc={doc} letters={letters} run={run} levels={status?.levels ?? []} showError={showError} />}
              {side === "automation" && <AutomationPane doc={doc} pat={pat} letters={letters} run={run} playingRow={playingRow} />}
            </div>
          </div>
        </div>
      )}

      <ExportDialog open={ask === "export"} onClose={() => setAsk(null)} onWav={() => void exportWav()} onStems={() => void exportStems()} onModule={(f) => void exportModule(f)} />
      <SettingsDialog open={ask === "settings"} onClose={() => setAsk(null)} r={remembered} choose={choose} showError={showError} />
      <AmplifyDialog
        open={ask === "amplify"}
        onClose={() => setAsk(null)}
        onApply={(from, to) => {
          setAsk(null);
          const b = block ?? { row: 0, ch: cursor.ch, rows, channels: 1 };
          run([{ op: "amplify", pattern: patIndex, row: b.row, channel: b.ch, rows: b.rows, channels: b.channels, from, to }]);
        }}
      />
      <EffectDialog
        open={ask === "effect"}
        onClose={() => setAsk(null)}
        letters={letters}
        cell={cellAt(cursor.row, cursor.ch)}
        onApply={(fx, param) => {
          setAsk(null);
          const c = cellAt(cursor.row, cursor.ch);
          putCell(cursor.row, cursor.ch, [c[0], c[1], c[2], c[3], fx, fx ? param : 0]);
          grid.current?.focus();
        }}
      />
      {doc && (
        <>
          <FindDialog
            open={ask === "find"}
            onClose={() => setAsk(null)}
            letters={letters}
            doc={doc}
            patIndex={patIndex}
            block={block ? [block.row, block.ch, block.rows, block.channels] : null}
            onGo={(p, row, ch) => {
              const o = doc.orders.indexOf(p);
              if (o >= 0) setOrderPos(o);
              else setPatIndex(p);
              setCursor({ row, ch, col: 0 });
            }}
            onReplaced={(r) => {
              setDoc(r.edited.doc);
              if (r.edited.pattern) setPat(r.edited.pattern);
              showStatus(r.count === 0 ? "Nothing matched, so nothing was replaced." : `Replaced ${r.count} ${r.count === 1 ? "cell" : "cells"}.`);
            }}
            showError={showError}
          />
          <StatsDialog open={ask === "stats"} onClose={() => setAsk(null)} doc={doc} run={(e) => run(e, () => showStatus("Cleaned up: what nothing played is gone, the rest renumbered. Undo puts it back."))} showError={showError} />
          <MacrosDialog open={ask === "macros"} onClose={() => setAsk(null)} doc={doc} run={run} />
        </>
      )}

      <Dialog
        open={ask !== null && typeof ask === "object"}
        onClose={() => setAsk(null)}
        title="Leave your changes?"
        actions={
          <>
            <button type="button" onClick={() => setAsk(null)}>
              Keep Editing
            </button>
            <button
              type="button"
              className="danger"
              onClick={() => {
                const a = ask;
                setAsk(null);
                if (a && typeof a === "object") a.discard();
              }}
            >
              Leave Them
            </button>
          </>
        }
      >
        <p>{`${doc?.name || "The song"} has changes that aren't saved. They'll be lost.`}</p>
      </Dialog>
    </section>
  );
}

