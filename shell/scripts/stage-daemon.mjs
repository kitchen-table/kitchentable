#!/usr/bin/env node
/**
 * Build kt-daemon and put it where the app bundle will pick it up.
 *
 * Kitchen Table is two processes and one product. The daemon is what serves,
 * and it has to be inside the .app, because "install the app" is the only step
 * anybody is going to take - a bundle without it is a window that opens onto a
 * permanent "not running" screen on any machine but a developer's.
 *
 * Tauri calls a binary staged like this a sidecar: it looks for
 * `binaries/kt-daemon-<target triple>` and copies it into `Contents/MacOS/`,
 * dropping the triple. That is exactly where `daemon.rs` already looks for it,
 * so nothing on the Rust side changes.
 *
 * Run automatically by `beforeBuildCommand`, so a bundle cannot be built
 * without a daemon in it. Not by `tauri dev`, which finds the daemon in the
 * workspace target directory instead.
 */
import { execFileSync } from "node:child_process";
import { mkdirSync, copyFileSync, chmodSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const workspace = resolve(here, "..", "..");
const staging = resolve(here, "..", "src-tauri", "binaries");

/**
 * The triple Tauri is building for.
 *
 * `TAURI_ENV_TARGET_TRIPLE` is set when this runs as `beforeBuildCommand`. Run
 * by hand it falls back to the host's, which is what somebody debugging this
 * script wants.
 */
function targetTriple() {
  if (process.env.TAURI_ENV_TARGET_TRIPLE) return process.env.TAURI_ENV_TARGET_TRIPLE;

  const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" })
    .split("\n")
    .find((line) => line.startsWith("host: "));
  if (!host) throw new Error("could not read the host target from rustc -vV");
  return host.slice("host: ".length).trim();
}

/**
 * Debug or release, matching the app being built around it.
 *
 * A release app carrying a debug daemon would ship the slow one and, worse,
 * would mean the thing tested was not the thing shipped.
 */
const debug = process.env.TAURI_ENV_DEBUG === "true";

function build(triple) {
  const args = ["build", "-p", "kt-daemon", "--target", triple];
  if (!debug) args.push("--release");

  console.log(`staging kt-daemon for ${triple} (${debug ? "debug" : "release"})`);
  execFileSync("cargo", args, { cwd: workspace, stdio: "inherit" });

  return join(workspace, "target", triple, debug ? "debug" : "release", "kt-daemon");
}

/**
 * A universal build needs a universal daemon.
 *
 * Tauri builds the shell for both architectures and lipos them; the sidecar is
 * handed over as a finished file, so this has to do the same for the daemon.
 * An arm64-only daemon inside a universal app is an Intel Mac that installs
 * cleanly and then cannot serve anything.
 */
function stage() {
  mkdirSync(staging, { recursive: true });
  const triple = targetTriple();
  const destination = join(staging, `kt-daemon-${triple}`);

  if (triple === "universal-apple-darwin") {
    const parts = ["aarch64-apple-darwin", "x86_64-apple-darwin"].map(build);
    execFileSync("lipo", ["-create", "-output", destination, ...parts], {
      stdio: "inherit",
    });
  } else {
    copyFileSync(build(triple), destination);
  }

  // `copyFileSync` keeps the mode, but `lipo` writes a fresh file; be explicit
  // either way, because a sidecar that is not executable fails at spawn time in
  // the built app rather than here.
  chmodSync(destination, 0o755);
  console.log(`staged ${destination}`);
}

stage();
