/**
 * The tracker's Instruments pane: an instrument's keymap as a piano
 * (Impulse Tracker's note-sample table), its envelopes drawn by hand with
 * tools (a point put in between two, fitted to its sample, sped up or
 * slowed, OpenMPT's release node), its new note action and duplicate
 * check, pitch-pan separation, random volume and panning, the resonant
 * filter, a MIDI output, the effect slot it plays through, and its phrase
 * (Renoise's: a column of notes its key plays, transposed).
 */
import { useEffect, useState } from "react";
import { host } from "../lib/host";
import * as tk from "../lib/tracker";
import { EnvelopeEditor, KeymapPiano, NumberField, TextField } from "./TrackerParts";

const NNAS = ["Cut", "Continue", "Note off", "Fade"];
const DCTS = ["Off", "Note", "Sample", "Instrument"];
const DCAS = ["Cut", "Note off", "Fade"];

interface Props {
  doc: tk.Doc;
  letters: tk.Letters | null;
  index: number;
  onIndex: (i: number) => void;
  run: (edits: tk.Edit[], then?: (d: tk.Doc) => void) => void;
  playNote: (n: number) => void;
  showStatus: (m: string) => void;
  showError: (m: string) => void;
}

/** A phrase as text, a row a line: "C-5 01 v40 A06", dots for what's empty. */
function phraseText(p: tk.Phrase, vol: Map<number, string>, fx: Map<number, string>): string {
  return p.cells
    .map((c) => {
      const note = c[0] ? tk.noteText(c[0]) : "...";
      const ins = c[1] ? tk.hex2(c[1]) : "..";
      const v = c[2] ? `${vol.get(c[2]) ?? "v"}${tk.hex2(c[3])}` : "...";
      const f = c[4] ? `${fx.get(c[4]) ?? "?"}${tk.hex2(c[5])}` : "...";
      return `${note} ${ins} ${v} ${f}`;
    })
    .join("\n");
}

/** A note as typed ("C-5", "C#5", "===", "^^^", "~~~"), or 0. */
function parseNote(t: string): number {
  if (t === "===") return tk.NOTE_OFF;
  if (t === "^^^") return tk.NOTE_CUT;
  if (t === "~~~") return tk.NOTE_FADE;
  const m = /^([A-Ga-g])([-#])(\d)$/.exec(t);
  if (!m) return 0;
  const base = { c: 0, d: 2, e: 4, f: 5, g: 7, a: 9, b: 11 }[m[1].toLowerCase()] ?? 0;
  return Number(m[3]) * 12 + base + (m[2] === "#" ? 1 : 0) + 1;
}

function parsePhrase(text: string, volCode: Map<string, number>, fxCode: Map<string, number>): tk.Cell[] {
  return text
    .split("\n")
    .filter((l) => l.trim() !== "")
    .slice(0, 256)
    .map((line) => {
      const [n = "...", i = "..", v = "...", f = "..."] = line.trim().split(/\s+/);
      const hex = (t: string) => (/^[0-9a-fA-F]{1,2}$/.test(t) ? parseInt(t, 16) : 0);
      const cell: tk.Cell = [parseNote(n), hex(i), 0, 0, 0, 0];
      if (v.length === 3 && volCode.has(v[0])) {
        cell[2] = volCode.get(v[0]) ?? 0;
        cell[3] = hex(v.slice(1));
      }
      if (f.length === 3 && fxCode.has(f[0])) {
        cell[4] = fxCode.get(f[0]) ?? 0;
        cell[5] = hex(f.slice(1));
      }
      return cell;
    });
}

export function InstrumentPane({ doc, letters, index, onIndex, run, playNote, showStatus, showError }: Props) {
  const [env, setEnv] = useState<"volumeEnv" | "panEnv" | "pitchEnv">("volumeEnv");
  const [range, setRange] = useState<[number, number]>([60, 60]);
  const [mapTo, setMapTo] = useState(1);
  const [playsAs, setPlaysAs] = useState<"same" | "fixed">("same");
  const [fixedNote, setFixedNote] = useState(60);
  const [point, setPoint] = useState(0);
  const [speed, setSpeed] = useState(100);
  const [phrase, setPhrase] = useState("");
  const x = doc.instruments[index];
  const volLetter = new Map((letters?.volumes ?? []).map(([c, l]) => [c, l]));
  const fxLetter = new Map((letters?.effects ?? []).map(([c, l]) => [c, l]));
  const volCode = new Map((letters?.volumes ?? []).map(([c, l]) => [l, c]));
  const fxCode = new Map((letters?.effects ?? []).map(([c, l]) => [l, c]));
  useEffect(() => setPhrase(x?.phrase ? phraseText(x.phrase, volLetter, fxLetter) : ""), [x?.phrase, letters]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!doc.instrumentsMode) {
    return (
      <div className="tracker-pane" data-testid="tracker-instruments">
        <p className="tracker-facts">This song plays its samples directly, as MOD and S3M do. Instruments give samples a keymap (a sample for each note), envelopes and new note actions.</p>
        <button type="button" data-testid="tracker-use-instruments" onClick={() => run([{ op: "instrumentsMode", on: true }])}>
          Use Instruments
        </button>
      </div>
    );
  }
  const change = (patch: Partial<tk.Instrument>) => {
    if (!x) return;
    const n = { ...x, ...patch };
    run([{ op: "instrument", index, name: n.name, globalVolume: n.globalVolume, fadeout: n.fadeout, pan: n.pan, nna: n.nna, volumeEnv: n.volumeEnv, panEnv: n.panEnv, pitchEnv: n.pitchEnv, more: tk.moreOf(n) }]);
  };
  const e = x?.[env];
  const setEnvelope = (v: tk.Envelope) => change({ [env]: v } as Partial<tk.Instrument>);
  const used = x ? Array.from(new Set(x.keymap.map(([, s]) => s).filter((s) => s > 0))) : [];
  const [lo, hi] = [Math.min(range[0], range[1]), Math.max(range[0], range[1])];

  /** The envelope's ticks scaled, its first point staying at 0. */
  const scale = (k: number) => e && setEnvelope({ ...e, points: e.points.map(([t, v]) => [Math.min(65535, Math.round(t * k)), v] as [number, number]) });
  /** The envelope stretched to its main sample's length at the song's tempo. */
  const fit = () => {
    if (!e || e.points.length < 2) return;
    const s = doc.samples[(x?.keymap[60]?.[1] ?? used[0] ?? 0) - 1];
    if (!s || s.frames === 0) return showError("Its keymap plays no sample at C-5 to fit to.");
    const seconds = s.frames / Math.max(1, s.c5);
    const ticks = seconds / (2.5 / Math.max(32, doc.tempo));
    const last = e.points[e.points.length - 1][0] || 1;
    scale(ticks / last);
    showStatus(`Fitted to ${s.name || "its sample"}: ${tk.seconds(seconds)}, ${Math.round(ticks)} ticks.`);
  };
  const filterHz = (c: number) => Math.round(110 * Math.pow(2, 0.25 + c / 24));

  async function saveXi() {
    if (!x) return;
    try {
      const [, bytes] = await tk.saveInstrument(index);
      const to = await (await host()).save(`${x.name || `instrument ${index + 1}`}.xi`, bytes, { name: "FastTracker 2 instrument", extensions: ["xi"] });
      if (to) showStatus(`Saved ${to}.`);
    } catch (err) {
      showError(String(err));
    }
  }

  return (
    <div className="tracker-pane" data-testid="tracker-instruments">
      <ul className="tracker-list short" aria-label="Instruments">
        {doc.instruments.map((v, i) => (
          <li key={i}>
            <button type="button" className={`tracker-item${i === index ? " selected" : ""}`} onClick={() => onIndex(i)} onDoubleClick={() => playNote(61)}>
              {`${tk.hex2(i + 1)} ${v.name || "(unnamed)"}${v.midi ? " ♪" : ""}${v.phrase ? " ≡" : ""}`}
            </button>
          </li>
        ))}
      </ul>
      <div className="tracker-list-buttons">
        <button type="button" className="small" onClick={() => run([{ op: "addInstrument" }], (d) => onIndex(d.instruments.length - 1))}>
          Add
        </button>
        <button type="button" className="small" disabled={!x} onClick={() => playNote(61)}>
          ▶ C-5
        </button>
        <button type="button" className="small" disabled={!x} title="Save it, and its samples, as a FastTracker 2 instrument" onClick={() => void saveXi()}>
          Save XI…
        </button>
      </div>
      {x && (
        <>
          <div className="tracker-form">
            <TextField label="Name" value={x.name} onCommit={(name) => change({ name })} wide />
            <NumberField label="Global vol." value={x.globalVolume} min={0} max={128} onCommit={(globalVolume) => change({ globalVolume })} />
            <NumberField label="Fadeout" value={x.fadeout} min={0} max={65536} onCommit={(fadeout) => change({ fadeout })} title="Taken off the volume each tick once the note's let go, of 65,536" />
            <label title="What a note still sounding does when the next comes">New note</label>
            <select value={x.nna} onChange={(ev) => change({ nna: Number(ev.currentTarget.value) })}>
              {NNAS.map((n, i) => (
                <option key={n} value={i}>
                  {n}
                </option>
              ))}
            </select>
            <label title="Which of its notes still sounding count as the same as a new one">Duplicate check</label>
            <select value={x.dct} onChange={(ev) => change({ dct: Number(ev.currentTarget.value) })}>
              {DCTS.map((n, i) => (
                <option key={n} value={i}>
                  {n}
                </option>
              ))}
            </select>
            {x.dct > 0 && (
              <>
                <label>Then</label>
                <select value={x.dca} onChange={(ev) => change({ dca: Number(ev.currentTarget.value) })}>
                  {DCAS.map((n, i) => (
                    <option key={n} value={i}>
                      {n}
                    </option>
                  ))}
                </select>
              </>
            )}
            <label>Panning</label>
            <span className="tracker-inline">
              <input type="checkbox" checked={x.pan !== null} aria-label="Its own panning" onChange={(ev) => change({ pan: ev.currentTarget.checked ? 128 : null })} />
              <input type="range" min={0} max={255} value={x.pan ?? 128} disabled={x.pan === null} aria-label="Panning" onChange={(ev) => change({ pan: Number(ev.currentTarget.value) })} />
            </span>
            <NumberField label="Pitch-pan" value={x.pitchPanSeparation} min={-32} max={32} onCommit={(pitchPanSeparation) => change({ pitchPanSeparation })} title="Notes above the center pan right (or, below 0, left), this much" />
            <label>Pitch-pan center</label>
            <select value={x.pitchPanCenter} onChange={(ev) => change({ pitchPanCenter: Number(ev.currentTarget.value) })}>
              {Array.from({ length: 120 }, (_, n) => (
                <option key={n} value={n}>
                  {tk.noteText(n + 1)}
                </option>
              ))}
            </select>
            <NumberField label="Random vol." value={x.randomVolume} min={0} max={100} onCommit={(randomVolume) => change({ randomVolume })} title="Each note's volume varies this many percent" />
            <NumberField label="Random pan" value={x.randomPan} min={0} max={64} onCommit={(randomPan) => change({ randomPan })} title="Each note's panning varies this much" />
            <label title="The resonant filter: its cutoff, 0 to 127">Filter cutoff</label>
            <span className="tracker-inline">
              <input type="checkbox" checked={x.cutoff !== null} aria-label="Set the cutoff" onChange={(ev) => change({ cutoff: ev.currentTarget.checked ? 127 : null })} />
              <input type="range" min={0} max={127} value={x.cutoff ?? 127} disabled={x.cutoff === null} aria-label="Cutoff" onChange={(ev) => change({ cutoff: Number(ev.currentTarget.value) })} />
              <span>{x.cutoff !== null ? `${filterHz(x.cutoff)} Hz` : "open"}</span>
            </span>
            <label>Resonance</label>
            <span className="tracker-inline">
              <input type="checkbox" checked={x.resonance !== null} aria-label="Set the resonance" onChange={(ev) => change({ resonance: ev.currentTarget.checked ? 0 : null })} />
              <input type="range" min={0} max={127} value={x.resonance ?? 0} disabled={x.resonance === null} aria-label="Resonance" onChange={(ev) => change({ resonance: Number(ev.currentTarget.value) })} />
            </span>
            <label title="Its notes sent to a MIDI output too, kept with the song for the trackers and players that send them">MIDI out</label>
            <select value={x.midi ? x.midi.channel : -1} onChange={(ev) => {
              const ch = Number(ev.currentTarget.value);
              change({ midi: ch < 0 ? null : { channel: ch, program: x.midi?.program ?? null, bank: x.midi?.bank ?? null } });
            }}>
              <option value={-1}>None</option>
              {Array.from({ length: 16 }, (_, c) => (
                <option key={c} value={c}>{`Channel ${c + 1}`}</option>
              ))}
            </select>
            {x.midi && (
              <>
                <label>Program</label>
                <span className="tracker-inline">
                  <input type="checkbox" checked={x.midi.program !== null} aria-label="Send a program change" onChange={(ev) => x.midi && change({ midi: { ...x.midi, program: ev.currentTarget.checked ? 0 : null } })} />
                  <input type="number" min={1} max={128} disabled={x.midi.program === null} aria-label="Program, 1 to 128" value={(x.midi.program ?? 0) + 1} onChange={(ev) => x.midi && change({ midi: { ...x.midi, program: Math.max(0, Math.min(127, Number(ev.currentTarget.value) - 1)) } })} />
                </span>
                <label>Bank</label>
                <span className="tracker-inline">
                  <input type="checkbox" checked={x.midi.bank !== null} aria-label="Send a bank select" onChange={(ev) => x.midi && change({ midi: { ...x.midi, bank: ev.currentTarget.checked ? 0 : null } })} />
                  <input type="number" min={0} max={16383} disabled={x.midi.bank === null} aria-label="Bank, 0 to 16383" value={x.midi.bank ?? 0} onChange={(ev) => x.midi && change({ midi: { ...x.midi, bank: Math.max(0, Math.min(16383, Number(ev.currentTarget.value))) } })} />
                </span>
              </>
            )}
            <label title="Its notes go through this effect slot (the Mix tab has them)">Effect slot</label>
            <select value={x.effect ?? -1} onChange={(ev) => change({ effect: Number(ev.currentTarget.value) < 0 ? null : Number(ev.currentTarget.value) })}>
              <option value={-1}>The channel's</option>
              {doc.effects.map((f, i) => (
                <option key={i} value={i}>{`${i + 1}: ${f.name || letters?.effectKinds[f.kind]?.[0] || "Effect"}`}</option>
              ))}
            </select>
          </div>

          <p className="tracker-head">Keymap</p>
          <KeymapPiano keymap={x.keymap} range={range} onRange={setRange} onPlay={(n) => playNote(n)} width={300} />
          <p className="tracker-facts">{`${tk.noteText(lo + 1)}${hi > lo ? ` to ${tk.noteText(hi + 1)}` : ""} chosen (Shift+click for a range). Plays ${used.length === 0 ? "no samples" : used.map((s) => tk.hex2(s)).join(", ")}.`}</p>
          <div className="tracker-inline tracker-keymap">
            <select aria-label="Sample" value={mapTo} onChange={(ev) => setMapTo(Number(ev.currentTarget.value))}>
              <option value={0}>none</option>
              {doc.samples.map((s, i) => (
                <option key={i} value={i + 1}>{`${tk.hex2(i + 1)} ${s.name}`}</option>
              ))}
            </select>
            <select aria-label="The note it plays" value={playsAs === "same" ? -1 : fixedNote} onChange={(ev) => {
              const v = Number(ev.currentTarget.value);
              setPlaysAs(v < 0 ? "same" : "fixed");
              if (v >= 0) setFixedNote(v);
            }}>
              <option value={-1}>as played</option>
              {Array.from({ length: 120 }, (_, n) => (
                <option key={n} value={n}>{`as ${tk.noteText(n + 1)}`}</option>
              ))}
            </select>
            <button
              type="button"
              className="small"
              onClick={() => {
                const edits: tk.Edit[] = [];
                for (let n = lo; n <= hi; n++) edits.push({ op: "keymapNote", index, note: n, toNote: playsAs === "same" ? n : fixedNote, sample: mapTo });
                run(edits);
              }}
            >
              Map
            </button>
            <button type="button" className="small" title="Every note to this sample, as played" onClick={() => run([{ op: "keymapAll", index, sample: mapTo }])}>
              All
            </button>
          </div>

          <div className="tracker-tabs small" role="tablist" aria-label="Envelopes">
            {(["volumeEnv", "panEnv", "pitchEnv"] as const).map((k) => (
              <button key={k} type="button" role="tab" aria-selected={env === k} className={env === k ? "selected" : ""} onClick={() => setEnv(k)}>
                {k === "volumeEnv" ? "Volume" : k === "panEnv" ? "Panning" : "Pitch"}
              </button>
            ))}
          </div>
          {e && (
            <>
              <EnvelopeEditor env={e} bipolar={env !== "volumeEnv"} width={300} height={96} label={`The ${env === "volumeEnv" ? "volume" : env === "panEnv" ? "panning" : "pitch"} envelope`} onChange={setEnvelope} />
              <div className="tracker-inline tracker-env-flags">
                <label className="form-check">
                  <input type="checkbox" checked={e.enabled} onChange={(ev) => setEnvelope({ ...e, enabled: ev.currentTarget.checked })} />
                  On
                </label>
                <label className="form-check" title="Holds at a point while the note's held">
                  <input type="checkbox" checked={e.sustain !== null} onChange={(ev) => setEnvelope({ ...e, sustain: ev.currentTarget.checked && e.points.length > 0 ? [Math.min(point, e.points.length - 1), Math.min(point, e.points.length - 1)] : null })} />
                  Sustain
                </label>
                <label className="form-check" title="Loops from the first point to the last">
                  <input type="checkbox" checked={e.looped !== null} onChange={(ev) => setEnvelope({ ...e, looped: ev.currentTarget.checked && e.points.length > 1 ? [0, e.points.length - 1] : null })} />
                  Loop
                </label>
                <label className="form-check" title="Carries on from where the note before left it (Impulse Tracker's)">
                  <input type="checkbox" checked={e.carry} onChange={(ev) => setEnvelope({ ...e, carry: ev.currentTarget.checked })} />
                  Carry
                </label>
                {env === "pitchEnv" && (
                  <label className="form-check" title={`Drives the resonant filter instead of the pitch: from 0 Hz up to ${x.cutoff !== null ? filterHz(x.cutoff) : filterHz(127)} Hz, the cutoff`}>
                    <input type="checkbox" checked={e.filter} onChange={(ev) => setEnvelope({ ...e, filter: ev.currentTarget.checked })} />
                    Filter
                  </label>
                )}
              </div>
              <div className="tracker-inline tracker-env-flags">
                <label htmlFor={`env-point-${index}`}>Point</label>
                <select id={`env-point-${index}`} value={Math.min(point, Math.max(0, e.points.length - 1))} onChange={(ev) => setPoint(Number(ev.currentTarget.value))}>
                  {e.points.map(([t, v], i) => (
                    <option key={i} value={i}>{`${i + 1}: tick ${t}, ${v}`}</option>
                  ))}
                </select>
                <button
                  type="button"
                  className="small"
                  disabled={point >= e.points.length - 1}
                  title="A point halfway between this one and the next"
                  onClick={() => {
                    const [a, b] = [e.points[point], e.points[point + 1]];
                    const mid: [number, number] = [Math.round((a[0] + b[0]) / 2), Math.round((a[1] + b[1]) / 2)];
                    if (mid[0] === a[0] || mid[0] === b[0]) return;
                    const shift = (p: [number, number] | null) => (p ? ([p[0] > point ? p[0] + 1 : p[0], p[1] > point ? p[1] + 1 : p[1]] as [number, number]) : null);
                    setEnvelope({ ...e, points: [...e.points.slice(0, point + 1), mid, ...e.points.slice(point + 1)], looped: shift(e.looped), sustain: shift(e.sustain), release: e.release !== null && e.release > point ? e.release + 1 : e.release });
                  }}
                >
                  Insert After
                </button>
                <label className="form-check" title="Once the note's let go, the envelope goes on from this point, from where it was (OpenMPT's release node)">
                  <input type="checkbox" checked={e.release === point} onChange={(ev) => setEnvelope({ ...e, release: ev.currentTarget.checked ? point : null })} />
                  Release node
                </label>
              </div>
              <div className="tracker-inline tracker-env-flags">
                <button type="button" className="small" disabled={e.points.length < 2} title="Its length made its sample's, at the song's tempo" onClick={fit}>
                  Fit to Sample
                </button>
                <label htmlFor={`env-speed-${index}`}>Speed</label>
                <input id={`env-speed-${index}`} type="number" min={10} max={1000} value={speed} onChange={(ev) => setSpeed(Number(ev.currentTarget.value))} />%
                <button type="button" className="small" title="Its ticks spaced out (under 100%) or drawn in (over)" onClick={() => scale(100 / Math.max(1, speed))}>
                  Apply
                </button>
              </div>
              <p className="tracker-facts">A click adds a point, a drag moves one, a double-click takes one out.</p>
            </>
          )}

          <p className="tracker-head">Phrase</p>
          <div className="tracker-inline tracker-env-flags">
            <label className="form-check" title="Its notes play a phrase: these rows, a row a row, transposed to the note">
              <input type="checkbox" checked={!!x.phrase} onChange={(ev) => change({ phrase: ev.currentTarget.checked ? { cells: [[61, 0, 0, 0, 0, 0]], looped: false, base: 61 } : null })} />
              Plays a phrase
            </label>
            {x.phrase && (
              <>
                <label className="form-check">
                  <input type="checkbox" checked={x.phrase.looped} onChange={(ev) => x.phrase && change({ phrase: { ...x.phrase, looped: ev.currentTarget.checked } })} />
                  Loop
                </label>
                <label htmlFor={`phrase-base-${index}`}>Written for</label>
                <select id={`phrase-base-${index}`} value={x.phrase.base} onChange={(ev) => x.phrase && change({ phrase: { ...x.phrase, base: Number(ev.currentTarget.value) } })}>
                  {Array.from({ length: 120 }, (_, n) => (
                    <option key={n} value={n + 1}>
                      {tk.noteText(n + 1)}
                    </option>
                  ))}
                </select>
              </>
            )}
          </div>
          {x.phrase && (
            <>
              <textarea
                className="tracker-message tracker-phrase"
                aria-label="The phrase, a row a line"
                spellCheck={false}
                value={phrase}
                onChange={(ev) => setPhrase(ev.currentTarget.value)}
                onBlur={() => x.phrase && change({ phrase: { ...x.phrase, cells: parsePhrase(phrase, volCode, fxCode) } })}
              />
              <p className="tracker-facts">A row a line, as the pattern writes it: "C-5 .. v40 A06" (note, instrument, volume column, effect; dots for none). A note in the pattern starts it, transposed from the note it's written for.</p>
            </>
          )}
        </>
      )}
    </div>
  );
}
