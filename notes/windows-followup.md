# Windows follow-up: robustness fixes from the 1.0 audit

The 1.0 audit changed the Windows ETW provider runtime
(`crates/anyprobe/src/windows.rs`). The changes build and pass clippy for
`x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`, but nobody has run
them on Windows yet. Linux and FreeBSD (golden_goose) were run for real.

## What changed

| Change | Where | Normal path exercised by |
|---|---|---|
| `Once::call_once` became `call_once_force`, so a panic during one probe's first attach no longer makes every later `enabled()` check of that probe panic | `etw::Probe::attach` | Every probe's first enabled check: all attach scripts |
| `unregister_all` takes a poisoned lock instead of treating it as held, so a panic elsewhere no longer skips unregistering (which, in a DLL, lets ETW call into unloaded code) | `etw::try_lock`, `etw::unregister_all` | Every process exit; DLL unload in `check-gaps-windows.ps1` |
| Comment on why a refused registration leaks its `Provider` | `etw::Provider::get` | Nothing to run |

Changes elsewhere in the audit that Windows also builds and runs:

- `cargo anyprobe wprp` escapes the binary path in the profile's header
  comment (`crates/cargo-anyprobe/src/script.rs`).
- `cargo anyprobe` reads its arguments with `args_os` and reports a
  non-UTF-8 one as a usage error. The unit test for that is `cfg(unix)`.
- `registry::parse` rejects names the macros never write.

The recovery branches themselves (a poisoned lock, a panicked first
attach) cannot be triggered by any script; the checks below confirm the
normal paths still behave as before.

## CI covers most of it

Once the changes are pushed, the `spike-windows` job runs steps 2 and 3
below on `windows-latest` and `windows-11-arm`, release and release-lto,
and the `build` job runs step 1. If both are green, only step 4 is left.

## Steps on a Windows machine

Run from the repository root in an elevated Windows PowerShell (step 1
does not need elevation). Pull the branch with the audit changes first.

1. Tests, each feature set:

   ```powershell
   cargo test --workspace
   cargo test --workspace --no-default-features
   cargo test --workspace --features autoref
   ```

   Expect no failures.

2. ETW attach, spike and the anyprobe `work` example:

   ```powershell
   powershell -ExecutionPolicy Bypass -File spike\scripts\attach-windows.ps1
   $env:ATTACH_CRATE = 'anyprobe'
   powershell -ExecutionPolicy Bypass -File spike\scripts\attach-windows.ps1
   ```

   Expect `ok` per check and a final `PASS` for each, with profile headers
   `== release (anyprobe-spike)` and `== release (anyprobe example work)`.
   This is the check that `call_once_force` still attaches every probe and
   turns it on under a session.

3. `#[probe]`, every encoding, `cargo anyprobe wprp` and `wpr`:

   ```powershell
   powershell -ExecutionPolicy Bypass -File spike\scripts\attach-windows-attr.ps1
   ```

   Expect a final `PASS`. This also runs the changed `wprp` output through
   `wpr -start`, so a profile the path escaping broke would fail here.

4. DLL load and unload (not in CI):

   ```powershell
   powershell -ExecutionPolicy Bypass -File spike\scripts\check-gaps-windows.ps1
   ```

   Loads a DLL that defines probes with `LoadLibraryW`, traces it, and
   unloads it. Unloading runs `unregister_all` through the CRT's `atexit`
   handling. Expect the same results `docs/GAPS.md` records under "Shared
   libraries" (events from a provider only the DLL defines, and from a
   provider both define, recorded from each module) and no crash at unload
   or at exit.

5. Optional, on an ARM64 Windows machine: repeat steps 2 and 3. CI's
   `windows-11-arm` runner covers them, so this matters only if that job is
   red or skipped.

## If something fails

- A failure in step 2 or 3 that `main` does not show points at the
  `call_once_force` change in `etw::Probe::attach`.
- A crash at DLL unload or process exit in step 4 points at
  `etw::unregister_all`.
- A `wpr -start` error about the profile in step 3 points at
  `script::xml_text` in `cargo-anyprobe`.

When all steps pass, nothing in the docs needs updating: the changes do not
alter any recorded output. Delete this file.
