import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createCorpus } from "./corpus.js";
import { nativeReplies, wasmPath } from "./native.js";
import { enumerate, validate } from "./oracle.js";
import { createWasm } from "./wasm.js";

export const verify = async (
  seed: number,
  count: number,
  save: (record: unknown) => void,
) => {
  const wasm = await createWasm(await readFile(wasmPath));
  const inputs = createCorpus(seed, count);
  const native = nativeReplies(inputs);
  let sat = 0;
  let unsat = 0;
  for (const [index, input] of inputs.entries()) {
    const record = {
      input,
      expected: enumerate(input),
      native: native[index],
      wasm: wasm.request(input),
    };
    save(record);
    assert.deepEqual(validate(input, record.native), record.expected);
    assert.deepEqual(validate(input, record.wasm), record.expected);
    sat += record.expected.filter((status) => status === "sat").length;
    unsat += record.expected.filter((status) => status === "unsat").length;
  }
  const base = {
    variables: 1,
    clauses: [],
    rounds: [{ add: [], assumptions: [] }],
  };
  const controls = [
    { input: { ...base, maxWork: 0 }, kind: "unsupported" },
    { input: { ...base, maxWork: 100 }, kind: "unsupported" },
    { input: { ...base, clauses: [[0]] }, kind: "error" },
    { input: { ...base, clauses: [[2]] }, kind: "error" },
    { input: { ...base, variables: 9 }, kind: "error" },
    { input: { ...base, rounds: [] }, kind: "error" },
  ];
  const replies = nativeReplies(controls.map((control) => control.input));
  for (const [index, control] of controls.entries()) {
    const record = {
      control,
      native: replies[index],
      wasm: wasm.request(control.input),
    };
    save(record);
    assert.deepEqual(record.native, record.wasm);
    assert.ok(typeof record.wasm === "object" && record.wasm !== null);
    assert.equal(Reflect.get(record.wasm, "kind"), control.kind);
    if (control.kind === "unsupported")
      assert.deepEqual(record.wasm, {
        kind: "unsupported",
        reason: "work-budget",
        candidate: "varisat-0.2.2",
      });
  }
  assert.ok(sat > 0 && unsat > 0);
  return {
    inputs: inputs.length,
    rounds: sat + unsat,
    sat,
    unsat,
    controls: controls.length,
    imports: wasm.imports,
  };
};
