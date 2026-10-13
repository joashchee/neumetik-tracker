/**
 * Where the app runs, and what that platform does for it: the desktop
 * and phones through Tauri (src-tauri), or a web browser, where the
 * tracker is WebAssembly (web/) playing in an AudioWorklet
 * (public/tracker-worklet.js), and songs are exported in a Worker
 * (public/tracker-worker.js) so the sound never stops for it.
 *
 * The screen only ever asks `host`: run a command, pick files to open,
 * save bytes as a file.
 */
import * as wire from "./wire";

export interface Picked {
  name: string;
  /** Where it is, on the desktop. */
  path: string | null;
  bytes: () => Promise<Uint8Array>;
}

export interface Filter {
  name: string;
  extensions: string[];
}

export interface Host {
  kind: "tauri" | "web";
  /** A phone or tablet: no computer keyboard to count on. */
  mobile: boolean;
  /** Why there's no sound, when there isn't. */
  noSound: string | null;
  call<T = unknown>(cmd: string, args?: unknown, data?: Uint8Array): Promise<wire.Reply<T>>;
  /** A module opened by its path, the files beside it found too (the desktop only). */
  openPath: ((path: string) => Promise<wire.Reply<unknown>>) | null;
  pick(filters: Filter[], multiple: boolean): Promise<Picked[]>;
  /** Saves `bytes` where the user says, named `name` to start with; the name it's saved as, or null if they didn't. */
  save(name: string, bytes: Uint8Array, filter: Filter): Promise<string | null>;
  /** Sound may need a tap or a key first (a browser's rule): this starts it. */
  wake(): void;
}

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

async function tauriHost(): Promise<Host> {
  const { invoke } = await import("@tauri-apps/api/core");
  const dialog = await import("@tauri-apps/plugin-dialog");
  const fs = await import("@tauri-apps/plugin-fs");
  const p = await invoke<{ os: string; mobile: boolean; noSound: string | null }>("platform");
  const call = async <T,>(cmd: string, args?: unknown, data?: Uint8Array) => wire.reply<T>(await invoke<ArrayBuffer>("call", wire.request(cmd, args, data)));
  return {
    kind: "tauri",
    mobile: p.mobile,
    noSound: p.noSound,
    call,
    openPath: p.mobile ? null : async (path) => wire.reply(await invoke<ArrayBuffer>("open_path", { path })),
    async pick(filters, multiple) {
      const picked = await dialog.open({ multiple, filters: p.mobile ? undefined : [...filters, { name: "All files", extensions: ["*"] }] });
      const paths = picked === null ? [] : Array.isArray(picked) ? picked : [picked];
      return paths.map((path) => ({ name: baseName(decodeURIComponent(path)), path: p.mobile ? null : path, bytes: () => fs.readFile(path) }));
    },
    async save(name, bytes, filter) {
      const to = await dialog.save({ defaultPath: name, filters: [filter] });
      if (!to) return null;
      await fs.writeFile(to, bytes);
      return baseName(decodeURIComponent(to));
    },
    wake() {},
  };
}

/** The tracker in an AudioWorklet, spoken to through its port. */
class Port {
  private next = 1;
  private waiting = new Map<number, (r: { reply?: ArrayBuffer; error?: string }) => void>();
  constructor(private port: MessagePort | Worker) {
    port.onmessage = (e: MessageEvent<{ id: number; reply?: ArrayBuffer; error?: string }>) => {
      const w = this.waiting.get(e.data.id);
      this.waiting.delete(e.data.id);
      w?.(e.data);
    };
  }
  send(msg: Uint8Array): Promise<ArrayBuffer> {
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.waiting.set(id, (r) => (r.reply ? resolve(r.reply) : reject(new Error(r.error ?? "The tracker stopped."))));
      this.port.postMessage({ id, msg }, [msg.buffer]);
    });
  }
}

/** The commands that play the whole song through: done in the Worker, from a copy of the song. */
const HEAVY = new Set(["exportWav", "exportStems", "stats"]);

async function webHost(): Promise<Host> {
  const base = import.meta.env.BASE_URL;
  const wasm = await (await fetch(`${base}tracker.wasm`)).arrayBuffer();
  const ctx = new AudioContext({ latencyHint: "interactive" });
  await ctx.audioWorklet.addModule(`${base}tracker-worklet.js`);
  const node = new AudioWorkletNode(ctx, "neumetik-tracker", { numberOfInputs: 0, numberOfOutputs: 1, outputChannelCount: [2], processorOptions: { wasm } });
  node.connect(ctx.destination);
  const sound = new Port(node.port);
  const worker = new Worker(`${base}tracker-worker.js`);
  worker.postMessage({ init: wasm });
  const work = new Port(worker);
  const call = async <T,>(cmd: string, args?: unknown, data?: Uint8Array): Promise<wire.Reply<T>> => {
    if (!HEAVY.has(cmd)) return wire.reply<T>(await sound.send(wire.request(cmd, args, data)));
    const song = wire.reply<string>(await sound.send(wire.request("snapshot", null)));
    wire.reply(await work.send(wire.request("open", { name: "song.ntt" }, song.bytes.slice())));
    return wire.reply<T>(await work.send(wire.request(cmd, args, data)));
  };
  return {
    kind: "web",
    mobile: matchMedia("(pointer: coarse)").matches,
    noSound: null,
    call,
    openPath: null,
    pick(filters, multiple) {
      return new Promise((resolve) => {
        const input = document.createElement("input");
        input.type = "file";
        input.multiple = multiple;
        input.accept = filters.flatMap((f) => f.extensions.filter((e) => e !== "*").map((e) => `.${e}`)).join(",");
        // Tracker modules have names a browser doesn't know (MOD., .669): all files may be chosen.
        if (filters.some((f) => f.extensions.length > 40)) input.accept = "";
        input.onchange = () => resolve(Array.from(input.files ?? []).map((f) => ({ name: f.name, path: null, bytes: async () => new Uint8Array(await f.arrayBuffer()) })));
        input.oncancel = () => resolve([]);
        input.click();
      });
    },
    async save(name, bytes, filter) {
      const url = URL.createObjectURL(new Blob([bytes.slice()], { type: filter.extensions[0] === "wav" ? "audio/wav" : "application/octet-stream" }));
      const a = document.createElement("a");
      a.href = url;
      a.download = name;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 60_000);
      return name;
    },
    wake() {
      if (ctx.state !== "running") void ctx.resume();
    },
  };
}

let made: Promise<Host> | null = null;

/** The platform, made ready once. */
export function host(): Promise<Host> {
  made ??= isTauri ? tauriHost() : webHost();
  return made;
}
