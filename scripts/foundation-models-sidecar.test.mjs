import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TAURI = path.join(ROOT, "src", "BatCave.App", "src-tauri");
const swift = fs.readFileSync(
  path.join(TAURI, "swift", "foundation-models-sidecar", "SidecarProtocol.swift"),
  "utf8",
);
const build = fs.readFileSync(path.join(TAURI, "build.rs"), "utf8");
const macosConfig = JSON.parse(fs.readFileSync(path.join(TAURI, "tauri.macos.conf.json"), "utf8"));
const verifier = fs.readFileSync(path.join(ROOT, "scripts", "verify-macos-bundle.sh"), "utf8");
const nativeTest = fs.readFileSync(
  path.join(ROOT, "scripts", "test-foundation-models-sidecar.sh"),
  "utf8",
);

test("Swift provider weak-links guarded Foundation Models APIs", () => {
  assert.match(swift, /import FoundationModels/u);
  assert.match(swift, /canImport\(FoundationModels\).*BATCAVE_FOUNDATION_MODELS_UNAVAILABLE/u);
  assert.match(swift, /@available\(macOS 26\.0, \*\)/u);
  assert.match(swift, /SystemLanguageModel\.default\.availability/u);
  assert.match(swift, /LanguageModelSession/u);
  assert.match(swift, /DynamicGenerationSchema/u);
  assert.match(swift, /maximumNarrativeCharacters = 180/u);
  assert.doesNotMatch(swift, /SystemLanguageModel\s*\(\s*adapter:/u);
  assert.doesNotMatch(swift, /URLSession|Network\.framework|com\.apple\.security\.network/u);
  assert.doesNotMatch(swift, /subject_stable_id|subjectStableID/u);
});

test("macOS build stages only the Apple Silicon sidecar", () => {
  assert.match(build, /fn build_macos_foundation_models_sidecar\(\)/u);
  assert.match(build, /CARGO_CFG_TARGET_OS/u);
  assert.match(build, /target_os == "macos"/u);
  assert.match(build, /arm64-apple-macos12\.0/u);
  assert.match(build, /-weak_framework/u);
  assert.match(build, /FoundationModels/u);
  assert.deepEqual(macosConfig.bundle.externalBin, [
    "target/foundation-models-sidecar/batcave-foundation-models",
  ]);
  assert.equal(macosConfig.bundle.macOS.minimumSystemVersion, "12.0");
});

test("bundle and native checks pin architecture, weak linkage, and nested signing", () => {
  for (const source of [verifier, nativeTest]) {
    assert.match(source, /batcave-foundation-models/u);
    assert.match(source, /LC_LOAD_WEAK_DYLIB/u);
    assert.match(source, /FoundationModels/u);
    assert.match(source, /arm64/u);
    assert.match(source, /12\.0/u);
  }
  assert.match(verifier, /codesign --verify/u);
  assert.match(verifier, /TeamIdentifier/u);
  assert.match(nativeTest, /FOUNDATION_MODELS_AVAILABILITY=/u);
  assert.match(nativeTest, /BATCAVE_FOUNDATION_MODELS_UNAVAILABLE/u);
  assert.match(nativeTest, /availability !== "unsupported"/u);
});
