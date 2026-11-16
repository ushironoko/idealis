import { createCorpus } from "./corpus.js";
import { enumerate, validate } from "./oracle.js";
import { createWasm } from "./wasm.js";

export const run = async () => {
  const bytes = new Uint8Array(await (await fetch("/sat.wasm")).arrayBuffer());
  const wasm = await createWasm(bytes);
  const records = createCorpus(0x3701, 32).map((input) => {
    const reply = wasm.request(input);
    return {
      input,
      reply,
      expected: enumerate(input),
      statuses: validate(input, reply),
    };
  });
  return {
    records,
    imports: wasm.imports,
    bytes: bytes.length,
    isolated: crossOriginIsolated,
    budget: wasm.request({
      variables: 1,
      clauses: [],
      rounds: [{ add: [], assumptions: [] }],
      maxWork: 0,
    }),
  };
};
