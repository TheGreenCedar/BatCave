import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { test } from "node:test";
import postcss from "postcss";

test("CSS annotations cannot load source maps outside a known source directory", async () => {
  const root = await mkdtemp(join(tmpdir(), "batcave-postcss-"));
  try {
    const sourceDirectory = join(root, "styles");
    await mkdir(sourceDirectory);
    const secret = join(root, "private.map");
    const map = JSON.stringify({
      version: 3,
      sources: ["private.css"],
      sourcesContent: ["PRIVATE_SOURCE_SENTINEL"],
      names: [],
      mappings: "AAAA",
    });
    await writeFile(secret, map);
    const from = join(sourceDirectory, "input.css");
    for (const [source, annotation] of [
      [undefined, secret],
      [undefined, relative(process.cwd(), secret)],
      [from, "../private.map"],
    ]) {
      const parsed = postcss.parse(`a { color: red }\n/*# sourceMappingURL=${annotation} */`, {
        from: source,
      });
      assert.equal(parsed.source.input.map, undefined, `untrusted map loaded: ${annotation}`);
    }

    // Ordinary build source maps remain available next to their known CSS file.
    await writeFile(join(sourceDirectory, "input.css.map"), map);
    const parsed = postcss.parse("a { color: red }\n/*# sourceMappingURL=input.css.map */", {
      from,
    });
    assert.deepEqual(parsed.source.input.map.consumer().sourcesContent, [
      "PRIVATE_SOURCE_SENTINEL",
    ]);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
