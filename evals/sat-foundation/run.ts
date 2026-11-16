import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { appendFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { basename, dirname, join } from "node:path";
import { performance } from "node:perf_hooks";
import { gzipSync } from "node:zlib";
import { nativePath, wasmPath } from "./native.js";
import { digest, fingerprint } from "./provenance.js";
import { verify } from "./verify.js";

const phase = process.argv[2];
const directory = process.argv[3];
const parent = process.argv[4];
assert.ok((phase === "development" || phase === "confirmation") && directory);
await mkdir(dirname(directory), { recursive: true });
await mkdir(directory);
const stamp = await fingerprint();
if (phase === "confirmation") {
  assert.ok(parent);
  const previous = JSON.parse(
    await readFile(join(parent, "result.json"), "utf8"),
  );
  const manifest = JSON.parse(
    await readFile(join(parent, "manifest.json"), "utf8"),
  );
  assert.equal(previous.status, "complete");
  assert.equal(manifest.phase, "development");
  assert.equal(previous.sourceVersion, stamp.sourceVersion);
}
const seed = phase === "development" ? 0x37a1 : 0x37c3;
const count = phase === "development" ? 64 : 128;
const binaries = await Promise.all(
  [nativePath, wasmPath].map(async (path) => {
    const bytes = await readFile(path);
    return { path, sha256: digest(bytes), bytes: bytes.length };
  }),
);
await writeFile(
  join(directory, "manifest.json"),
  JSON.stringify(
    {
      ...stamp,
      phase,
      seed,
      count,
      parent,
      binaries,
      commit: execFileSync("git", ["rev-parse", "HEAD"]).toString().trim(),
      dirtyPaths: execFileSync("git", ["status", "--short"]).toString().trim(),
      environment: {
        node: process.version,
        platform: process.platform,
        arch: process.arch,
        rustc: execFileSync("rustc", ["-Vv"]).toString().trim(),
      },
      comparison:
        "native/Wasm statuses vs independent complete enumeration, both concrete models validated, self-checking enabled",
    },
    null,
    2,
  ),
  { flag: "wx" },
);
const records: unknown[] = [];
const start = performance.now();
try {
  const checked = await verify(seed, count, (record) => records.push(record));
  assert.equal((await fingerprint()).sourceVersion, stamp.sourceVersion);
  await writeFile(
    join(directory, "records.json.gz"),
    gzipSync(JSON.stringify(records)),
    { flag: "wx" },
  );
  const files = await Promise.all(
    ["manifest.json", "records.json.gz"].map(async (path) => ({
      path,
      sha256: digest(await readFile(join(directory, path))),
    })),
  );
  const result = {
    status: "complete",
    sourceVersion: stamp.sourceVersion,
    ...checked,
    elapsedMs: performance.now() - start,
    files,
  };
  await writeFile(
    join(directory, "result.json"),
    JSON.stringify(result, null, 2),
    { flag: "wx" },
  );
  await appendFile(
    "evals/runs/ledger.jsonl",
    `${JSON.stringify({ runId: basename(directory), kind: "sat-foundation", status: "complete", sourceVersion: stamp.sourceVersion })}\n`,
  );
  console.log(JSON.stringify(result));
} catch (error) {
  await writeFile(
    join(directory, "error.json"),
    JSON.stringify(
      { status: "error", message: String(error), records },
      null,
      2,
    ),
    { flag: "wx" },
  );
  await appendFile(
    "evals/runs/ledger.jsonl",
    `${JSON.stringify({ runId: basename(directory), kind: "sat-foundation", status: "error", sourceVersion: stamp.sourceVersion })}\n`,
  );
  throw error;
}
