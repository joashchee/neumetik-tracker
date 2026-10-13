/**
 * The session's wire format (session/src/lib.rs, `Session::call_wire`),
 * which the desktop and mobile apps (Tauri's raw IPC) and the web build
 * (the WebAssembly module, in an AudioWorklet) both speak.
 *
 * In: the command's name and its JSON, each after its length (u32,
 * little-endian), then the bytes it takes. Out: 0 (done) or 1 (failed),
 * the JSON (or the failure's words) after its length, then the bytes it
 * gives.
 */
const enc = new TextEncoder();
const dec = new TextDecoder();

export interface Reply<T = unknown> {
  json: T;
  bytes: Uint8Array;
}

export function request(cmd: string, args: unknown, data?: Uint8Array): Uint8Array {
  const c = enc.encode(cmd);
  const a = enc.encode(args === undefined ? "null" : JSON.stringify(args));
  const d = data ?? new Uint8Array();
  const out = new Uint8Array(8 + c.length + a.length + d.length);
  const view = new DataView(out.buffer);
  view.setUint32(0, c.length, true);
  out.set(c, 4);
  view.setUint32(4 + c.length, a.length, true);
  out.set(a, 8 + c.length);
  out.set(d, 8 + c.length + a.length);
  return out;
}

/** A reply read: its JSON and bytes, or what failed, thrown in words. */
export function reply<T>(buf: ArrayBuffer | Uint8Array): Reply<T> {
  const b = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
  if (b.length < 5) throw new Error("The tracker gave back an empty answer.");
  const n = new DataView(b.buffer, b.byteOffset, b.byteLength).getUint32(1, true);
  const json = JSON.parse(dec.decode(b.subarray(5, 5 + n)));
  if (b[0] !== 0) throw new Error(typeof json === "string" ? json : "It failed.");
  return { json: json as T, bytes: b.subarray(5 + n) };
}
