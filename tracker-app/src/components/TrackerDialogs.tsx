/**
 * The tracker's dialogs: find and replace (Impulse Tracker's,
 * OpenMPT's), export (the song as WAV, each channel as its own WAV, or
 * as MOD, S3M, XM or IT), settings (how samples are read, the Amiga's
 * filters, a MIDI keyboard, chords and templates), a block's volumes
 * amplified or faded, an effect edited in words, the song's statistics
 * and cleanup, and its MIDI macros.
 */
import { useEffect, useState } from "react";
import * as midi from "../lib/midi";
import * as tk from "../lib/tracker";
import { Dialog } from "./Dialog";

const ANY = -1;

function NoteSelect({ value, onChange, label, any }: { value: number; onChange: (v: number) => void; label: string; any?: string }) {
  return (
    <select aria-label={label} value={value} onChange={(e) => onChange(Number(e.currentTarget.value))}>
      {any && <option value={ANY}>{any}</option>}
      {Array.from({ length: 120 }, (_, n) => (
        <option key={n} value={n + 1}>
          {tk.noteText(n + 1)}
        </option>
      ))}
      <option value={tk.NOTE_OFF}>=== note off</option>
      <option value={tk.NOTE_CUT}>^^^ note cut</option>
      <option value={tk.NOTE_FADE}>~~~ note fade</option>
    </select>
  );
}

function HexInput({ value, onChange, label }: { value: number; onChange: (v: number) => void; label: string }) {
  const [text, setText] = useState(tk.hex2(value));
  useEffect(() => setText(tk.hex2(value)), [value]);
  return (
    <input
      type="text"
      className="tracker-hex"
      aria-label={label}
      maxLength={2}
      value={text}
      onChange={(e) => setText(e.currentTarget.value)}
      onBlur={() => {
        const v = parseInt(text, 16);
        if (Number.isFinite(v) && v >= 0 && v <= 255) onChange(v);
        else setText(tk.hex2(value));
      }}
    />
  );
}

// ---- Find and replace ----

interface FindProps {
  open: boolean;
  onClose: () => void;
  letters: tk.Letters | null;
  doc: tk.Doc;
  patIndex: number;
  block: [number, number, number, number] | null;
  /** Goes to a cell found: (pattern, row, channel). */
  onGo: (p: number, row: number, ch: number) => void;
  onReplaced: (r: { count: number; edited: tk.Edited }) => void;
  showError: (m: string) => void;
}

export function FindDialog({ open, onClose, letters, doc, patIndex, block, onGo, onReplaced, showError }: FindProps) {
  const [q, setQ] = useState({ from: ANY, to: ANY, instr: ANY, vol: ANY, volLo: 0, volHi: 255, fx: ANY, paramLo: 0, paramHi: 255 });
  const [w, setW] = useState({ note: "keep" as "keep" | "set" | "transpose", noteTo: 61, transpose: 12, instr: ANY, vol: ANY, volp: 0, fx: ANY, param: 0, clear: false });
  const [scope, setScope] = useState<"song" | "pattern" | "block">("song");
  const [found, setFound] = useState<[number, number, number][] | null>(null);
  const [at, setAt] = useState(0);
  const query = (): tk.Query => ({
    notes: q.from === ANY ? null : [q.from, q.to === ANY ? q.from : Math.max(q.from, q.to)],
    instr: q.instr === ANY ? null : q.instr,
    vol: q.vol === ANY ? null : q.vol,
    volp: q.vol === ANY && q.volLo === 0 && q.volHi === 255 ? null : [q.volLo, q.volHi],
    fx: q.fx === ANY ? null : q.fx,
    param: q.fx === ANY && q.paramLo === 0 && q.paramHi === 255 ? null : [q.paramLo, q.paramHi],
  });
  const where = (): tk.Scope => (scope === "song" ? { pattern: null, block: null } : scope === "pattern" || !block ? { pattern: patIndex, block: null } : { pattern: patIndex, block });
  const empty = Object.values(query()).every((v) => v === null);

  function find() {
    tk.find(query(), where()).then((f) => {
      setFound(f);
      setAt(0);
      if (f[0]) onGo(...f[0]);
    }, (e) => showError(String(e)));
  }

  function replaceAll() {
    const r: tk.Replacement = {
      note: w.note === "set" ? w.noteTo : null,
      transpose: w.note === "transpose" ? w.transpose : 0,
      instr: w.instr === ANY ? null : w.instr,
      vol: w.vol === ANY ? null : [w.vol, w.volp],
      fx: w.fx === ANY ? null : [w.fx, w.param],
      clear: w.clear,
    };
    tk.replace(query(), r, where(), patIndex).then((res) => {
      onReplaced(res);
      setFound(null);
    }, (e) => showError(String(e)));
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Find and Replace"
      className="dialog-wide"
      actions={
        <>
          <button type="button" onClick={onClose}>
            Close
          </button>
          <button type="button" disabled={empty} data-testid="tracker-replace-all" onClick={replaceAll}>
            Replace All
          </button>
          <button type="button" className="primary" disabled={empty} data-testid="tracker-find-go" onClick={find}>
            Find
          </button>
        </>
      }
    >
      <div className="tracker-form tracker-find">
        <label>Notes</label>
        <span className="tracker-inline">
          <NoteSelect label="From note" value={q.from} any="any" onChange={(from) => setQ({ ...q, from })} />
          to
          <NoteSelect label="To note" value={q.to} any="the same" onChange={(to) => setQ({ ...q, to })} />
        </span>
        <label>Instrument</label>
        <select value={q.instr} onChange={(e) => setQ({ ...q, instr: Number(e.currentTarget.value) })}>
          <option value={ANY}>any</option>
          {Array.from({ length: Math.max(doc.samples.length, doc.instruments.length) }, (_, i) => (
            <option key={i} value={i + 1}>
              {tk.hex2(i + 1)}
            </option>
          ))}
        </select>
        <label>Volume column</label>
        <span className="tracker-inline">
          <select aria-label="Volume command" value={q.vol} onChange={(e) => setQ({ ...q, vol: Number(e.currentTarget.value) })}>
            <option value={ANY}>any</option>
            {(letters?.volumes ?? []).map(([c, l]) => (
              <option key={c} value={c}>
                {l}
              </option>
            ))}
          </select>
          <HexInput label="Lowest value" value={q.volLo} onChange={(volLo) => setQ({ ...q, volLo })} />
          to
          <HexInput label="Highest value" value={q.volHi} onChange={(volHi) => setQ({ ...q, volHi })} />
        </span>
        <label>Effect</label>
        <span className="tracker-inline">
          <select aria-label="Effect" value={q.fx} onChange={(e) => setQ({ ...q, fx: Number(e.currentTarget.value) })}>
            <option value={ANY}>any</option>
            {(letters?.effects ?? []).map(([c, l, d]) => (
              <option key={c} value={c}>{`${l}: ${d}`}</option>
            ))}
          </select>
          <HexInput label="Lowest parameter" value={q.paramLo} onChange={(paramLo) => setQ({ ...q, paramLo })} />
          to
          <HexInput label="Highest parameter" value={q.paramHi} onChange={(paramHi) => setQ({ ...q, paramHi })} />
        </span>
        <label>Where</label>
        <select value={scope} onChange={(e) => setScope(e.currentTarget.value as typeof scope)}>
          <option value="song">The whole song</option>
          <option value="pattern">{`Pattern ${patIndex}`}</option>
          <option value="block" disabled={!block}>
            The block marked
          </option>
        </select>
      </div>
      {found && (
        <div className="tracker-inline">
          <span>{found.length === 0 ? "Nothing matches." : `${found.length} found: ${at + 1} of ${found.length}, pattern ${found[at][0]} row ${found[at][1]} channel ${found[at][2] + 1}.`}</span>
          <button
            type="button"
            className="small"
            disabled={found.length < 2}
            onClick={() => {
              const n = (at + 1) % found.length;
              setAt(n);
              onGo(...found[n]);
            }}
          >
            Next
          </button>
        </div>
      )}
      <p className="tracker-head">Replace with</p>
      <div className="tracker-form tracker-find">
        <label>Note</label>
        <span className="tracker-inline">
          <select aria-label="What happens to the note" value={w.note} onChange={(e) => setW({ ...w, note: e.currentTarget.value as typeof w.note })}>
            <option value="keep">as it is</option>
            <option value="set">made</option>
            <option value="transpose">moved by</option>
          </select>
          {w.note === "set" && <NoteSelect label="The note" value={w.noteTo} onChange={(noteTo) => setW({ ...w, noteTo })} />}
          {w.note === "transpose" && <input type="number" aria-label="Semitones" min={-119} max={119} value={w.transpose} onChange={(e) => setW({ ...w, transpose: Number(e.currentTarget.value) })} />}
        </span>
        <label>Instrument</label>
        <select value={w.instr} onChange={(e) => setW({ ...w, instr: Number(e.currentTarget.value) })}>
          <option value={ANY}>as it is</option>
          <option value={0}>none</option>
          {Array.from({ length: Math.max(doc.samples.length, doc.instruments.length) }, (_, i) => (
            <option key={i} value={i + 1}>
              {tk.hex2(i + 1)}
            </option>
          ))}
        </select>
        <label>Volume column</label>
        <span className="tracker-inline">
          <select aria-label="Volume command" value={w.vol} onChange={(e) => setW({ ...w, vol: Number(e.currentTarget.value) })}>
            <option value={ANY}>as it is</option>
            <option value={0}>none</option>
            {(letters?.volumes ?? []).map(([c, l]) => (
              <option key={c} value={c}>
                {l}
              </option>
            ))}
          </select>
          {w.vol > 0 && <HexInput label="Value" value={w.volp} onChange={(volp) => setW({ ...w, volp })} />}
        </span>
        <label>Effect</label>
        <span className="tracker-inline">
          <select aria-label="Effect" value={w.fx} onChange={(e) => setW({ ...w, fx: Number(e.currentTarget.value) })}>
            <option value={ANY}>as it is</option>
            <option value={0}>none</option>
            {(letters?.effects ?? []).map(([c, l, d]) => (
              <option key={c} value={c}>{`${l}: ${d}`}</option>
            ))}
          </select>
          {w.fx > 0 && <HexInput label="Parameter" value={w.param} onChange={(param) => setW({ ...w, param })} />}
        </span>
        <label>Or</label>
        <label className="form-check">
          <input type="checkbox" checked={w.clear} onChange={(e) => setW({ ...w, clear: e.currentTarget.checked })} />
          Clear the cells found
        </label>
      </div>
    </Dialog>
  );
}

// ---- Export ----

export function ExportDialog({ open, onClose, onWav, onStems, onModule }: { open: boolean; onClose: () => void; onWav: () => void; onStems: () => void; onModule: (format: string) => void }) {
  return (
    <Dialog open={open} onClose={onClose} title="Export">
      <div className="tracker-export">
        <button type="button" className="primary" data-testid="tracker-export-wav" onClick={onWav}>
          WAV…
        </button>
        <p className="tracker-facts">The song played once through, at 44.1 kHz.</p>
        <button type="button" data-testid="tracker-export-stems" onClick={onStems}>
          A WAV a Channel…
        </button>
        <p className="tracker-facts">Each channel played on its own (stems), into a folder: they add up to the song.</p>
        <div className="tracker-inline">
          {[
            ["mod", "MOD"],
            ["s3m", "S3M"],
            ["xm", "XM"],
            ["it", "IT"],
          ].map(([f, n]) => (
            <button key={f} type="button" data-testid={`tracker-export-${f}`} onClick={() => onModule(f)}>
              {`${n}…`}
            </button>
          ))}
        </div>
        <p className="tracker-facts">For ProTracker, Scream Tracker 3, FastTracker 2 and Impulse Tracker (and everything that reads them). What the format can't keep is said once it's written.</p>
      </div>
    </Dialog>
  );
}

// ---- Settings ----

interface SettingsProps {
  open: boolean;
  onClose: () => void;
  r: tk.Remembered;
  choose: (patch: Partial<tk.Remembered>) => void;
  showError: (m: string) => void;
}

export function SettingsDialog({ open, onClose, r, choose, showError }: SettingsProps) {
  // The MIDI keyboards there are (none where the platform has no MIDI).
  const [inputs, setInputs] = useState<string[] | null>(null);
  useEffect(() => {
    if (open) midi.inputs().then(setInputs, (e) => showError(String(e)));
  }, [open, showError]);
  return (
    <Dialog open={open} onClose={onClose} title="Tracker Settings">
      <div className="tracker-form tracker-settings-form">
        <label title="How a sample's points are read between them">Interpolation</label>
        <select value={r.interp} data-testid="tracker-interp" onChange={(e) => choose({ interp: Number(e.currentTarget.value) })}>
          {tk.INTERPS.map((n, i) => (
            <option key={n} value={i}>
              {n}
            </option>
          ))}
        </select>
        <label title="An Amiga 500's low-pass, and its LED filter (E00 on, E01 off), over MOD files">Amiga filters</label>
        <input type="checkbox" checked={r.amiga} data-testid="tracker-amiga" onChange={(e) => choose({ amiga: e.currentTarget.checked })} />
        <label>MIDI keyboard</label>
        <select value={r.midiIn ?? ""} disabled={inputs !== null && inputs.length === 0} data-testid="tracker-midi-in" onChange={(e) => choose({ midiIn: e.currentTarget.value || null })}>
          <option value="">{inputs !== null && inputs.length === 0 ? "None here" : "None"}</option>
          {(inputs ?? []).map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
        <label title="A MIDI note's velocity written in the volume column">Velocity to volume</label>
        <input type="checkbox" checked={r.velocity} onChange={(e) => choose({ velocity: e.currentTarget.checked })} />
        <label title="Notes held together go into channels side by side (FastTracker 2's multichannel record)">Chords across channels</label>
        <input type="checkbox" checked={r.spread} data-testid="tracker-spread" onChange={(e) => choose({ spread: e.currentTarget.checked })} />
        <label title="Each key writes this chord across channels from the cursor's">Chord a key</label>
        <select value={tk.CHORDS.findIndex(([, c]) => c.join() === r.chord.join())} onChange={(e) => choose({ chord: tk.CHORDS[Number(e.currentTarget.value)]?.[1] ?? [] })}>
          {tk.CHORDS.map(([n], i) => (
            <option key={n} value={i}>
              {n}
            </option>
          ))}
        </select>
        <label title="Notes typed get the template's volume column and effect too (Cmd+T makes the cell under the cursor the template)">Template</label>
        <span className="tracker-inline">
          <span>{r.template ? `volume ${r.template[2] ? tk.hex2(r.template[3]) : "none"}, effect ${r.template[4] ? tk.hex2(r.template[5]) : "none"}` : "none"}</span>
          <button type="button" className="small" disabled={!r.template} onClick={() => choose({ template: null })}>
            Clear
          </button>
        </span>
      </div>
    </Dialog>
  );
}

// ---- Amplify ----

export function AmplifyDialog({ open, onClose, onApply }: { open: boolean; onClose: () => void; onApply: (from: number, to: number) => void }) {
  const [from, setFrom] = useState(100);
  const [to, setTo] = useState(100);
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Amplify"
      actions={
        <>
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button type="button" className="primary" data-testid="tracker-amplify-go" onClick={() => onApply(from, to)}>
            Apply
          </button>
        </>
      }
    >
      <p>The block's volumes (or, where a note has none, its sample's) scaled from its first row to its last: the same for louder or quieter, different for a fade.</p>
      <div className="tracker-form">
        <label htmlFor="amp-from">First row</label>
        <span className="tracker-inline">
          <input id="amp-from" type="number" min={0} max={800} value={from} onChange={(e) => setFrom(Number(e.currentTarget.value))} />%
        </span>
        <label htmlFor="amp-to">Last row</label>
        <span className="tracker-inline">
          <input id="amp-to" type="number" min={0} max={800} value={to} onChange={(e) => setTo(Number(e.currentTarget.value))} />%
        </span>
      </div>
      <div className="tracker-inline">
        <button type="button" className="small" onClick={() => (setFrom(0), setTo(100))}>
          Fade In
        </button>
        <button type="button" className="small" onClick={() => (setFrom(100), setTo(0))}>
          Fade Out
        </button>
        <button type="button" className="small" onClick={() => (setFrom(50), setTo(50))}>
          Half
        </button>
        <button type="button" className="small" onClick={() => (setFrom(200), setTo(200))}>
          Double
        </button>
      </div>
    </Dialog>
  );
}

// ---- An effect in words ----

/** What an effect's parameter means, in words, as the module's tracker reads it. */
export function effectWords(fx: number, param: number, letters: tk.Letters | null): string {
  const e = (letters?.effects ?? []).find(([c]) => c === fx);
  if (!e) return "";
  const [, letter, words] = e;
  const x = param >> 4;
  const y = param & 15;
  const two = /x\b|\by\b|x and y|x its|y its|x up|y down/.test(words);
  return `${letter}${tk.hex2(param)}: ${words}${two ? ` (x ${x}, y ${y}; ${param} in all)` : ` (${param})`}`;
}

interface EffectProps {
  open: boolean;
  onClose: () => void;
  letters: tk.Letters | null;
  cell: tk.Cell;
  onApply: (fx: number, param: number) => void;
}

/** OpenMPT's effect dialog: the command chosen from a list, its parameter as a whole or as two nibbles. */
export function EffectDialog({ open, onClose, letters, cell, onApply }: EffectProps) {
  const [fx, setFx] = useState(cell[4]);
  const [param, setParam] = useState(cell[5]);
  useEffect(() => {
    if (open) {
      setFx(cell[4]);
      setParam(cell[5]);
    }
  }, [open, cell]);
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Effect"
      actions={
        <>
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button type="button" className="primary" data-testid="tracker-effect-go" onClick={() => onApply(fx, param)}>
            Apply
          </button>
        </>
      }
    >
      <div className="tracker-form">
        <label>Command</label>
        <select value={fx} data-testid="tracker-effect-select" onChange={(e) => setFx(Number(e.currentTarget.value))}>
          <option value={0}>None</option>
          {(letters?.effects ?? []).map(([c, l, d]) => (
            <option key={c} value={c}>{`${l}: ${d}`}</option>
          ))}
        </select>
        <label>Parameter</label>
        <span className="tracker-inline">
          <input type="range" min={0} max={255} value={param} aria-label="Parameter" onChange={(e) => setParam(Number(e.currentTarget.value))} />
          <span>{tk.hex2(param)}</span>
        </span>
        <label>x (high)</label>
        <input type="range" min={0} max={15} value={param >> 4} aria-label="x" onChange={(e) => setParam((Number(e.currentTarget.value) << 4) | (param & 15))} />
        <label>y (low)</label>
        <input type="range" min={0} max={15} value={param & 15} aria-label="y" onChange={(e) => setParam((param & 0xf0) | Number(e.currentTarget.value))} />
      </div>
      <p className="tracker-facts">{fx ? effectWords(fx, param, letters) : "No effect."}</p>
    </Dialog>
  );
}

// ---- Statistics and cleanup ----

export function StatsDialog({ open, onClose, doc, run, showError }: { open: boolean; onClose: () => void; doc: tk.Doc; run: (edits: tk.Edit[]) => void; showError: (m: string) => void }) {
  const [s, setS] = useState<tk.Stats | null>(null);
  const [what, setWhat] = useState({ patterns: true, samples: true, instruments: true });
  useEffect(() => {
    if (open) {
      setS(null);
      tk.stats().then(setS, (e) => showError(String(e)));
    }
  }, [open, doc, showError]);
  const list = (v: number[], hex: boolean) => (v.length === 0 ? "none" : v.slice(0, 24).map((i) => (hex ? tk.hex2(i + 1) : String(i))).join(", ") + (v.length > 24 ? "…" : ""));
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Song Info & Cleanup"
      actions={
        <>
          <button type="button" onClick={onClose}>
            Close
          </button>
          <button
            type="button"
            className="danger"
            data-testid="tracker-cleanup"
            disabled={!s || (!what.patterns && !what.samples && !what.instruments)}
            onClick={() => {
              run([{ op: "cleanup", ...what }]);
              onClose();
            }}
          >
            Clean Up
          </button>
        </>
      }
    >
      {!s ? (
        <p>Working it out…</p>
      ) : (
        <>
          <p data-testid="tracker-stats-text">{`It plays for ${tk.seconds(s.seconds)}: ${s.orders} orders of ${s.patterns} patterns, ${s.notes.toLocaleString()} notes.`}</p>
          <p className="tracker-facts">{`Patterns no order list plays: ${list(s.unusedPatterns, false)}. Empty patterns: ${list(s.emptyPatterns, false)}.`}</p>
          <p className="tracker-facts">{`Samples nothing plays: ${list(s.unusedSamples, true)} (${tk.bytes(s.unusedBytes)}).${doc.instrumentsMode ? ` Instruments no note plays: ${list(s.unusedInstruments, true)}.` : ""}`}</p>
          <p className="tracker-head">Clean up (one undo step)</p>
          <label className="form-check">
            <input type="checkbox" checked={what.patterns} onChange={(e) => setWhat({ ...what, patterns: e.currentTarget.checked })} />
            Take out unplayed patterns (the rest renumbered)
          </label>
          <label className="form-check">
            <input type="checkbox" checked={what.samples} onChange={(e) => setWhat({ ...what, samples: e.currentTarget.checked })} />
            Take out unused samples (the rest renumbered)
          </label>
          {doc.instrumentsMode && (
            <label className="form-check">
              <input type="checkbox" checked={what.instruments} onChange={(e) => setWhat({ ...what, instruments: e.currentTarget.checked })} />
              Take out unused instruments (the rest renumbered)
            </label>
          )}
        </>
      )}
    </Dialog>
  );
}

// ---- MIDI macros ----

const IT_MACROS = (): tk.Macros => ({
  parametered: ["F0F000z", ...Array(15).fill("")],
  fixed: Array.from({ length: 128 }, (_, i) => (i < 16 ? `F0F001${tk.hex2(i * 8)}` : "")),
});

export function MacrosDialog({ open, onClose, doc, run }: { open: boolean; onClose: () => void; doc: tk.Doc; run: (edits: tk.Edit[]) => void }) {
  const [m, setM] = useState<tk.Macros>(doc.macros ?? IT_MACROS());
  useEffect(() => {
    if (open) setM(doc.macros ?? IT_MACROS());
  }, [open, doc.macros]);
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="MIDI Macros"
      className="dialog-wide"
      actions={
        <>
          <button type="button" onClick={() => setM(IT_MACROS())}>
            Impulse Tracker's
          </button>
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="primary"
            onClick={() => {
              run([{ op: "macros", macros: m }]);
              onClose();
            }}
          >
            Save
          </button>
        </>
      }
    >
      <p className="tracker-facts">
        SFx chooses which of the first 16 Z00 to Z7F send, with z its value; Z80 to ZFF each send their own. Hex bytes, and letters: z the value, n the note, v the velocity, u the volume, x the
        panning, c the channel. F0F000z sets the filter's cutoff, F0F001z its resonance, F0F002z its mode (00 low-pass, 10 high-pass), F0F080z and up an effect slot's parameters. Anything else goes
        to the instrument's MIDI output.
      </p>
      <div className="tracker-macros">
        {m.parametered.map((t, i) => (
          <label key={`p${i}`} className="tracker-inline">
            <span className="tracker-macro-name">{`SF${i.toString(16).toUpperCase()}`}</span>
            <input type="text" value={t} spellCheck={false} onChange={(e) => setM({ ...m, parametered: m.parametered.map((x, k) => (k === i ? e.currentTarget.value : x)) })} />
          </label>
        ))}
        {m.fixed.map((t, i) => (
          <label key={`f${i}`} className="tracker-inline">
            <span className="tracker-macro-name">{`Z${tk.hex2(0x80 + i)}`}</span>
            <input type="text" value={t} spellCheck={false} onChange={(e) => setM({ ...m, fixed: m.fixed.map((x, k) => (k === i ? e.currentTarget.value : x)) })} />
          </label>
        ))}
      </div>
    </Dialog>
  );
}
