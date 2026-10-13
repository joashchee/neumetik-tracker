// Copyright 2026 ansiapps. Neumetik Tracker: MIT licensed, see the LICENSE at the repository root.
//
// A second tracker, with no sound, for the work that plays a whole song
// through (a WAV, each channel's, the song's length): the page hands it
// a copy of the song, so the one playing never stops for it.

let module = null;
let x = null;
let broken = null;

function start() {
  x = new WebAssembly.Instance(module, {}).exports;
  x.nt_init(0);
}

onmessage = (e) => {
  if (e.data.init) {
    try {
      module = new WebAssembly.Module(e.data.init);
      start();
    } catch (err) {
      broken = `The tracker couldn't start here. (${err})`;
    }
    return;
  }
  const { id, msg } = e.data;
  if (broken || !x) return postMessage({ id, error: broken ?? "The tracker hasn't started." });
  try {
    const p = x.nt_alloc(msg.length);
    new Uint8Array(x.memory.buffer, p, msg.length).set(msg);
    const out = x.nt_call(p, msg.length);
    const reply = new Uint8Array(x.memory.buffer, out, x.nt_reply_len()).slice();
    postMessage({ id, reply: reply.buffer }, [reply.buffer]);
  } catch (err) {
    // Started again from scratch, so the next export works.
    start();
    postMessage({ id, error: `The export stopped after a fault. (${err})` });
  }
};
