import { readFile, writeFile } from "node:fs/promises";
import { expect, test } from "@playwright/test";
import { build } from "esbuild";
import type { run } from "./browser-entry.js";
import { nativeReplies, wasmPath } from "./native.js";
import { validate } from "./oracle.js";

test("Varisat solves CNF in Chromium without host imports or isolation", async ({
  page,
}, info) => {
  const bundle = await build({
    entryPoints: ["evals/sat-foundation/browser-entry.ts"],
    bundle: true,
    platform: "browser",
    format: "iife",
    globalName: "satEval",
    write: false,
  });
  const javascript = bundle.outputFiles[0];
  if (!javascript) throw new Error("Missing SAT browser bundle");
  const wasm = await readFile(wasmPath);
  await page.route("http://sat.test/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    await route.fulfill(
      path === "/bundle.js"
        ? { contentType: "application/javascript", body: javascript.text }
        : path === "/sat.wasm"
          ? { contentType: "application/wasm", body: wasm }
          : {
              contentType: "text/html",
              body: '<!doctype html><script src="/bundle.js"></script>',
            },
    );
  });
  await page.goto("http://sat.test/");
  const result: Awaited<ReturnType<typeof run>> = await page.evaluate(
    async () => {
      const api = Reflect.get(globalThis, "satEval");
      if (typeof api?.run !== "function")
        throw new Error("Missing SAT browser API");
      return api.run();
    },
  );
  expect(result.records).toHaveLength(32);
  expect(result.imports).toEqual([]);
  expect(result.isolated).toBe(false);
  expect(result.budget).toEqual({
    kind: "unsupported",
    reason: "work-budget",
    candidate: "varisat-0.2.2",
  });
  const native = nativeReplies(result.records.map((record) => record.input));
  for (const [index, record] of result.records.entries()) {
    expect(record.statuses).toEqual(record.expected);
    expect(validate(record.input, native[index])).toEqual(record.expected);
  }
  const path = info.outputPath("records.json");
  await writeFile(path, JSON.stringify({ ...result, native }));
  await info.attach("sat-browser-records", {
    path,
    contentType: "application/json",
  });
});
