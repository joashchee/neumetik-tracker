// Copyright 2026 ansiapps. Neumetik Tracker: MIT licensed, see the LICENSE at the repository root.
//
// The tracker where the browser makes sound: the WebAssembly module
// (web/src/lib.rs) runs here, the song's session and its sound in one,
// so every edit is heard as it's made. Commands come through the port in
// the session's wire format (src/lib/wire.ts) and go back the same way;
// there's no text decoder in here, and none is needed.

class TrackerProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.broken = null;
    try {
      this.x = new WebAssembly.Instance(new WebAssembly.Module(options.processorOptions.wasm), {}).exports;
      this.x.nt_init(sampleRate);
    } catch (e) {
      this.broken = `The tracker couldn't start here. (${e})`;
    }
    this.port.onmessage = (e) => this.command(e.data);
  }

  command({ id, msg }) {
    if (this.broken) return this.port.postMessage({ id, error: this.broken });
    try {
      const x = this.x;
      const p = x.nt_alloc(msg.length);
      new Uint8Array(x.memory.buffer, p, msg.length).set(msg);
      const out = x.nt_call(p, msg.length);
      const reply = new Uint8Array(x.memory.buffer, out, x.nt_reply_len()).slice();
      this.port.postMessage({ id, reply: reply.buffer }, [reply.buffer]);
    } catch (e) {
      // A fault in the tracker leaves it unsafe to go on: it says so from here on.
      this.broken = `The tracker stopped after a fault; reload the page to start it again. (${e})`;
      this.port.postMessage({ id, error: this.broken });
    }
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    if (this.broken || !out || out.length === 0) return true;
    const n = out[0].length;
    try {
      const p = this.x.nt_render(n);
      const both = new Float32Array(this.x.memory.buffer, p, n * 2);
      out[0].set(both.subarray(0, n));
      if (out[1]) out[1].set(both.subarray(n, n * 2));
    } catch (e) {
      this.broken = `The tracker stopped after a fault; reload the page to start it again. (${e})`;
    }
    return true;
  }
}

registerProcessor("neumetik-tracker", TrackerProcessor);
