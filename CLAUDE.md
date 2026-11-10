# CLAUDE.md

Guidance for Claude Code (and humans) working in this repository.

NetLaunch is a Windows desktop app (Rust + Tauri) that stores admin console logins and opens each one in an automated Edge window. **`docs/SPEC.md` is the authoritative spec.** Read it before changing behavior, and update it when a decision changes.

## Status

- **M1 browser spike: done** (Windows 11, Edge 155). Attach, release and managed close all work, and the isolation switches stop sync, extensions and password saving.
- Edge's implicit sign-in to the Windows account remains (`account_info[0].email`; open decision 9).
- **M2 mock site: done** (`crates/mock-site`). Route tests pass, all scenarios pass in Chromium, and you checked them by hand in Edge. Decision 9: accept and warn.
- **M3 vault: built** (`crates/vault`). Tests pass on Linux and on Windows under Wine; CI runs them on real Windows. Decisions 4 and 8: no master password, usernames not encrypted.
- **M3 vault: done.** CI passes, and all 31 tests pass on the owner's PC with real DPAPI.
- **M4 Tauri shell: done** (`app/`). Site list and add/edit screens; checked on the owner's PC. Decision 7: Vue 3 + Vite.
  - Tauri is pinned to 2.11 on both sides: the Rust crate `~2.11` and npm `~2.11.x`. 2.12 needs Rust 1.90. Bump all three together.
  - The owner builds with the GNU toolchain (MinGW). It works, but the linker warns about duplicate manifests. Switch to MSVC before the M7 installer.
- **M5 login engine: done** (`crates/engine`, `crates/driver-pw`, Open in the app). Every mock scenario passes in Edge in CI and on the owner's PC.
- **M6 windows: built** (`crates/sessions`, the Open windows panel). 2FA watched until success, managed and released modes, Release/Close/Retry, signed-in warning. The owner confirmed `/mfa`; released mode, `/no-success` and the warning are unconfirmed.
- **M7 pilot: in progress.** CI builds and checks the installer (MSVC, bundled Playwright driver, `--self-check`, uninstall keeps the vault); log file; `probe` tool. The UniFi recipe waits for the owner's probe output.
- **CI** (`.github/workflows/ci.yml`) runs tests and `clippy -D warnings` on Windows and Linux for every push.

## Commands

```bash
cargo build                                  # first build downloads the Playwright driver (~130 MB)
cargo test --workspace --exclude netlaunch-app        # on Linux; the Windows-only app needs WebKitGTK to build here
cargo test -p netlaunch-driver-pw --test mock_site    # the login engine in a real browser against the mock site
cargo test -p netlaunch-sessions --test windows       # 2FA watch, managed/released windows, in a real browser
cargo clippy --workspace --all-targets
cd app && npm install && cargo tauri dev              # run the app (Windows); see app/README.md
cargo run -p netlaunch-spike -- help         # M1 spike; see spike/README.md
cargo run -p netlaunch-mock-site             # mock login site on https://localhost:8443; see crates/mock-site/README.md
cargo run -p netlaunch-probe -- <login URL>  # print a real login page's form, to write a recipe; see probe/README.md
cd app && npm run installer                           # NSIS installer with the bundled driver (Windows, MSVC); CI does this
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

- **Browser tests** (`crates/driver-pw/tests/mock_site.rs`, `crates/sessions/tests/windows.rs`) use `NETLAUNCH_TEST_BROWSER`, else the installed Edge, else Chrome/Chromium in the usual Linux places. With none they print SKIPPED and pass; `NETLAUNCH_REQUIRE_BROWSER=1` (set in CI) makes that a failure. In a Linux container they add `--no-sandbox`.

- **If the driver download fails** (HTTPS-inspecting proxy; the build script uses its own certificate list), assemble the folder by hand. Follow `spike/README.md` and point `PLAYWRIGHT_DRIVER_PATH` (or `spike --driver-path`) at it.

## Layout

```
crates/vault       SQLite + DPAPI + XChaCha20-Poly1305 (M3)
crates/engine      recipes (JSON in recipes/), origin check, runner, BrowserDriver trait (M5); never imports playwright-rs
crates/driver-pw   Edge launch + isolation (from the spike), Session = BrowserDriver on playwright-rs (M5)
crates/sessions    the open windows: runs a login, watches 2FA, release/close, signed-in warning (M6); no Tauri
crates/mock-site   axum test site with the spec's eight scenarios + a lookalike site (M2)
spike/             M1 throwaway CLI
probe/             prints a real login page's form (never typed values), for writing recipes (M7)
vendor/playwright-rs  playwright-rs 0.20.1 with the no-console-window patch; not a workspace member
app/               Tauri 2 app: Vue 3 frontend in src/, Rust commands in src-tauri/ (M4)
docs/SPEC.md       the spec
```

- `vault`, `engine`, `driver-pw` and `sessions` must not depend on Tauri, so `cargo test` covers them without a GUI.
- **Sessions** keep a `std::sync::Mutex` map, never held across an `await`. A kept window is an `Arc<Session>`; Close and Release abort its watcher, then need sole ownership (`Arc::try_unwrap`), so don't hold `Sessions::driver()` copies for long.
- **Tauri commands** (`app/src-tauri/src/main.rs`) are thin wrappers. Their logic lives in `sites.rs` (site list), `logins.rs` (Open, and the hooks that turn session news into events and `login_log` rows) and `crates/sessions`, so it's tested without a window.
- **Capabilities** (`app/src-tauri/capabilities/default.json`): `core:default` only, so no shell, file system or opener plugins. Keep it that way.
- **Wine check of the app.** The windows-gnu build links `WebView2Loader.dll` dynamically. Copy it from `~/.cargo/registry/src/*/webview2-com-sys-*/x64/` next to the test `.exe` before running it under Wine. MSVC builds link it statically.

## Rules that must not be broken

These come from the spec's security section. Treat a change that breaks one as a bug, even if tests pass.

- **No Tauri command ever returns a password.** The frontend may send one when saving; nothing sends one back.
  - `Vault::reveal_password` is the only way to get a password back. It's called in one place, the closure `logins.rs` builds for `Sessions::open`; the engine calls it when it fills the password field.
  - `Site` (what the UI gets) has no password field. A test checks its JSON contains no password.
  - Never enable the vault's `insecure-test-protector` feature outside tests; it stores the data key unprotected.
- **The 2FA watcher only reads.** `netlaunch_engine::logged_in` checks the `success` rule; nothing after the hand-over types or clicks.
- **Never log filled text.**
  - Set the log level in code, never from `RUST_LOG`.
  - Cap `playwright_rs` at `debug`: at `trace` it logs the full JSON of every request, including the password being filled. The spike proved this.
  - Keep the test that fills a known password and asserts it's absent from the logs.
- **Origin check before every fill**, against the frame that holds the field, with origins normalized (default ports, lowercase host).
  - `BrowserDriver::frame_url_of` must return the URL as the browser recorded it, never one read by JavaScript in the page.
  - Today selectors never enter iframes, so that's the main frame's URL. If recipes ever get frame support, `frame_url_of` must return the field's own frame.
  - URL rules (`url_contains`, `url_not_contains`) only count on the site's own origin.
- **The log filter comes from `netlaunch_engine::log_directives`**, which caps playwright-rs at `debug`.
- **NetLaunch launches Edge itself, never playwright-rs.**
  - Each site session gets its own profile folder and `--remote-debugging-port=0`. NetLaunch reads the port from `DevToolsActivePort` and attaches with `connect_over_cdp`.
  - Always add `edge::isolation_args()` (now in `crates/driver-pw/src/edge.rs`) and seed the profile's preferences (password saving off). Without them, Edge on a work PC signs the new profile in to the Windows account, syncs it and loads synced extensions next to the password field.
  - Policies override switches, so check the result: no extra tabs, no running extensions (CDP `Target.getTargets` with `filter: [{}]`; playwright-rs wraps the reply in `result`), no signed-in account.
  - The profile's `extensions.settings` can't tell installed extensions from Edge's own entries. Don't use it as a pass/fail check.
  - Use the browser's default context and its `about:blank` tab (found by URL, not position), not `new_context()`.
  - One Edge process per site session, because `--ignore-certificate-errors` is process-wide and must only apply to self-signed sites.
- **Managed close** sends the CDP `Browser.close` command (a clean quit), then kills the Edge process NetLaunch started if it is still running after 10 s. Releasing means playwright-rs `Browser::close()` (for an attached browser this only disconnects) plus stopping the driver.
- **playwright-rs is patched** (`vendor/playwright-rs`, through `[patch.crates-io]`): on Windows it starts `node.exe` with `CREATE_NO_WINDOW`, or the installed app opens a terminal next to every login window. Upgrading means re-applying that patch; see `vendor/playwright-rs/NETLAUNCH.md`.
- **Keep `playwright-rs` pinned exactly** in the root `Cargo.toml`. Upgrade on purpose, then re-run the mock-site suite. Its bundled Playwright version (currently 1.64.0) is what the manual driver fallback must match, and so is `PLAYWRIGHT_VERSION` in `app/scripts/stage-driver.mjs`.
- **The installed app uses its bundled driver.** `main` calls `use_bundled_driver()` first thing, while no other thread exists (it sets an environment variable). Keep it first.
- **Uninstalling must keep the vault.** The app installs into the vault's folder; CI checks `vault.db` survives an uninstall. Don't add installer steps that delete that folder.
- **The 2FA code is never read, stored or typed by the engine.** Tests may type the mock site's test code through the driver.
- **No real credentials in the repo**, including tests and fixtures. The spike's `spike-user` / `Spike-Pa55-7f3c` account is fake.

## Conventions

- Rust edition 2024. Workspace-wide dependency versions live in `[workspace.dependencies]`.
- Explain non-obvious *why* in comments; the owner is learning Rust.
- Keep `cargo clippy --workspace --all-targets` warning-free.
