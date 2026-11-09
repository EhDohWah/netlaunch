# CLAUDE.md

Guidance for Claude Code (and humans) working in this repository.

NetLaunch is a Windows desktop app (Rust + Tauri) that stores admin console logins and opens each one in an automated Edge window. **`docs/SPEC.md` is the authoritative spec.** Read it before changing behavior, and update it when a decision changes.

## Status

- **M1 browser spike: done** (Windows 11, Edge 155). Attach, release and managed close all work, and the isolation switches stop sync, extensions and password saving.
- Edge's implicit sign-in to the Windows account remains (`account_info[0].email`; open decision 9).
- **M2 mock site: done** (`crates/mock-site`). Route tests pass, all scenarios pass in Chromium, and you checked them by hand in Edge. Decision 9: accept and warn.
- **M3 vault: built** (`crates/vault`). Tests pass on Linux and on Windows under Wine; CI runs them on real Windows. Decisions 4 and 8: no master password, usernames not encrypted.
- **M3 vault: done.** CI passes, and all 31 tests pass on the owner's PC with real DPAPI.
- **M4 Tauri shell: built** (`app/`). Site list and add/edit screens. Command tests and UI checks pass; the restart check by hand on Windows is pending. Decision 7: Vue 3 + Vite.
- **Everything else** (engine, driver) is a placeholder crate.
- **CI** (`.github/workflows/ci.yml`) runs tests and `clippy -D warnings` on Windows and Linux for every push.

## Commands

```bash
cargo build                                  # first build downloads the Playwright driver (~130 MB)
cargo test --workspace --exclude netlaunch-app        # on Linux; the Windows-only app needs WebKitGTK to build here
cargo clippy --workspace --all-targets
cd app && npm install && cargo tauri dev              # run the app (Windows); see app/README.md
cargo run -p netlaunch-spike -- help         # M1 spike; see spike/README.md
cargo run -p netlaunch-mock-site             # mock login site on https://localhost:8443; see crates/mock-site/README.md
```

- **Windows-only code** (registry, DPAPI, process flags) is behind `#[cfg(windows)]`. From Linux, build and run it with mingw and Wine. Bundled SQLite and `ring` need the mingw C compiler.

  ```bash
  apt-get install -y gcc-mingw-w64-x86-64 wine64
  rustup target add x86_64-pc-windows-gnu
  PLAYWRIGHT_SKIP_DRIVER_DOWNLOAD=1 cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
  cargo test -p netlaunch-vault --target x86_64-pc-windows-gnu --no-run
  WINEDEBUG=-all /usr/lib/wine/wine64 target/x86_64-pc-windows-gnu/debug/deps/netlaunch_vault-<hash>.exe
  ```

  Wine's DPAPI isn't the real one; CI's `windows-latest` job is the real-Windows check.

- **If the driver download fails** (HTTPS-inspecting proxy; the build script uses its own certificate list), assemble the folder by hand. Follow `spike/README.md` and point `PLAYWRIGHT_DRIVER_PATH` (or `spike --driver-path`) at it.

## Layout

```
crates/vault       SQLite + DPAPI + XChaCha20-Poly1305 (M3)
crates/engine      recipes, origin check, BrowserDriver trait (M5); never imports playwright-rs
crates/driver-pw   BrowserDriver on playwright-rs (after M1)
crates/mock-site   axum test site with the spec's eight scenarios + a lookalike site (M2)
spike/             M1 throwaway CLI
app/               Tauri 2 app: Vue 3 frontend in src/, Rust commands in src-tauri/ (M4)
docs/SPEC.md       the spec
```

- `vault` and `engine` must not depend on Tauri, so `cargo test` covers them without a GUI.
- **Tauri commands** (`app/src-tauri/src/main.rs`) are thin wrappers. Their logic lives in `sites.rs` as plain functions over a `Vault`, so it's tested without a window.
- **Capabilities** (`app/src-tauri/capabilities/default.json`): `core:default` only, so no shell, file system or opener plugins. Keep it that way.
- **Wine check of the app.** The windows-gnu build links `WebView2Loader.dll` dynamically. Copy it from `~/.cargo/registry/src/*/webview2-com-sys-*/x64/` next to the test `.exe` before running it under Wine. MSVC builds link it statically.

## Rules that must not be broken

These come from the spec's security section. Treat a change that breaks one as a bug, even if tests pass.

- **No Tauri command ever returns a password.** The frontend may send one when saving; nothing sends one back.
  - `Vault::reveal_password` is the only way to get a password back. Only the login engine calls it, at fill time.
  - `Site` (what the UI gets) has no password field. A test checks its JSON contains no password.
  - Never enable the vault's `insecure-test-protector` feature outside tests; it stores the data key unprotected.
- **Never log filled text.**
  - Set the log level in code, never from `RUST_LOG`.
  - Cap `playwright_rs` at `debug`: at `trace` it logs the full JSON of every request, including the password being filled. The spike proved this.
  - Keep the test that fills a known password and asserts it's absent from the logs.
- **Origin check before every fill**, against the frame that holds the field, with origins normalized (default ports, lowercase host).
- **NetLaunch launches Edge itself, never playwright-rs.**
  - Each site session gets its own profile folder and `--remote-debugging-port=0`. NetLaunch reads the port from `DevToolsActivePort` and attaches with `connect_over_cdp`.
  - Always add `edge::isolation_args()` and seed the profile's preferences (password saving off). Without them, Edge on a work PC signs the new profile in to the Windows account, syncs it and loads synced extensions next to the password field.
  - Policies override switches, so check the result: no extra tabs, no running extensions (CDP `Target.getTargets` with `filter: [{}]`; playwright-rs wraps the reply in `result`), no signed-in account.
  - The profile's `extensions.settings` can't tell installed extensions from Edge's own entries. Don't use it as a pass/fail check.
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
