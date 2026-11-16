import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createCorpus, fixed } from "./corpus.js";
import { nativeReplies, wasmPath } from "./native.js";
import { enumerate, validate } from "./oracle.js";
import { createWasm } from "./wasm.js";

test("native and import-free Wasm agree with complete enumeration on fixed CNFs", async () => {
  const wasm = await createWasm(await readFile(wasmPath));
  assert.deepEqual(wasm.imports, []);
  const native = nativeReplies(fixed);
  for (const [index, input] of fixed.entries()) {
    const expected = enumerate(input);
    assert.deepEqual(validate(input, native[index]), expected);
    assert.deepEqual(validate(input, wasm.request(input)), expected);
  }
  assert.deepEqual(fixed.map(enumerate).slice(0, 4), [
    ["sat"],
    ["unsat"],
    ["sat"],
    ["sat", "unsat", "sat", "unsat"],
  ]);
});

test("budget requests and invalid literals stay separate from SAT/UNSAT", async () => {
  const wasm = await createWasm(await readFile(wasmPath));
  const base = {
    variables: 1,
    clauses: [],
    rounds: [{ add: [], assumptions: [] }],
  };
  const inputs = [
    { ...base, maxWork: 0 },
    { ...base, maxWork: 100 },
    { ...base, clauses: [[0]] },
    { ...base, clauses: [[2]] },
    { ...base, rounds: [{ add: [], assumptions: [-2] }] },
  ];
  const native = nativeReplies(inputs);
  for (const [index, input] of inputs.entries()) {
    const reply = wasm.request(input);
    assert.deepEqual(reply, native[index]);
    assert.ok(typeof reply === "object" && reply !== null);
    assert.equal(
      Reflect.get(reply, "kind"),
      index < 2 ? "unsupported" : "error",
    );
  }
});

test("the independent validator rejects false UNSAT and a bad SAT witness", () => {
  const input = {
    variables: 1,
    clauses: [[1]],
    rounds: [{ add: [], assumptions: [] }],
  };
  assert.throws(() =>
    validate(input, {
      kind: "results",
      selfChecking: true,
      workBudget: false,
      results: [{ status: "unsat", model: null }],
    }),
  );
  assert.throws(() =>
    validate(input, {
      kind: "results",
      selfChecking: true,
      workBudget: false,
      results: [{ status: "sat", model: [false] }],
    }),
  );
  const corpus = createCorpus(0x3701, 32);
  assert.equal(corpus.length, 32);
  assert.deepEqual(corpus, createCorpus(0x3701, 32));
  assert.deepEqual(
    [...new Set(corpus.map((input) => input.variables))].sort(),
    [0, 1, 2, 3, 4, 5, 6, 7, 8],
  );
});
