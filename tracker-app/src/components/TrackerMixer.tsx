/**
 * The tracker's Mix and Automation panes.
 *
 * - Mix: each channel's fader as the song plays (Impulse Tracker's info
 *   screen: a level and a panning over the song's, not saved), and the
 *   song's effect slots (OpenMPT's plugins, Renoise's DSP chains, built
 *   in): each an effect, its settings, how much is heard, and where its
 *   sound goes (a later slot, a chain, or the mix).
 * - Automation: the pattern's lanes (Renoise's automation, OpenMPT's
 *   parameter control events): the global volume, the tempo, a channel's
 *   volume or panning, or an effect's parameter, drawn over its rows.
 */
import { useEffect, useState } from "react";
import * as tk from "../lib/tracker";
import { LaneEditor } from "./TrackerParts";

/** A slider that shows its value as it's dragged and sends it when let go. */
function CommitRange({ label, value, min, max, step, onCommit, display }: { label: string; value: number; min: number; max: number; step: number; onCommit: (v: number) => void; display?: (v: number) => string }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  const commit = () => v !== value && onCommit(v);
  return (
    <label className="tracker-range">
      <span>{label}</span>
      <input type="range" min={min} max={max} step={step} value={v} aria-label={label} aria-valuetext={display ? display(v) : String(v)} onChange={(e) => setV(Number(e.currentTarget.value))} onMouseUp={commit} onKeyUp={commit} onBlur={commit} onTouchEnd={commit} />
      <span className="tracker-range-value">{display ? display(v) : v.toFixed(2)}</span>
    </label>
  );
}

interface MixProps {
  doc: tk.Doc;
  letters: tk.Letters | null;
  run: (edits: tk.Edit[], then?: (d: tk.Doc) => void) => void;
  levels: number[];
  showError: (m: string) => void;
}

export function MixPane({ doc, letters, run, levels, showError }: MixProps) {
  const [faders, setFaders] = useState<{ gain: number; pan: number | null }[]>([]);
  const [kind, setKind] = useState(0);
  // A new song starts with every fader as the song has it.
  useEffect(() => setFaders(doc.channels.map(() => ({ gain: 1, pan: null }))), [doc.name, doc.channels.length]);
  const setFader = (ch: number, f: { gain: number; pan: number | null }) => {
    setFaders((all) => all.map((x, i) => (i === ch ? f : x)));
    tk.fader(ch, f.gain, f.pan).catch((e) => showError(String(e)));
  };
  const kinds = letters?.effectKinds ?? [];
  const setEffects = (effects: tk.EffectSlot[]) => run([{ op: "effects", effects }]);
  const slot = (i: number, patch: Partial<tk.EffectSlot>) => setEffects(doc.effects.map((e, k) => (k === i ? { ...e, ...patch } : e)));
  const nameOf = (i: number) => `${i + 1}: ${doc.effects[i]?.name || kinds[doc.effects[i]?.kind ?? 0]?.[0] || "Effect"}`;
  return (
    <div className="tracker-pane" data-testid="tracker-mix">
      <p className="tracker-head">Faders</p>
      <p className="tracker-facts">As the song plays, over its own levels; not saved with it.</p>
      <div className="tracker-faders">
        {doc.channels.map((c, ch) => {
          const f = faders[ch] ?? { gain: 1, pan: null };
          return (
            <div key={ch} className="tracker-fader">
              <span className="tracker-fader-name" title={c.name}>{`${String(ch + 1).padStart(2, "0")} ${c.name}`}</span>
              <span className="tracker-fader-vu" aria-hidden="true">
                <span style={{ width: `${Math.round(Math.min(1, levels[ch] ?? 0) * 100)}%` }} />
              </span>
              <input type="range" min={0} max={2} step={0.01} value={f.gain} aria-label={`Channel ${ch + 1}'s level`} aria-valuetext={`${Math.round(f.gain * 100)}%`} onChange={(e) => setFader(ch, { ...f, gain: Number(e.currentTarget.value) })} onDoubleClick={() => setFader(ch, { ...f, gain: 1 })} />
              <input type="range" min={0} max={1} step={0.01} value={f.pan ?? c.pan / 255} aria-label={`Channel ${ch + 1}'s panning`} className={f.pan === null ? "as-song" : undefined} onChange={(e) => setFader(ch, { ...f, pan: Number(e.currentTarget.value) })} onDoubleClick={() => setFader(ch, { ...f, pan: null })} title="Double-click: the song's panning" />
            </div>
          );
        })}
      </div>
      <button type="button" className="small" onClick={() => doc.channels.forEach((_, ch) => setFader(ch, { gain: 1, pan: null }))}>
        All as the Song Has Them
      </button>

      <p className="tracker-head">Effect slots</p>
      <div className="tracker-inline">
        <select aria-label="The effect to add" value={kind} onChange={(e) => setKind(Number(e.currentTarget.value))}>
          {kinds.map(([n], i) => (
            <option key={n} value={i}>
              {n}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="small"
          data-testid="tracker-add-effect"
          disabled={doc.effects.length >= 32}
          onClick={() => setEffects([...doc.effects, { kind, name: "", params: (kinds[kind]?.[1] ?? []).map(([, d]) => d), mix: [0, 1, 2].includes(kind) ? 0.35 : 1, bypass: false, output: null }])}
        >
          Add
        </button>
      </div>
      {doc.effects.length === 0 && <p className="tracker-facts">None. A channel (in the Song tab) or an instrument plays through a slot once it has one; a slot's sound can go on into a later one.</p>}
      {doc.effects.map((e, i) => (
        <div key={i} className="tracker-effect">
          <div className="tracker-inline">
            <span className="tracker-head">{nameOf(i)}</span>
            <label className="form-check">
              <input type="checkbox" checked={e.bypass} onChange={(ev) => slot(i, { bypass: ev.currentTarget.checked })} />
              Bypass
            </label>
            <button
              type="button"
              className="small danger"
              onClick={() => setEffects(doc.effects.filter((_, k) => k !== i).map((x) => ({ ...x, output: x.output === null ? null : x.output === i ? null : x.output > i ? x.output - 1 : x.output })))}
            >
              Remove
            </button>
          </div>
          <div className="tracker-inline">
            <label htmlFor={`fx-out-${i}`}>Into</label>
            <select id={`fx-out-${i}`} value={e.output ?? -1} onChange={(ev) => slot(i, { output: Number(ev.currentTarget.value) < 0 ? null : Number(ev.currentTarget.value) })}>
              <option value={-1}>The mix</option>
              {doc.effects.map((_, k) => k > i && (
                <option key={k} value={k}>
                  {nameOf(k)}
                </option>
              ))}
            </select>
            <input key={e.name} type="text" aria-label="Its name" placeholder="Name" defaultValue={e.name} onBlur={(ev) => ev.currentTarget.value !== e.name && slot(i, { name: ev.currentTarget.value })} />
          </div>
          <CommitRange label="Mix" value={e.mix} min={0} max={1} step={0.01} display={(v) => `${Math.round(v * 100)}%`} onCommit={(mix) => slot(i, { mix })} />
          {(kinds[e.kind]?.[1] ?? []).map(([pn], k) => (
            <CommitRange key={pn} label={pn} value={e.params[k] ?? 0} min={0} max={1} step={0.01} display={(v) => `${Math.round(v * 100)}%`} onCommit={(v) => slot(i, { params: e.params.map((p, q) => (q === k ? v : p)) })} />
          ))}
        </div>
      ))}
    </div>
  );
}

interface AutomationProps {
  doc: tk.Doc;
  pat: tk.Pattern | null;
  letters: tk.Letters | null;
  run: (edits: tk.Edit[], then?: (d: tk.Doc) => void) => void;
  playingRow: number;
}

function targetName(t: tk.Target, doc: tk.Doc, letters: tk.Letters | null): string {
  switch (t.kind) {
    case "globalVolume":
      return "Global volume";
    case "tempo":
      return "Tempo";
    case "channelVolume":
      return `Channel ${t.a + 1}'s volume`;
    case "channelPan":
      return `Channel ${t.a + 1}'s panning`;
    case "effect": {
      const e = doc.effects[t.a];
      const k = letters?.effectKinds[e?.kind ?? 0];
      return `Slot ${t.a + 1} ${k?.[0] ?? ""}: ${k?.[1][t.b]?.[0] ?? `parameter ${t.b + 1}`}`;
    }
    default:
      return "Nothing";
  }
}

/** Everything a lane can move, as targets. */
function targets(doc: tk.Doc, letters: tk.Letters | null): tk.Target[] {
  const t: tk.Target[] = [
    { kind: "globalVolume", a: 0, b: 0 },
    { kind: "tempo", a: 0, b: 0 },
  ];
  doc.channels.forEach((_, c) => t.push({ kind: "channelVolume", a: c, b: 0 }, { kind: "channelPan", a: c, b: 0 }));
  doc.effects.forEach((e, s) => (letters?.effectKinds[e.kind]?.[1] ?? []).forEach((_, p) => t.push({ kind: "effect", a: s, b: p })));
  return t;
}

export function AutomationPane({ doc, pat, letters, run, playingRow }: AutomationProps) {
  const [lane, setLane] = useState(0);
  const [adding, setAdding] = useState(0);
  if (!pat) return <div className="tracker-pane" />;
  const lanes = pat.lanes;
  const all = targets(doc, letters);
  const set = (next: tk.Lane[]) => run([{ op: "lanes", pattern: pat.index, lanes: next }]);
  const l = lanes[Math.min(lane, lanes.length - 1)];
  const value = (t: tk.Target, v: number) => (t.kind === "tempo" ? `${Math.round(32 + v * 480)} bpm` : t.kind === "globalVolume" ? `${Math.round(v * 128)}` : t.kind === "channelVolume" ? `${Math.round(v * 64)}` : `${Math.round(v * 100)}%`);
  return (
    <div className="tracker-pane" data-testid="tracker-automation">
      <p className="tracker-facts">{`Pattern ${pat.index}'s lanes: each moves something over its rows, in straight lines between the points, tick by tick. Before its first point and after its last, it leaves it be.`}</p>
      <div className="tracker-inline">
        <select aria-label="What a new lane moves" value={adding} onChange={(e) => setAdding(Number(e.currentTarget.value))}>
          {all.map((t, i) => (
            <option key={i} value={i}>
              {targetName(t, doc, letters)}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="small"
          data-testid="tracker-add-lane"
          onClick={() => {
            set([...lanes, { target: all[adding], points: [[0, 1], [pat.rows - 1, 1]] }]);
            setLane(lanes.length);
          }}
        >
          Add Lane
        </button>
      </div>
      {lanes.length > 0 && l && (
        <>
          <div className="tracker-inline">
            <select aria-label="Lane" value={Math.min(lane, lanes.length - 1)} onChange={(e) => setLane(Number(e.currentTarget.value))}>
              {lanes.map((x, i) => (
                <option key={i} value={i}>
                  {targetName(x.target, doc, letters)}
                </option>
              ))}
            </select>
            <button type="button" className="small danger" onClick={() => set(lanes.filter((_, i) => i !== lane))}>
              Remove
            </button>
          </div>
          <LaneEditor points={l.points} rows={pat.rows} width={300} height={110} now={playingRow} label={targetName(l.target, doc, letters)} onChange={(points) => set(lanes.map((x, i) => (i === lane ? { ...x, points } : x)))} />
          <p className="tracker-facts">{l.points.map(([r, v]) => `row ${r}: ${value(l.target, v)}`).join(" · ")}</p>
          <p className="tracker-facts">A click adds a point, a drag moves one, a double-click takes one out.</p>
        </>
      )}
    </div>
  );
}
