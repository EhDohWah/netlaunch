# CLAUDE.md

Guidance for Claude Code (and humans) working in this repository.

NetLaunch is a Windows desktop app (Rust + Tauri) that stores admin console logins and opens each one in an automated Edge window. **`docs/SPEC.md` is the authoritative spec.** Read it before changing behavior, and update it when a decision changes.

## Status

- **M1 browser spike:** passed on Windows/Edge 155. That run found that new profiles weren't isolated. The spike now adds isolation, and a second Windows run is pending.
- **Everything else** (vault, engine, driver, mock site, Tauri app) is a placeholder crate.

## Commands

```bash
cargo build                                  # first build downloads the Playwright driver (~130 MB)
cargo test
cargo clippy --workspace --all-targets
cargo run -p netlaunch-spike -- help         # M1 spike; see spike/README.md
```

- **Windows-only code** (registry, DPAPI, process flags) is behind `#[cfg(windows)]`. From Linux, type-check it with:

  ```bash
  rustup target add x86_64-pc-windows-gnu
  PLAYWRIGHT_SKIP_DRIVER_DOWNLOAD=1 cargo check -p netlaunch-spike --target x86_64-pc-windows-gnu
  ```

- **If the driver download fails** (HTTPS-inspecting proxy; the build script uses its own certificate list), assemble the folder by hand. Follow `spike/README.md` and point `PLAYWRIGHT_DRIVER_PATH` (or `spike --driver-path`) at it.

## Layout

```
crates/vault       SQLite + DPAPI + XChaCha20-Poly1305 (M3)
crates/engine      recipes, origin check, BrowserDriver trait (M5); never imports playwright-rs
crates/driver-pw   BrowserDriver on playwright-rs (after M1)
crates/mock-site   axum test site with the spec's test pages (M2)
spike/             M1 throwaway CLI
docs/SPEC.md       the spec
```

- `vault` and `engine` must not depend on Tauri, so `cargo test` covers them without a GUI.
- The Tauri app goes in `app/` when M4 starts.

## Rules that must not be broken

These come from the spec's security section. Treat a change that breaks one as a bug, even if tests pass.

- **No Tauri command ever returns a password.** The frontend may send one when saving; nothing sends one back.
- **Never log filled text.**
  - Set the log level in code, never from `RUST_LOG`.
  - Cap `playwright_rs` at `debug`: at `trace` it logs the full JSON of every request, including the password being filled. The spike proved this.
  - Keep the test that fills a known password and asserts it's absent from the logs.
- **Origin check before every fill**, against the frame that holds the field, with origins normalized (default ports, lowercase host).
- **NetLaunch launches Edge itself, never playwright-rs.**
  - Each site session gets its own profile folder and `--remote-debugging-port=0`. NetLaunch reads the port from `DevToolsActivePort` and attaches with `connect_over_cdp`.
  - Always add `edge::isolation_args()` and seed the profile's preferences (password saving off). Without them, Edge on a work PC signs the new profile in to the Windows account, syncs it and loads synced extensions next to the password field.
  - Policies override switches, so check the result: no extra tabs, no signed-in account, no user extensions.
  - Use the browser's default context and its `about:blank` tab (found by URL, not position), not `new_context()`.
  - One Edge process per site session, because `--ignore-certificate-errors` is process-wide and must only apply to self-signed sites.
- **Managed close** sends the CDP `Browser.close` command (a clean quit), then kills the Edge process NetLaunch started if it is still running after 10 s. Releasing means playwright-rs `Browser::close()` (for an attached browser this only disconnects) plus stopping the driver.
- **Keep `playwright-rs` pinned exactly** in the root `Cargo.toml`. Upgrade on purpose, then re-run the mock-site suite. Its bundled Playwright version (currently 1.64.0) is what the manual driver fallback must match.
- **The 2FA code is never read, stored or typed by the engine.** Tests may type the mock site's test code through the driver.
- **No real credentials in the repo**, including tests and fixtures. The spike's `spike-user` / `Spike-Pa55-7f3c` account is fake.

## Conventions

- Rust edition 2024. Workspace-wide dependency versions live in `[workspace.dependencies]`.
- Explain non-obvious *why* in comments; the owner is learning Rust.
- Keep `cargo clippy --workspace --all-targets` warning-free.
