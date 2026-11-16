import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { gunzipSync } from "node:zlib";
import { digest, fingerprint } from "./provenance.js";
import { verify } from "./verify.js";

const directory = process.argv[2];
assert.ok(directory);
const result = JSON.parse(
  await readFile(join(directory, "result.json"), "utf8"),
);
assert.equal(result.status, "complete");
assert.equal(result.sourceVersion, (await fingerprint()).sourceVersion);
for (const file of result.files)
  assert.equal(
    digest(await readFile(join(directory, file.path))),
    file.sha256,
    file.path,
  );
const manifest = JSON.parse(
  await readFile(join(directory, "manifest.json"), "utf8"),
);
assert.equal(manifest.sourceVersion, result.sourceVersion);
assert.ok(
  manifest.phase === "development" || manifest.phase === "confirmation",
);
assert.equal(manifest.seed, manifest.phase === "development" ? 0x37a1 : 0x37c3);
assert.equal(manifest.count, manifest.phase === "development" ? 64 : 128);
for (const binary of manifest.binaries)
  assert.equal(digest(await readFile(binary.path)), binary.sha256, binary.path);
const previous = JSON.parse(
  gunzipSync(await readFile(join(directory, "records.json.gz"))).toString(),
);
let index = 0;
const checked = await verify(manifest.seed, manifest.count, (record) =>
  assert.deepEqual(record, previous[index++]),
);
assert.equal(index, previous.length);
for (const [key, value] of Object.entries(checked))
  assert.deepEqual(value, result[key], key);
console.log(JSON.stringify({ status: "complete", ...checked, records: index }));
