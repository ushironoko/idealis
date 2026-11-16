import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { digest } from "./provenance.js";

const directory = process.argv[2];
assert.ok(directory);
const metadata = JSON.parse(
  execFileSync("cargo", [
    "metadata",
    "--locked",
    "--format-version",
    "1",
    "--manifest-path",
    "evals/sat-foundation/rust/Cargo.toml",
  ]).toString(),
);
const packages: {
  name: string;
  version: string;
  manifest_path: string;
  license: string;
}[] = metadata.packages;
const candidate = packages.find((entry) => entry.name === "varisat");
assert.equal(candidate?.version, "0.2.2");
assert.ok(candidate);
const root = dirname(candidate.manifest_path);
const vcs = JSON.parse(
  await readFile(join(root, ".cargo_vcs_info.json"), "utf8"),
);
const files = await Promise.all(
  ["src/solver.rs", "src/config.rs", "src/schedule.rs"].map(async (path) => {
    const source = await readFile(join(root, path), "utf8");
    return {
      path,
      sha256: digest(source),
      url: `https://github.com/jix/varisat/blob/${vcs.git.sha1}/varisat/${path}`,
      publicMethods: [...source.matchAll(/pub fn ([^\n{]+)/g)].map(
        (match) => match[1],
      ),
    };
  }),
);
await writeFile(
  join(directory, "source-review.json"),
  JSON.stringify(
    {
      candidate: `${candidate.name}-${candidate.version}`,
      license: candidate.license,
      upstreamCommit: vcs.git.sha1,
      files,
      dependencies: packages
        .map((entry) => ({
          name: entry.name,
          version: entry.version,
          license: entry.license,
        }))
        .sort((left, right) => left.name.localeCompare(right.name)),
      observation: {
        solve:
          "Solver::solve loops over private schedule_step until completion or internal error",
        assumptions:
          "Solver::assume replaces previous assumptions. Added clauses persist across solves",
        selfChecking:
          "enable_self_checking called before clauses, enabled in every evaluation. No cost comparison with checking disabled",
        budget:
          "No direct work/conflict limit or interrupt/resume control found in public Solver methods or SolverConfig. Interrupted is recoverable, but its presence does not supply a public trigger",
        proofProcessor:
          "Public proof processor callbacks can fail. A budget/cancel adapter using them is not implemented or evaluated",
        decision:
          "Retain isolated CNF candidate. Hold adoption as a budgeted backend. maxWork requests return unsupported before solve",
      },
    },
    null,
    2,
  ),
  { flag: "wx" },
);
