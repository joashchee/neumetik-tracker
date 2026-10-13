/**
 * Pieces the tracker draws with: a waveform (with its loops marked),
 * the sample editor's waveform (marked, zoomed, its loops dragged, drawn
 * on, sliced), an envelope and an automation lane edited by hand, a
 * keymap as a piano, the fields the panes are made of, and a song's
 * comments with their formatting. The rest of the tab is TrackerPanel.tsx
 * (the editor), TrackerSamples.tsx, TrackerInstruments.tsx,
 * TrackerSong.tsx, TrackerMixer.tsx and TrackerDialogs.tsx.
 */
import { Fragment, useEffect, useId, useRef, useState, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";
import type { Envelope } from "../lib/tracker";
import { noteText } from "../lib/tracker";

/** The accent and line colors, read from the theme so the canvas follows it. */
function themeColor(name: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

interface WaveformProps {
  /** [low, high] a column, -1 to 1. */
  peaks: [number, number][] | null;
  width: number;
  height: number;
  frames: number;
  /** The loop and sustain loop, in frames, when they're on. */
  loop?: [number, number] | null;
  sustain?: [number, number] | null;
  label: string;
  testId?: string;
}

export function Waveform({ peaks, width, height, frames, loop, sustain, label, testId }: WaveformProps) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    const g = c.getContext("2d");
    if (!g) return;
    const ratio = window.devicePixelRatio || 1;
    c.width = width * ratio;
    c.height = height * ratio;
    g.scale(ratio, ratio);
    g.clearRect(0, 0, width, height);
    const mid = height / 2;
    g.fillStyle = themeColor("--line", "#2c313a");
    g.fillRect(0, mid, width, 1);
    if (peaks && peaks.length > 0) {
      g.fillStyle = themeColor("--accent", "#2dd4bf");
      const w = width / peaks.length;
      peaks.forEach(([lo, hi], i) => {
        const y1 = mid - hi * (mid - 1);
        const y2 = mid - lo * (mid - 1);
        g.fillRect(i * w, y1, Math.max(1, w), Math.max(1, y2 - y1));
      });
    }
    const mark = (range: [number, number], color: string) => {
      if (frames <= 0) return;
      const [a, b] = range.map((f) => (f / frames) * width);
      g.fillStyle = color;
      g.globalAlpha = 0.18;
      g.fillRect(a, 0, b - a, height);
      g.globalAlpha = 1;
      g.fillRect(a, 0, 1, height);
      g.fillRect(Math.max(0, b - 1), 0, 1, height);
    };
    if (loop) mark(loop, themeColor("--good", "#4ade80"));
    if (sustain) mark(sustain, themeColor("--warn", "#f2545b"));
  }, [peaks, width, height, frames, loop, sustain]);
  return <canvas ref={canvas} className="tracker-wave" style={{ width, height }} role="img" aria-label={label} data-testid={testId} />;
}

interface EnvelopeEditorProps {
  env: Envelope;
  /** Volume (0 to 64), or panning and pitch (-32 to 32). */
  bipolar: boolean;
  width: number;
  height: number;
  label: string;
  onChange: (env: Envelope) => void;
}

/**
 * An envelope as points on a line: a click on empty ground adds a point,
 * a drag moves one (its tick held between its neighbours'), a
 * double-click takes one out (the first stays). The sustain is marked
 * red and the loop green.
 */
export function EnvelopeEditor({ env, bipolar, width, height, label, onChange }: EnvelopeEditorProps) {
  const [drag, setDrag] = useState<number | null>(null);
  const [draft, setDraft] = useState<Envelope | null>(null);
  const shown = draft ?? env;
  const lastTick = Math.max(32, ...shown.points.map((p) => p[0])) * 1.15;
  const lo = bipolar ? -32 : 0;
  const hi = bipolar ? 32 : 64;
  const x = (t: number) => 6 + (t / lastTick) * (width - 12);
  const y = (v: number) => 6 + (1 - (v - lo) / (hi - lo)) * (height - 12);
  const toPoint = (e: ReactMouseEvent<SVGSVGElement>): [number, number] => {
    const r = e.currentTarget.getBoundingClientRect();
    const sx = ((e.clientX - r.left) / r.width) * width;
    const sy = ((e.clientY - r.top) / r.height) * height;
    const t = Math.max(0, Math.round(((sx - 6) / (width - 12)) * lastTick));
    const v = Math.round(lo + (1 - (sy - 6) / (height - 12)) * (hi - lo));
    return [t, Math.max(lo, Math.min(hi, v))];
  };
  const near = (p: [number, number]) => shown.points.findIndex(([t, v]) => Math.abs(x(t) - x(p[0])) < 6 && Math.abs(y(v) - y(p[1])) < 6);

  return (
    <svg
      className="tracker-env"
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={`${label}: ${shown.points.length} points${shown.enabled ? "" : ", off"}`}
      onMouseDown={(e) => {
        const p = toPoint(e);
        const i = near(p);
        if (i >= 0) {
          setDrag(i);
          setDraft(env);
          return;
        }
        // A new point, in tick order.
        const points = [...env.points, p].sort((a, b) => a[0] - b[0]);
        onChange({ ...env, enabled: true, points });
      }}
      onMouseMove={(e) => {
        if (drag === null || !draft) return;
        const [t, v] = toPoint(e);
        const pts = draft.points.map((q) => [...q] as [number, number]);
        const before = drag > 0 ? pts[drag - 1][0] + 1 : 0;
        const after = drag < pts.length - 1 ? pts[drag + 1][0] - 1 : 65535;
        pts[drag] = [drag === 0 ? 0 : Math.max(before, Math.min(after, t)), v];
        setDraft({ ...draft, points: pts });
      }}
      onMouseUp={() => {
        if (draft && drag !== null) onChange(draft);
        setDrag(null);
        setDraft(null);
      }}
      onMouseLeave={() => {
        if (draft && drag !== null) onChange(draft);
        setDrag(null);
        setDraft(null);
      }}
      onDoubleClick={(e) => {
        const i = near(toPoint(e));
        if (i > 0) {
          const points = env.points.filter((_, k) => k !== i);
          const fix = (p: [number, number] | null) => (p && p[1] < points.length ? p : null);
          onChange({ ...env, points, looped: fix(env.looped), sustain: fix(env.sustain) });
        }
      }}
    >
      <rect x={0} y={0} width={width} height={height} className="tracker-env-ground" />
      {bipolar && <line x1={0} x2={width} y1={y(0)} y2={y(0)} className="tracker-env-axis" />}
      {shown.looped && shown.points[shown.looped[1]] && (
        <rect x={x(shown.points[shown.looped[0]][0])} y={0} width={Math.max(1, x(shown.points[shown.looped[1]][0]) - x(shown.points[shown.looped[0]][0]))} height={height} className="tracker-env-loop" />
      )}
      {shown.sustain && shown.points[shown.sustain[0]] && <line x1={x(shown.points[shown.sustain[0]][0])} x2={x(shown.points[shown.sustain[0]][0])} y1={0} y2={height} className="tracker-env-sustain" />}
      <polyline points={shown.points.map(([t, v]) => `${x(t)},${y(v)}`).join(" ")} className={`tracker-env-line${shown.enabled ? "" : " off"}`} />
      {shown.points.map(([t, v], i) => (
        <rect key={i} x={x(t) - 3} y={y(v) - 3} width={6} height={6} className="tracker-env-point" />
      ))}
    </svg>
  );
}

/** A number field that sends its value when it's left or Enter is pressed, not every keystroke. */
export function NumberField({ label, value, min, max, onCommit, testId, title, step }: { label: string; value: number; min: number; max: number; onCommit: (v: number) => void; testId?: string; title?: string; step?: number }) {
  const id = useId();
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value]);
  const commit = () => {
    const raw = Number(text);
    const v = Math.max(min, Math.min(max, step && step < 1 ? raw : Math.round(raw)));
    if (Number.isFinite(v) && v !== value) onCommit(v);
    else setText(String(value));
  };
  return (
    <>
      <label htmlFor={id} title={title}>
        {label}
      </label>
      <input
        id={id}
        type="number"
        min={min}
        max={max}
        step={step}
        value={text}
        data-testid={testId}
        onChange={(e) => setText(e.currentTarget.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
        }}
      />
    </>
  );
}

export function TextField({ label, value, onCommit, testId, wide, title }: { label: string; value: string; onCommit: (v: string) => void; testId?: string; wide?: boolean; title?: string }) {
  const id = useId();
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  return (
    <>
      <label htmlFor={id} title={title}>
        {label}
      </label>
      <input
        id={id}
        type="text"
        className={wide ? "tracker-wide" : undefined}
        value={text}
        data-testid={testId}
        onChange={(e) => setText(e.currentTarget.value)}
        onBlur={() => text !== value && onCommit(text)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && text !== value) onCommit(text);
        }}
      />
    </>
  );
}

/** What the sample editor's mouse does. */
export type WaveTool = "select" | "draw";

interface SampleCanvasProps {
  /** [low, high] a column over the frames in view. */
  peaks: [number, number][] | null;
  width: number;
  height: number;
  frames: number;
  /** The frames in view. */
  view: [number, number];
  selection: [number, number] | null;
  loop: [number, number] | null;
  sustain: [number, number] | null;
  slices: number[];
  tool: WaveTool;
  label: string;
  onSelect: (s: [number, number] | null) => void;
  onView: (v: [number, number]) => void;
  /** A loop's start or end dragged to a frame. */
  onLoopEdge: (which: "loopStart" | "loopEnd" | "susStart" | "susEnd", frame: number) => void;
  /** Points drawn from a frame: one value a frame, -32768 to 32767. */
  onDraw: (at: number, points: number[]) => void;
  /** A slice added (or one near taken away) at a frame. */
  onSlice: (frame: number) => void;
  /** Played from a frame (a double-click). */
  onPlayFrom: (frame: number) => void;
}

/**
 * The sample editor's waveform: a drag marks a range, a click puts the
 * cursor; the loops' edges (green, the sustain loop's red) drag; Shift+click
 * adds or takes away a slice (the blue lines); the wheel scrolls and,
 * with Cmd or Ctrl, zooms; a double-click plays from there. With the
 * pencil, a drag draws the waveform.
 */
export function SampleCanvas({ peaks, width, height, frames, view, selection, loop, sustain, slices, tool, label, onSelect, onView, onLoopEdge, onDraw, onSlice, onPlayFrom }: SampleCanvasProps) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const drag = useRef<null | { kind: "select"; from: number } | { kind: "edge"; which: "loopStart" | "loopEnd" | "susStart" | "susEnd" } | { kind: "draw"; points: Map<number, number> }>(null);
  const [edge, setEdge] = useState<null | { which: string; frame: number }>(null);
  const [drawn, setDrawn] = useState<Map<number, number> | null>(null);
  const [a, b] = view;
  const span = Math.max(1, b - a);
  const toX = (f: number) => ((f - a) / span) * width;
  const toFrame = (x: number) => Math.round(Math.max(0, Math.min(frames, a + (x / width) * span)));
  const frameAt = (e: { clientX: number; currentTarget: HTMLCanvasElement }) => {
    const r = e.currentTarget.getBoundingClientRect();
    return toFrame(((e.clientX - r.left) / r.width) * width);
  };
  const valueAt = (e: { clientY: number; currentTarget: HTMLCanvasElement }) => {
    const r = e.currentTarget.getBoundingClientRect();
    const y = ((e.clientY - r.top) / r.height) * height;
    return Math.round(Math.max(-1, Math.min(1, 1 - (2 * y) / height)) * 32767);
  };

  useEffect(() => {
    const c = canvas.current;
    if (!c) return;
    const g = c.getContext("2d");
    if (!g) return;
    const ratio = window.devicePixelRatio || 1;
    c.width = width * ratio;
    c.height = height * ratio;
    g.scale(ratio, ratio);
    g.clearRect(0, 0, width, height);
    const mid = height / 2;
    if (selection && selection[1] > selection[0]) {
      g.fillStyle = themeColor("--accent", "#2dd4bf");
      g.globalAlpha = 0.22;
      g.fillRect(toX(selection[0]), 0, Math.max(1, toX(selection[1]) - toX(selection[0])), height);
      g.globalAlpha = 1;
    }
    g.fillStyle = themeColor("--line", "#2c313a");
    g.fillRect(0, mid, width, 1);
    if (peaks && peaks.length > 0) {
      g.fillStyle = themeColor("--accent", "#2dd4bf");
      const w = width / peaks.length;
      peaks.forEach(([lo, hi], i) => {
        const y1 = mid - hi * (mid - 1);
        const y2 = mid - lo * (mid - 1);
        g.fillRect(i * w, y1, Math.max(1, w), Math.max(1, y2 - y1));
      });
    }
    if (drawn) {
      g.fillStyle = "#ffff55";
      for (const [f, v] of drawn) g.fillRect(toX(f), mid - (v / 32768) * (mid - 1), 2, 2);
    }
    const mark = (range: [number, number], color: string, edges: [string, string]) => {
      const s = edge && edge.which === edges[0] ? edge.frame : range[0];
      const e2 = edge && edge.which === edges[1] ? edge.frame : range[1];
      g.fillStyle = color;
      g.globalAlpha = 0.14;
      g.fillRect(toX(s), 0, toX(e2) - toX(s), height);
      g.globalAlpha = 1;
      for (const x of [toX(s), toX(e2)]) {
        g.fillRect(x - 1, 0, 2, height);
        g.fillRect(x - 4, 0, 8, 6);
      }
    };
    if (loop) mark(loop, themeColor("--good", "#4ade80"), ["loopStart", "loopEnd"]);
    if (sustain) mark(sustain, themeColor("--warn", "#f2545b"), ["susStart", "susEnd"]);
    g.fillStyle = "#5599ff";
    for (const sl of slices) if (sl >= a && sl <= b) g.fillRect(toX(sl), 0, 1, height);
    if (selection && selection[1] === selection[0]) {
      g.fillStyle = "#ffff55";
      g.fillRect(toX(selection[0]), 0, 1, height);
    }
    // toX reads the view, which the deps cover.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [peaks, width, height, frames, a, b, selection, loop, sustain, slices, edge, drawn]);

  /** A loop edge near the mouse, by its handle. */
  const edgeNear = (x: number): "loopStart" | "loopEnd" | "susStart" | "susEnd" | null => {
    const near = (f: number) => Math.abs(toX(f) - x) < 5;
    if (loop && near(loop[0])) return "loopStart";
    if (loop && near(loop[1])) return "loopEnd";
    if (sustain && near(sustain[0])) return "susStart";
    if (sustain && near(sustain[1])) return "susEnd";
    return null;
  };

  return (
    <canvas
      ref={canvas}
      className={`tracker-wave tracker-sample-canvas${tool === "draw" ? " drawing" : ""}`}
      style={{ width, height }}
      role="img"
      aria-label={label}
      onMouseDown={(e) => {
        if (frames === 0) return;
        const r = e.currentTarget.getBoundingClientRect();
        const x = ((e.clientX - r.left) / r.width) * width;
        const f = frameAt(e);
        if (e.shiftKey && tool === "select") {
          onSlice(f);
          return;
        }
        const ed = tool === "select" ? edgeNear(x) : null;
        if (ed) {
          drag.current = { kind: "edge", which: ed };
          setEdge({ which: ed, frame: f });
        } else if (tool === "draw") {
          const points = new Map([[f, valueAt(e)]]);
          drag.current = { kind: "draw", points };
          setDrawn(new Map(points));
        } else {
          drag.current = { kind: "select", from: f };
          onSelect([f, f]);
        }
      }}
      onMouseMove={(e) => {
        const d = drag.current;
        if (!d) return;
        const f = frameAt(e);
        if (d.kind === "select") onSelect([Math.min(d.from, f), Math.max(d.from, f)]);
        else if (d.kind === "edge") setEdge({ which: d.which, frame: f });
        else {
          // Every frame between the last drawn and this one, in a straight line.
          const keys = [...d.points.keys()];
          const last = keys[keys.length - 1];
          const lv = d.points.get(last) ?? 0;
          const v = valueAt(e);
          const step = f > last ? 1 : -1;
          for (let k = last; k !== f + step && Math.abs(k - last) <= 65536; k += step) {
            const t = f === last ? 1 : (k - last) / (f - last);
            d.points.set(k, Math.round(lv + (v - lv) * t));
          }
          setDrawn(new Map(d.points));
        }
      }}
      onMouseUp={() => {
        const d = drag.current;
        drag.current = null;
        if (d?.kind === "edge" && edge) onLoopEdge(d.which, edge.frame);
        if (d?.kind === "draw") {
          const keys = [...d.points.keys()].sort((x, y) => x - y);
          if (keys.length > 0) {
            const from = keys[0];
            const pts: number[] = [];
            for (let k = from; k <= keys[keys.length - 1]; k++) pts.push(d.points.get(k) ?? 0);
            onDraw(from, pts);
          }
        }
        setEdge(null);
        setDrawn(null);
      }}
      onMouseLeave={() => {
        if (drag.current?.kind === "edge" && edge) onLoopEdge(drag.current.which, edge.frame);
        drag.current = null;
        setEdge(null);
        setDrawn(null);
      }}
      onDoubleClick={(e) => onPlayFrom(frameAt(e))}
      onWheel={(e) => {
        if (frames === 0) return;
        if (e.metaKey || e.ctrlKey) {
          // Zoom round the mouse.
          const f = frameAt(e);
          const k = e.deltaY > 0 ? 1.25 : 0.8;
          const len = Math.max(16, Math.min(frames, Math.round(span * k)));
          const t = (f - a) / span;
          const start = Math.max(0, Math.min(frames - len, Math.round(f - t * len)));
          onView([start, start + len]);
        } else {
          const d = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
          const move = Math.round((Math.sign(d) * span) / 8);
          const start = Math.max(0, Math.min(frames - span, a + move));
          if (start !== a) onView([start, start + span]);
        }
      }}
    />
  );
}

/** A color for a sample number, for a keymap. */
export function sampleHue(sample: number): string {
  return sample <= 0 ? "#222222" : `hsl(${(sample * 47) % 360} 60% 45%)`;
}

const BLACK = [1, 3, 6, 8, 10];

interface KeymapPianoProps {
  /** For each of 120 notes: [note played (0-based), 1-based sample]. */
  keymap: [number, number][];
  /** The notes chosen, from and to (0-based). */
  range: [number, number];
  onRange: (r: [number, number]) => void;
  onPlay: (note: number) => void;
  width: number;
}

/**
 * A keymap as a piano of ten octaves (Impulse Tracker's note-sample table,
 * FastTracker 2's keyboard): each key the color of the sample it plays. A
 * click chooses a key, Shift+click a range; a double-click plays it.
 */
export function KeymapPiano({ keymap, range, onRange, onPlay, width }: KeymapPianoProps) {
  const whites = 70;
  const ww = width / whites;
  const h = 46;
  const [lo, hi] = [Math.min(range[0], range[1]), Math.max(range[0], range[1])];
  const keys: ReactNode[] = [];
  const blacks: ReactNode[] = [];
  let w = 0;
  for (let n = 0; n < 120; n++) {
    const black = BLACK.includes(n % 12);
    const [plays, s] = keymap[n] ?? [n, 0];
    const chosen = n >= lo && n <= hi;
    const title = `${noteText(n + 1)}: ${s ? `sample ${s.toString(16).toUpperCase().padStart(2, "0")}${plays !== n ? `, as ${noteText(plays + 1)}` : ""}` : "nothing"}`;
    const common = {
      onMouseDown: (e: ReactMouseEvent) => {
        e.preventDefault();
        onRange(e.shiftKey ? [range[0], n] : [n, n]);
      },
      onDoubleClick: () => onPlay(n + 1),
    };
    if (black) {
      blacks.push(
        <rect key={n} {...common} x={w * ww - ww * 0.32} y={0} width={ww * 0.64} height={h * 0.6} fill={s ? sampleHue(s) : "#111111"} stroke={chosen ? "#ffffff" : "#000000"} strokeWidth={chosen ? 1.5 : 0.5}>
          <title>{title}</title>
        </rect>,
      );
    } else {
      keys.push(
        <rect key={n} {...common} x={w * ww} y={0} width={ww} height={h} fill={s ? sampleHue(s) : "#dddddd"} fillOpacity={s ? 0.85 : 1} stroke={chosen ? "#ffffff" : "#555555"} strokeWidth={chosen ? 1.5 : 0.5}>
          <title>{title}</title>
        </rect>,
      );
      w++;
    }
  }
  return (
    <svg className="tracker-piano" width={width} height={h} viewBox={`0 0 ${width} ${h}`} role="img" aria-label={`The keymap, ${noteText(lo + 1)} to ${noteText(hi + 1)} chosen`}>
      {keys}
      {blacks}
      {Array.from({ length: 10 }, (_, o) => (
        <text key={o} x={o * 7 * ww + 1} y={h - 3} className="tracker-piano-octave">
          {o}
        </text>
      ))}
    </svg>
  );
}

interface LaneEditorProps {
  points: [number, number][];
  rows: number;
  width: number;
  height: number;
  /** The row playing, when the pattern is. */
  now: number;
  label: string;
  onChange: (points: [number, number][]) => void;
}

/**
 * An automation lane: values (0 to 1) at rows, joined by lines. A click on
 * empty ground adds a point, a drag moves one, a double-click takes one out.
 */
export function LaneEditor({ points, rows, width, height, now, label, onChange }: LaneEditorProps) {
  const [drag, setDrag] = useState<number | null>(null);
  const [draft, setDraft] = useState<[number, number][] | null>(null);
  const shown = draft ?? points;
  const x = (r: number) => 6 + (r / Math.max(1, rows - 1)) * (width - 12);
  const y = (v: number) => 6 + (1 - v) * (height - 12);
  const toPoint = (e: ReactMouseEvent<SVGSVGElement>): [number, number] => {
    const r = e.currentTarget.getBoundingClientRect();
    const sx = ((e.clientX - r.left) / r.width) * width;
    const sy = ((e.clientY - r.top) / r.height) * height;
    const row = Math.max(0, Math.min(rows - 1, Math.round(((sx - 6) / (width - 12)) * (rows - 1))));
    return [row, Math.max(0, Math.min(1, 1 - (sy - 6) / (height - 12)))];
  };
  const near = (p: [number, number]) => shown.findIndex(([r, v]) => Math.abs(x(r) - x(p[0])) < 6 && Math.abs(y(v) - y(p[1])) < 8);
  const sorted = (ps: [number, number][]) => [...ps].sort((p, q) => p[0] - q[0]).filter((p, i, all) => i === 0 || all[i - 1][0] !== p[0]);
  return (
    <svg
      className="tracker-env"
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={`${label}: ${shown.length} points`}
      onMouseDown={(e) => {
        const p = toPoint(e);
        const i = near(p);
        if (i >= 0) {
          setDrag(i);
          setDraft(points);
          return;
        }
        onChange(sorted([...points, p]));
      }}
      onMouseMove={(e) => {
        if (drag === null || !draft) return;
        const p = toPoint(e);
        const next = draft.map((q, i) => (i === drag ? p : q));
        setDraft(next);
      }}
      onMouseUp={() => {
        if (draft && drag !== null) onChange(sorted(draft));
        setDrag(null);
        setDraft(null);
      }}
      onMouseLeave={() => {
        if (draft && drag !== null) onChange(sorted(draft));
        setDrag(null);
        setDraft(null);
      }}
      onDoubleClick={(e) => {
        const i = near(toPoint(e));
        if (i >= 0) onChange(points.filter((_, k) => k !== i));
      }}
    >
      <rect x={0} y={0} width={width} height={height} className="tracker-env-ground" />
      {Array.from({ length: Math.floor(rows / 16) + 1 }, (_, k) => (
        <line key={k} x1={x(k * 16)} x2={x(k * 16)} y1={0} y2={height} className="tracker-env-axis" />
      ))}
      {now >= 0 && <line x1={x(now)} x2={x(now)} y1={0} y2={height} className="tracker-env-sustain" />}
      <polyline points={shown.map(([r, v]) => `${x(r)},${y(v)}`).join(" ")} className="tracker-env-line" />
      {shown.map(([r, v], i) => (
        <rect key={i} x={x(r) - 3} y={y(v) - 3} width={6} height={6} className="tracker-env-point" />
      ))}
    </svg>
  );
}

/** A line of comment text with its **bold**, *italic* and `code` made so. */
function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  const re = /(\*\*[^*]+\*\*|\*[^*]+\*|`[^`]+`)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let k = 0;
  while ((m = re.exec(text))) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const t = m[0];
    if (t.startsWith("**")) out.push(<strong key={k++}>{t.slice(2, -2)}</strong>);
    else if (t.startsWith("`")) out.push(<code key={k++}>{t.slice(1, -1)}</code>);
    else out.push(<em key={k++}>{t.slice(1, -1)}</em>);
    last = m.index + t.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/**
 * A song's comments as they read with their formatting: # headings, - and
 * * lists, **bold**, *italic*, `code`, the rest as written (trackers'
 * messages are often laid out in a fixed-width font, so it keeps that).
 */
export function Comments({ text }: { text: string }) {
  const lines = text.split(/\r?\n/);
  return (
    <div className="tracker-comments">
      {lines.map((line, i) => {
        const h = /^(#{1,3})\s+(.*)$/.exec(line);
        if (h) return <p key={i} className={`tracker-comment-h${h[1].length}`}>{inline(h[2])}</p>;
        const li = /^\s*[-*]\s+(.*)$/.exec(line);
        if (li) return <p key={i} className="tracker-comment-li">{inline(li[1])}</p>;
        return <Fragment key={i}>{line.trim() === "" ? <p className="tracker-comment-gap" /> : <p className="tracker-comment-p">{inline(line)}</p>}</Fragment>;
      })}
    </div>
  );
}
