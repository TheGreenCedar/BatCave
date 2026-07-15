# Windows NSIS owned-adapter source contract

- Status: accepted source contract; native install proof remains outstanding
- Date: 2026-07-15
- Issue: #113

## Decision

Keep the Windows NSIS slice test-only until a signed public installer can run through the complete standard-user install and uninstall gates. The source contract owns one inert executable image in a private root, retains a read handle that blocks write/delete replacement, launches only that image with a fixed command line and empty environment, assigns the suspended child to a kill-on-close job before it can execute, and rehashes the same handle after the complete job settles. Cleanup records the image's stable file identity, keeps a no-delete-share handle on the exact root, reopens and revalidates the exact leaf with delete authority, and marks that leaf delete-pending. Windows immediately removes the pending name from the link count, so cleanup requires zero remaining links while the same handle stays open; delete-pending blocks new hard links, and any already-linked image leaves a nonzero count before the root can be removed or evidence can be emitted. Cleanup then deletes the exact root by handle. It never releases both ownership handles and follows the old pathname for recursive deletion.

The entry accepts only a private hostile scenario. It has no caller command, executable, path, argument, environment, status, observation, receipt, or evidence input. UAC denial is represented only as a pre-child source state; this suite does not request elevation.

Denial, timeout, child failure, residue, ownership failure, and cleanup failure remain separate. Timeout terminates the entire job and waits for zero active processes. A pathname, stable-identity, or zero-remaining-link mismatch fails cleanup without deleting a replacement or claiming removal. Source evidence is constructed only after child-tree settlement, byte revalidation, residue evaluation, and handle-authorized private-root cleanup.

## Proof boundary

The exercised image is the Windows test binary, not an NSIS installer. `package_bytes_executed`, `public_artifact_verified`, and `native_proven` are therefore not claimed, and `release_evidence` remains null. The source evidence retains `windows_service_etw_out_of_scope` while #70 is incomplete.

Final #113 proof still requires the exact Rust-owned immutable public artifact, Authenticode publisher verification from #42, standard-user install and UAC outcomes, installed app/CLI/uninstaller identity, settings restart, standard-access degradation, telemetry, uninstall, helper/broker/pipe/signal settlement, user-state policy, and zero residue. None of those machine mutations occur in this source slice.

## Verification

```powershell
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml --test windows_nsis_owned_adapter -- --nocapture
cargo clippy --manifest-path src/BatCave.App/src-tauri/Cargo.toml --test windows_nsis_owned_adapter -- -D warnings
```
