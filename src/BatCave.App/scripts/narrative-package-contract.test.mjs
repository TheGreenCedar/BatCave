import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

const tauriRoot = new URL("../src-tauri/", import.meta.url);
const repoRoot = new URL("../../../", tauriRoot);

async function tauriText(path) {
  return readFile(new URL(path, tauriRoot), "utf8");
}

test("Foundry runtime and model stay exact and explicitly staged", async () => {
  const cargo = await tauriText("Cargo.toml");
  const build = await tauriText("build.rs");
  const model = JSON.parse(await tauriText("resources/narratives/foundry-model.v1.json"));
  assert.match(
    cargo,
    /\[target\.'cfg\(any\(target_os = "windows", target_os = "linux"\)\)'\.dependencies\][\s\S]*foundry-local-sdk = "=1\.2\.0"/u,
  );
  assert.match(
    cargo,
    /\[target\.'cfg\(any\(target_os = "windows", target_os = "linux"\)\)'\.build-dependencies\][\s\S]*foundry-local-sdk = "=1\.2\.0"/u,
  );
  assert.match(build, /fn stage_foundry_native_libraries/u);
  for (const library of [
    "Microsoft.AI.Foundry.Local.Core.dll",
    "onnxruntime.dll",
    "onnxruntime-genai.dll",
    "Microsoft.AI.Foundry.Local.Core.so",
    "libonnxruntime.so",
    "libonnxruntime-genai.so",
  ]) {
    assert.match(build, new RegExp(library.replaceAll(".", String.raw`\.`), "u"));
  }
  assert.deepEqual(model, {
    schema_version: "batcave_foundry_model_v1",
    sdk_crate: "foundry-local-sdk",
    sdk_version: "1.2.0",
    alias: "qwen2.5-0.5b",
    model_id: "qwen2.5-0.5b-instruct-generic-cpu:4",
    model_name: "qwen2.5-0.5b-instruct-generic-cpu",
    model_version: 4,
    publisher: "Microsoft",
    runtime: "CPUExecutionProvider",
    download_size_mb: 822,
    license: "Apache-2.0",
    license_url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/blob/main/LICENSE",
    catalog_uri:
      "azureml://registries/azureml/models/qwen2.5-0.5b-instruct-generic-cpu/versions/4",
  });
});

test("all platform bundles carry bounded narrative notices and payloads", async () => {
  const base = JSON.parse(await tauriText("tauri.conf.json"));
  const windows = JSON.parse(await tauriText("tauri.windows.conf.json"));
  const linux = JSON.parse(await tauriText("tauri.linux.conf.json"));
  const macos = JSON.parse(await tauriText("tauri.macos.conf.json"));
  const notices = await readFile(new URL("THIRD_PARTY_NOTICES.md", repoRoot), "utf8");

  assert.deepEqual(base.bundle.resources, {
    "../../../THIRD_PARTY_NOTICES.md": "THIRD_PARTY_NOTICES.md",
    "resources/narratives/foundry-model.v1.json": "narratives/foundry-model.v1.json",
  });
  assert.deepEqual(windows.bundle.resources, {
    ".generated/foundry-native/*.dll": "foundry-native/",
  });
  assert.deepEqual(linux.bundle.resources, {
    ".generated/foundry-native/*.so": "foundry-native/",
  });
  assert.deepEqual(macos.bundle.externalBin, [
    "target/foundation-models-sidecar/batcave-foundation-models",
  ]);
  assert.match(notices, /foundry-local-sdk` 1\.2\.0/u);
  assert.match(notices, /qwen2\.5-0\.5b-instruct-generic-cpu:4/u);
  assert.match(notices, /822 MB/u);
  assert.match(notices, /Apache License 2\.0/u);
});
