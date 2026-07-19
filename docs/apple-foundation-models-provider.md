# Apple Foundation Models provider

BatCave's macOS narrative provider is a bundled Swift helper named
`batcave-foundation-models`. It is local-only and optional. The main app keeps
its macOS 12 deployment target; the helper weak-links `FoundationModels` and
guards every framework use with `@available(macOS 26.0, *)`.

The helper handles one request and exits. It reads one JSON line from stdin and
writes one JSON line to stdout. Input is capped at 32 KiB and output at 4 KiB.
The native adapter owns the ten-second deadline and kills and reaps the helper
when the request is cancelled or times out.

Requests use protocol version `1`:

```json
{"version":1,"operation":"status"}
```

```json
{"version":1,"operation":"generate","request":{"surface":"overview_contributor","publication_seq":42,"fact_digest":"<64 lowercase hex characters>"},"facts":{}}
```

Generation metadata deliberately excludes subject IDs, process IDs, and paths.
Only the typed fact packet is included in the model prompt. The helper returns
`available`, `unsupported`, `model_not_ready`, `runtime_missing`, or `busy`.
A successful result echoes only the provider name, publication sequence, fact
digest, and one sentence capped at 180 characters.

Build and verify the helper directly on Apple Silicon:

```sh
bash scripts/test-foundation-models-sidecar.sh
```

The Cargo build script compiles the helper with the active macOS SDK and stages
it as Tauri external binary output under `src-tauri/target/`. The normal macOS
bundle flow signs it as nested code. `scripts/verify-macos-bundle.sh` checks its
arm64-only architecture, macOS 12 deployment target, weak framework load
command, nested signature, hardened runtime, matching release identity, and
absence of network entitlements.

No custom model adapter is bundled. Runtime readiness comes only from
`SystemLanguageModel.default.availability`; SDK compilation alone does not
prove that the model is ready on a user's Mac.
