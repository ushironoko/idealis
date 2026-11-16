import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";

export const digest = (value: string | Uint8Array): string =>
  createHash("sha256").update(value).digest("hex");
export const fingerprint = async () => {
  const paths = execFileSync("git", [
    "ls-files",
    "-z",
    "--cached",
    "--others",
    "--exclude-standard",
    "--",
    "evals/sat-foundation",
    "evals/plans/0037-sat-foundation.md",
    "evals/plans/0037-sat-protocol.md",
    "sat-playwright.config.ts",
    "justfile",
    ".github/workflows/ci.yml",
    "package.json",
    "pnpm-lock.yaml",
    "tsconfig.json",
    "tsconfig.build.json",
    "biome.json",
    ".gitignore",
  ])
    .toString()
    .split("\0")
    .filter((path) => path && !path.endsWith("README.md"));
  const sources = await Promise.all(
    [...new Set(paths)]
      .sort()
      .map(async (path) => ({ path, sha256: digest(await readFile(path)) })),
  );
  return { sources, sourceVersion: digest(JSON.stringify(sources)) };
};
