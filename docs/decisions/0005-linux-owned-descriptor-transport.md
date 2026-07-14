# Linux Rust-owned descriptor transport

- Status: accepted for the synthetic issue #139 spike; not accepted for native package execution
- Date: 2026-07-14
- Parent architecture: [Rust-owned native artifact consumption authority](0003-private-native-artifact-consumption-authority.md)
- Scope: issues #139, #115, #130, and #62

## Decision

A sealed anonymous memory file, reopened as a read-only descriptor and inherited only by a fixed child, is the preferred Linux byte transport for the Rust-owned authority. The child may read that inherited descriptor directly. A fixed child that requires a filename-shaped input may use its own `/proc/self/fd/N` alias, but the descriptor number and alias stay inside the Rust supervisor and fixed child.

This decision does not approve a Linux package adapter. It proves only that exact synthetic bytes can remain bound to Rust ownership through a fixed consumer and complete process-group settlement. `dpkg`, AppImage runtime behavior, trust checks, installation, mounting, launch, removal, and residue remain native gates in #115.

There is no temporary-file fallback. If `memfd_create`, sealing, read-only reopening, descriptor inheritance, process-group ownership, or required `/proc` behavior is unavailable, that transport is unsupported.

## Closed transport

1. Rust verifies the selected regular non-link source against a closed size and SHA-256 binding.
2. Rust copies the exact bytes into a `memfd_create(MFD_ALLOW_SEALING)` object.
3. Rust applies `F_SEAL_WRITE`, `F_SEAL_GROW`, `F_SEAL_SHRINK`, and `F_SEAL_SEAL`.
4. Rust reopens the memory file through its private `/proc/self/fd/N` view as `O_RDONLY`, confirms device/inode identity, access mode, and seals, then drops the writable descriptor.
5. The fixed child receives the read-only object at a fixed inherited descriptor. No caller supplies a descriptor, path, executable, arguments, environment, callback, command runner, status, or completion.
6. Rust owns a new process group, acts as a child subreaper, enforces the deadline, detects surviving descendants, terminates the group, reaps adopted children, and checks that the group has disappeared.
7. A supervisor error before spawn is a consumption failure with no process ownership. Any error after spawn carries the child and process-group state back into the authority as unresolved ownership.
8. While the authority is live, an unresolved group retains the child handle, process-group identity, subreaper guard, and descriptor until settlement succeeds. Explicit recovery terminates, reaps, and only then closes. `Drop` is a bounded fail-safe that attempts the same containment before fields are released; the production composition root must keep the authority alive for observable retry rather than use `Drop` as its normal recovery path.
9. A simulated cleanup failure after proven settlement retains the descriptor until an explicit retry.

The sanitized outcome contains the selected internal transport, failure boundaries, observed synthetic size and digest, settlement, cleanup, and residue state. It contains no descriptor, private path, generic command surface, native receipt, or evidence packet.

## Transport comparison

| Option | Decision | Reason |
| --- | --- | --- |
| Inherited read-only sealed `memfd` | Preferred | Keeps the owned object unnamed and binds the fixed child to the descriptor Rust validated. |
| Child-private `/proc/self/fd/N` | Conditional | Useful only as an alias inside a fixed child; it is not returned to JavaScript or accepted as caller input. |
| Ordinary temporary or memory-backed path | Rejected | A discoverable same-user namespace reintroduces replacement and cleanup ambiguity. |
| `fexecve` or `execveat` | Rejected for package-byte transport | These execute an executable image. They do not make a deb payload consumable by `dpkg`, and they do not establish AppImage runtime, mount, or cleanup behavior. |
| Caller-visible descriptor or path | Rejected | It lets the caller disclose, close, replay, or pair authority from different operations. |

Real AppImage testing may still determine that `execveat` is useful inside a fixed native adapter, but that is a separate empirical decision in #115. It is not part of this spike and cannot be inferred from synthetic bytes.

## Failure behavior

The integration test covers the failure boundary that can be established without running a package:

- replacement of the public source after acquisition does not alter the owned bytes;
- a substituted descriptor causes the fixed consumer to fail;
- replay and close-before-use fail before launch;
- timeout terminates and reaps the owned group before cleanup;
- a parent that exits while a descendant survives is classified as settlement failure, then terminated and reaped;
- a supervisor failure after spawn retains group and descriptor ownership, rejects early close, and requires a successful terminate/reap retry;
- dropping that unresolved authority takes the bounded containment path and leaves the retained process group absent;
- cleanup failure retains the descriptor until retry;
- linked roots and mismatched source bytes fail acquisition; and
- non-Linux hosts report the transport as unsupported without a weaker fallback.

All failure outcomes keep `package_bytes_executed` and `native_proven` false. Successful synthetic consumption also keeps both false.

## Kernel and tool assumptions

The accepted spike requires Linux support for `memfd_create`, file seals, `/proc/self/fd` for the private read-only reopen, inherited descriptors, process groups, `prctl(PR_SET_CHILD_SUBREAPER)`, `kill`, and `waitpid`. Container policies or kernels that remove any required primitive are unsupported unless a later reviewed transport satisfies the same closed contract.

The process supervisor trusts the reviewed Rust binary, Linux kernel, Rust standard library, and fixed child binary. Arbitrary native code inside the Rust process, a compromised kernel, debugger access, or hostile administrator remains outside the ordinary-caller threat boundary defined in ADR 0003.

## Evidence and non-claims

[`linux_owned_descriptor_transport_spike.rs`](../../src/BatCave.App/src-tauri/tests/linux_owned_descriptor_transport_spike.rs) is an integration-test crate. It is not linked into production, registered as a Tauri command, exposed as a CLI mode, or callable from JavaScript.

Run the focused test with:

```sh
cargo test --manifest-path src/BatCave.App/src-tauri/Cargo.toml --test linux_owned_descriptor_transport_spike
```

On Linux, the test launches only its own fixed synthetic consumers. On other hosts, it checks the explicit unsupported result. The normal Rust validation suite discovers the test without a workflow-specific test hook.

No `dpkg` command or AppImage is run. No package is installed, mounted, staged, launched, removed, or accepted. The spike does not mint a native execution receipt or public evidence and does not satisfy #115 by itself.
