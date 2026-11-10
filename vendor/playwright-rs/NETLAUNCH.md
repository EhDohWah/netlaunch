# playwright-rs 0.20.1, patched for NetLaunch

A copy of [playwright-rs](https://crates.io/crates/playwright-rs) 0.20.1 (Apache-2.0), used through `[patch.crates-io]` in the root `Cargo.toml`. Only `src/`, `build.rs`, `README.md` and `clippy.toml` are copied. Tests, examples and benchmarks are left out, and their targets and dev-dependencies are removed from `Cargo.toml`.

## The change

On Windows, every program the crate starts gets the `CREATE_NO_WINDOW` creation flag:
- the Playwright driver (`node.exe … run-driver`, in `src/server/playwright_server.rs`);
- the browser installer and the `npm` / `where` lookups (`src/server/driver.rs`).

**Why:** the installed NetLaunch is a windowed program with no console. Windows gives a console program started from it, like `node.exe`, a new terminal window unless it's started with `CREATE_NO_WINDOW`. Without the patch, a terminal window appeared in front of every login window. The crate has no option to pass creation flags.

Every changed place is marked `NetLaunch patch`.

## Upgrading playwright-rs

1. Bump the pin in the root `Cargo.toml`.
2. Copy the new version's files here the same way.
3. Re-apply the change (search the old copy for `NetLaunch patch`).

Or drop this folder and the `[patch.crates-io]` entry, if upstream has added a way to hide the window.
