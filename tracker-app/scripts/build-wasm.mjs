// Builds the web build's WebAssembly (web/) and puts it where the page
// loads it from: public/tracker.wasm.
import { execFileSync } from "node:child_process";
import { homedir } from "node:os";
import { copyFileSync, mkdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const target = process.env.CARGO_TARGET_DIR ?? join(root, "target");
// Without the build machine's paths in it (rustc bakes them into panic messages).
const remap = [`--remap-path-prefix=${homedir()}=~`, `--remap-path-prefix=${join(root, "..")}=.`];
if (process.env.CARGO_HOME) remap.push(`--remap-path-prefix=${process.env.CARGO_HOME}=~/.cargo`);
const env = { ...process.env, RUSTFLAGS: `${process.env.RUSTFLAGS ?? ""} ${remap.join(" ")}`.trim() };
execFileSync("cargo", ["build", "--release", "--target", "wasm32-unknown-unknown", "-p", "neumetik-tracker-web"], { cwd: root, stdio: "inherit", env });
mkdirSync(join(root, "public"), { recursive: true });
copyFileSync(join(target, "wasm32-unknown-unknown", "release", "neumetik_tracker_web.wasm"), join(root, "public", "tracker.wasm"));
const wasm = readFileSync(join(root, "public", "tracker.wasm"));
if (wasm.includes(Buffer.from(homedir()))) throw new Error(`public/tracker.wasm still has ${homedir()} in it.`);
console.log("public/tracker.wasm made.");
