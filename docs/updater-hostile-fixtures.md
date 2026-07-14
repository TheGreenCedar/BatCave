# Local updater hostile-case verification

BatCave keeps updater checks explicit and offline-independent. The production app still uses only the embedded public key and stable GitHub endpoint in `tauri.conf.json`; the local fixture matrix changes neither.

Run the native boundary matrix with:

```bash
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml updater_hostile_fixtures
```

The tests bind an ephemeral loopback HTTP server and call the pinned `tauri-plugin-updater` 2.10.1 Rust API. Tauri performs the real HTTP request, JSON parsing, target selection, default forward-only SemVer comparison, payload download, and Minisign verification. Disposable fixture keys sign only the four-byte fixture payload and are compiled only into Rust tests.

| Fixture | Expected boundary |
| --- | --- |
| HTTP 503, missing manifest, malformed JSON, empty response, or wrong target | Check fails; no payload request starts. |
| HTTP 204 | No update is returned. |
| Equal or lower SemVer | No update is returned; no downgrade comparator is enabled. |
| Valid higher SemVer and exact signed bytes | An update is selected and `Update::download` returns the verified bytes. |
| Unrelated key, malformed signature, or byte-tampered payload | Download fails during Tauri signature verification. No installer call is made. |
| Unreachable loopback endpoint | Check fails as an ordinary offline error. |

Frontend lifecycle tests run through `npm run test:update-lifecycle`. They prove a new explicit check closes the previous JavaScript `Update` before requesting another, and that download or installation completion and failure close the consumed resource. Cleanup treats only Tauri 2.11.5's exact invalid-resource-ID error as an idempotent already-removed handle. Any other close failure blocks replacement and remains available for Retry; a combined operation and cleanup failure preserves both errors.

## Evidence boundary

This matrix deliberately stops at `Update::download`: that is the last common, non-mutating boundary after exact-byte signature verification and before platform-specific installation. Running `downloadAndInstall` with a disposable package could modify the implementation host, and a generic test package would not be a truthful packaged BatCave update.

The matrix therefore does not prove a packaged GUI flow, a platform installer transition, public endpoint routing, a public A-to-B version change, preserved settings across installation, Authenticode, Developer ID, notarization, manifest expiry, newest-release freshness, replay resistance for a still-higher version, or cryptographic stable-channel binding. Those public and platform claims remain in #47 after #42 and #76 produce signed release artifacts.
