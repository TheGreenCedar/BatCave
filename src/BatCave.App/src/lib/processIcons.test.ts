import assert from "node:assert/strict";
import test from "node:test";
import {
  buildResolvedProcessIconCatalog,
  processIconFamily,
  processIconKey,
  type ProcessIconCandidate,
} from "./processIcons.ts";

const nativeCodeIcon = "data:image/png;base64,code";

function process(name: string, exe = name): ProcessIconCandidate {
  return { name, exe };
}

test("Code.exe donates to terminal helper roles without creating an inference chain", () => {
  const donor = process("Code.exe", "C:\\Program Files\\Microsoft VS Code\\Code.exe");
  const renderer = process("Code Helper (Renderer)", "C:\\Code Helper (Renderer).exe");
  const utility = process("Code Utility", "C:\\Code Utility.exe");
  const catalog = buildResolvedProcessIconCatalog([donor, renderer, utility], {
    [processIconKey(donor)]: nativeCodeIcon,
  });

  assert.deepEqual(catalog[processIconKey(donor)], { src: nativeCodeIcon, origin: "native" });
  assert.deepEqual(catalog[processIconKey(renderer)], {
    src: nativeCodeIcon,
    origin: "name_match",
  });
  assert.deepEqual(catalog[processIconKey(utility)], {
    src: nativeCodeIcon,
    origin: "name_match",
  });
});

test("numeric process instances share an exact normalized family", () => {
  const donor = process("SearchIndexer-211.exe");
  const target = process("SearchIndexer-223.exe");
  const catalog = buildResolvedProcessIconCatalog([donor, target], {
    [processIconKey(donor)]: "search-indexer-icon",
  });

  assert.equal(processIconFamily(donor.name), "searchindexer");
  assert.equal(processIconFamily(target.name), "searchindexer");
  assert.equal(catalog[processIconKey(target)].origin, "name_match");
});

test("near spellings never match", () => {
  const donor = process("SearchIndexer.exe");
  const target = process("SearchIndexor.exe");
  const catalog = buildResolvedProcessIconCatalog([donor, target], {
    [processIconKey(donor)]: "search-indexer-icon",
  });

  assert.deepEqual(catalog[processIconKey(target)], { origin: "fallback" });
});

test("generic or too-short names cannot donate or receive", () => {
  const helper = process("Helper.exe");
  const renderer = process("Renderer-211.exe");
  const short = process("GPU.exe");
  const gpuProcess = process("GPU Process.exe");
  const catalog = buildResolvedProcessIconCatalog([helper, renderer, short, gpuProcess], {
    [processIconKey(helper)]: "generic-icon",
  });

  assert.equal(processIconFamily(helper.name), null);
  assert.equal(processIconFamily(renderer.name), null);
  assert.equal(processIconFamily(short.name), null);
  assert.equal(processIconFamily(gpuProcess.name), null);
  assert.deepEqual(catalog[processIconKey(renderer)], { origin: "fallback" });
});

test("conflicting direct donor images make an iconless family ambiguous", () => {
  const first = process("SearchIndexer-211.exe");
  const second = process("SearchIndexer-223.exe");
  const target = process("SearchIndexer-244.exe");
  const catalog = buildResolvedProcessIconCatalog([first, second, target], {
    [processIconKey(first)]: "first-icon",
    [processIconKey(second)]: "second-icon",
  });

  assert.equal(catalog[processIconKey(first)].origin, "native");
  assert.equal(catalog[processIconKey(second)].origin, "native");
  assert.deepEqual(catalog[processIconKey(target)], { origin: "fallback" });
});

test("a newly available direct icon replaces an inferred icon and removes provenance", () => {
  const donor = process("Code.exe");
  const target = process("Code Helper (Renderer)");
  const inferred = buildResolvedProcessIconCatalog([donor, target], {
    [processIconKey(donor)]: nativeCodeIcon,
  });
  const direct = buildResolvedProcessIconCatalog([donor, target], {
    [processIconKey(donor)]: nativeCodeIcon,
    [processIconKey(target)]: "renderer-icon",
  });

  assert.equal(inferred[processIconKey(target)].origin, "name_match");
  assert.deepEqual(direct[processIconKey(target)], {
    src: "renderer-icon",
    origin: "native",
  });
});

test("removing the direct donor from the current result restores fallback", () => {
  const donor = process("Code.exe");
  const target = process("Code Helper (Renderer)");
  const nativeIcons = { [processIconKey(donor)]: nativeCodeIcon };

  assert.equal(
    buildResolvedProcessIconCatalog([donor, target], nativeIcons)[processIconKey(target)].origin,
    "name_match",
  );
  assert.deepEqual(buildResolvedProcessIconCatalog([target], nativeIcons)[processIconKey(target)], {
    origin: "fallback",
  });
});

test("normalization folds Unicode compatibility and case without fuzzy matching", () => {
  assert.equal(processIconFamily("ＣＯＤＥ Helper"), "code");
  assert.equal(processIconFamily("code utility.exe"), "code");
});
