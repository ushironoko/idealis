import { execFileSync } from "node:child_process";
export const nativePath =
  "evals/sat-foundation/rust/target/release/idealis-sat-eval";
export const wasmPath =
  "evals/sat-foundation/rust/target/wasm32-unknown-unknown/release/idealis_sat_eval.wasm";
export const nativeReplies = (
  inputs: readonly unknown[],
): readonly unknown[] => {
  const output = execFileSync(nativePath, [], {
    input: `${inputs.map((input) => JSON.stringify(input)).join("\n")}\n`,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  })
    .trimEnd()
    .split("\n");
  if (output.length !== inputs.length)
    throw new Error("Missing native SAT replies");
  return output.map((line): unknown => JSON.parse(line));
};
