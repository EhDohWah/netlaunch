# NetLaunch — Spec v0.2

v0.1: Nov 3, 2026 · @SawEhDohWah
v0.2: Nov 3, 2026. Decisions 1 and 5 settled, corrections from reading playwright-rs 0.20.1, and the M1 spike design.

## Overview

NetLaunch is a Windows desktop app (Rust + Tauri) that stores admin console logins and opens each one with a single click. It launches an automated Edge window, fills the login form, and hands the window to the admin at any 2FA prompt.

## Goals

- One-click login to internal consoles (UniFi, routers, switches, Proxmox, Synology DSM, iDRAC).
- Credentials stored encrypted at rest. Plaintext exists only in memory, only for the moment of login.
- 2FA is never automated. The app pauses and the admin completes it in the same window.
- After login, the window either stays managed by the app or is released to the admin.

## Non-goals (v1)

- Not a general password manager: no browser extension, no autofill on arbitrary sites, no password sharing or sync.
- No storage of TOTP secrets. The second factor stays on the phone in Microsoft Authenticator.
- No CAPTCHA or bot-detection bypass. Such sites fall back to manual login.
- macOS and Linux builds are out of scope until the Windows version is stable.

## Choices

| Choice | Verdict | Why |
|---|---|---|
| Rust + Tauri | Good fit | Small native Windows app; the Rust backend keeps secrets out of the web UI; Vue skills cover the frontend. |
| Windows first | Good fit | DPAPI protects the key; Edge is always installed. |
| Microsoft Authenticator for 2FA | Good fit, manual only | The app never reads the code. It detects the 2FA screen, pauses, and the admin types the code into the same window. |
| Storage | SQLite | See the comparison below. |
| Browser driver | **playwright-rs** (decided) | See "Browser layer". |
| Browser | **Installed Edge** (decided) | Always present on Windows. |

### Storage comparison

| Option | Pros | Cons |
|---|---|---|
| **SQLite + encrypted secret fields (chosen)** | Queryable site list, safe atomic writes, mature crate (`rusqlite`), only secrets are encrypted | You manage the encryption key (solved by DPAPI) |
| SQLCipher (whole-DB encryption) | Everything encrypted, including URLs and usernames | Extra native build dependency on Windows; still needs a key |
| Custom encrypted `.dat` file | Simple to start | You design the format, corruption handling and migrations; whole file rewritten on every save |

## Browser layer

### playwright-rs, corrected (v0.2)

v0.1 said playwright-rs needs Node.js 18+ on the PC. That is wrong for 0.20.x:

- **The driver ships its own Node.js runtime.** The crate's build script downloads `playwright-core` (npm) and a pinned Node binary and assembles them into one folder: `node.exe` + `package/cli.js`. About 130 MB.
- **The first build needs internet access; the app never does.** The build script uses its own certificate list. A network that inspects HTTPS (some office proxies) makes the download fail. The fallback is to assemble the folder by hand and set `PLAYWRIGHT_DRIVER_PATH` (see `spike/README.md`).
- **At runtime the driver is found in this order:**
  1. `PLAYWRIGHT_DRIVER_PATH` (a folder holding `node.exe` and `package/cli.js`)
  2. the path the build script recorded
  3. the user cache
  4. global, then local, npm

  **The installer (M7) ships the driver folder as a Tauri resource and sets `PLAYWRIGHT_DRIVER_PATH` to it before starting Playwright.** Expect an installer of roughly 130 MB or more.
- **`Locator::fill` sends real input events**, so React/Vue "controlled" forms such as UniFi see the typed text. The spike checks this with a controlled-form page.
- **At `trace` level the crate logs every request's full JSON, including filled text.** The spike confirmed the password shows up in a `trace` log and not at `info` or `debug`. See the security rules.

Pin the crate exactly (`=0.20.1`), and keep all driver code behind the `BrowserDriver` trait.

### How the browser runs

NetLaunch never lets playwright-rs launch the browser. It starts Edge itself, as a separate process:

```
msedge.exe --user-data-dir=<profile folder>
           --remote-debugging-port=0          (Edge picks a free port)
           --no-first-run --no-default-browser-check
           --disable-sync                     (isolation, see below)
           --disable-extensions
           --disable-component-extensions-with-background-pages
           --disable-default-apps
           --disable-features=msForceBrowserSignIn,SwitchIntranetSitesToWorkProfile
           --edge-skip-compat-layer-relaunch  (Edge must not relaunch itself as a new process)
           [--ignore-certificate-errors]      (only for a self-signed site)
           about:blank
```

Before launch, write `<profile folder>\Default\Preferences` with `credentials_enable_service: false`, `profile.password_manager_enabled: false` and autofill off, so Edge never offers to save the password it sees typed.

1. Read the port from `<profile folder>\DevToolsActivePort` (first line).
2. Attach with `chromium().connect_over_cdp("http://127.0.0.1:<port>")`.
3. Use the browser's **default context and its `about:blank` tab**, never `new_context()`. Pick the tab by its URL, not by position, because something else may have opened a tab first.
4. **Released:** call `Browser::close()` (for an attached browser this only disconnects), then stop the driver. Edge keeps running.
5. **Managed close:** send the CDP `Browser.close` command so Edge quits cleanly and saves the profile. If it hasn't exited after 10 s, kill the process. Then delete a fresh profile folder.

**Isolation.** The first Windows run showed that a brand-new profile isn't automatically clean. Without the isolation switches, Edge on a work PC did three things:
- signed the new profile in to the Windows account
- synced the account's favorites
- installed the synced extensions, which opened their own tabs

Those extensions run in the window that receives the password, and the admin console URLs would sync back to the account. Playwright adds most of these switches when it launches a browser itself; NetLaunch launches Edge itself, so it adds them. The list follows Playwright 1.64's `chromiumSwitches.ts`, keeping only the switches about isolation.

**Edge policies beat command-line switches.** Machine policies such as these override the switches:
- `BrowserSignin`
- `ForceSync`
- `ExtensionInstallForcelist`
- `PasswordManagerEnabled`

So the app must check its result, not assume it. After launch, look for tabs it didn't open, and ask the browser which extensions are running (CDP `Target.getTargets`, with an explicit `filter: [{}]`). After a managed close, read the profile's preferences for a signed-in account and password saving. Warn the admin when isolation failed. The profile's extension list can't tell installed extensions from Edge's own entries, so it's diagnostic only.

`SwitchIntranetSitesToWorkProfile` is disabled too. Edge can move intranet URLs, such as a 10.x console, into the admin's "Work" profile window, away from the automated one.

Why:
- Recent Chrome/Edge versions refuse remote debugging on the default profile, so every launch uses NetLaunch's own profile folder.
- `--ignore-certificate-errors` applies to the whole browser process, so the rule is **one Edge process per site session**.
- Fresh profiles live in a temp folder that is deleted on managed close. Persistent profiles live in `%LOCALAPPDATA%\NetLaunch\profiles\<site_id>`.

Find Edge through the registry (`HKLM`, then `HKCU`, `SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\msedge.exe`). If that fails, check `%ProgramFiles(x86)%`, `%ProgramFiles%` and `%LOCALAPPDATA%` under `Microsoft\Edge\Application\msedge.exe`.

**Spike status:**
- **Steps 1–5 pass on Windows with Edge 155** (Nov 4, 2026), in both released and managed modes.
- **That run found the isolation problem above.** The second run, with isolation, removed the synced favorites, synced extensions, extra tabs and password saving.
- **Edge still signs the profile in to the Windows account** (implicit sign-in). No command-line switch stops it; only the `ImplicitSignInEnabled` policy does. With sync and extensions off, what remains is single sign-on to Microsoft sites in that window. See open decision 9.
- **The third Windows run closes M1** (Nov 4, 2026). With isolation on, every check passes except sign-in:
  - no extra tabs and no running extensions
  - password saving off
  - clean quit, and the profile deletes afterwards
- **The sign-in sits at `account_info[0].email`.** That's Chromium's list of signed-in accounts, which confirms Edge's implicit sign-in.

## Architecture

One Tauri process holds everything sensitive. The browser is a separate process that receives the password only at the instant it is typed into the form.

- **Vue frontend** shows sites and status, and calls Rust through Tauri commands.
- **Vault** (`crates/vault`) owns the SQLite file and the data key.
- **Login engine** (`crates/engine`) runs a site's recipe, checks the origin, and watches for success or a 2FA screen.
- **`BrowserDriver` trait** (in `crates/engine`) hides the automation library. `crates/driver-pw` implements it with playwright-rs. The engine never imports playwright-rs.
- **Playwright driver** is a child process (bundled Node + `cli.js`) that playwright-rs talks to over stdio. Passwords pass through it on their way to Edge.

## Login flow and 2FA handoff

Every path ends with a usable window in the admin's hands. The app never guesses or reads a 2FA code.

- **2FA handoff** means the app stops sending input, brings the window to the front (`Page::bring_to_front`), and shows "Waiting for 2FA" in the session panel. It keeps watching for the success rule, so the status flips to Logged in once the admin finishes.
- **Managed window:** the app keeps the automation connection. Closing the app closes the window.
- **Released window:** the app disconnects and Edge keeps running on its own. The debug port stays open until that window closes (see Risks).

## Credential storage and security

Passwords are encrypted per field with a random data key. That key is protected by Windows DPAPI, so only the admin's Windows account can unlock it.

### Key handling

1. First run: generate a random 256-bit data key.
2. Wrap it with DPAPI (`CryptProtectData`, current-user scope) and store the wrapped blob in `app_meta` with a `key_version`.
3. On start: unwrap with DPAPI and hold the key in memory using the `secrecy` / `zeroize` crates (wiped on drop).
4. Optional master password on top (Argon2id-derived key); see Open decisions.

### Encrypting a password

- Algorithm: XChaCha20-Poly1305 (`chacha20poly1305` crate), with a fresh random 24-byte nonce on every save.
- Associated data: `"sites.password:" + site id`. A ciphertext copied onto another row, or reused for a different field later, fails to decrypt.

### Rules the code must follow

- **No command returns a password.** The frontend can send a password when saving, but nothing ever sends one back to it.
- **Origin check before every fill.** Before each `fill` (not once per login), the engine checks that the scheme, host and port of the frame holding the field match the saved origin exactly.
  - Origins are normalized with the `url` crate (lowercase host, default ports, IPv6 brackets), so `https://x` and `https://x:443` match.
  - A redirect to any other origin, or a password field inside a frame from another origin, stops the login.
- **Self-signed exceptions stay per site.** Ignoring certificate errors applies only to the Edge process opened for that site, and only when the site has the self-signed flag on.
- **No passwords in logs or errors.** Usernames are allowed.
  - The log level is set in code, never from `RUST_LOG`.
  - `playwright_rs` is capped at `debug`, because at `trace` it logs filled text.
  - A test fills a known password and checks the log output doesn't contain it.
- **Minimal Tauri capabilities:** no general shell or filesystem access from the frontend.
- **Credentials are typed in by hand.** Nothing is bulk-imported from the knowledge base PDF, and those exposed passwords should be rotated.
- **Known limit:** `secrecy` wipes the app's own copy of the password, but the JSON message to the driver holds it briefly and can't be wiped.

## Data model

One SQLite file in `%LOCALAPPDATA%\NetLaunch\vault.db` with three tables. Only the password column is encrypted.

```sql
CREATE TABLE app_meta (
  key   TEXT PRIMARY KEY,   -- 'schema_version', 'wrapped_data_key', 'key_version'
  value BLOB NOT NULL
);

CREATE TABLE sites (
  id                 INTEGER PRIMARY KEY,
  name               TEXT NOT NULL,        -- 'UniFi Controller'
  group_name         TEXT,                 -- 'Wireless', 'Servers'
  login_url          TEXT NOT NULL,        -- full URL of the login page
  origin             TEXT NOT NULL,        -- normalized scheme://host:port, derived from login_url on save
  username           TEXT NOT NULL,
  password_ct        BLOB NOT NULL,        -- XChaCha20-Poly1305 ciphertext
  password_nonce     BLOB NOT NULL,        -- 24 bytes
  recipe_id          TEXT NOT NULL DEFAULT 'generic-form',
  allow_self_signed  INTEGER NOT NULL DEFAULT 0,
  profile_mode       TEXT NOT NULL DEFAULT 'fresh',   -- 'fresh' | 'persistent'
  window_mode        TEXT NOT NULL DEFAULT 'managed', -- 'managed' | 'released'
  notes              TEXT,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL,
  last_used_at       TEXT
);

-- One row per login attempt; outcome is updated when the attempt ends.
CREATE TABLE login_log (
  id          INTEGER PRIMARY KEY,
  site_id     INTEGER NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
  started_at  TEXT NOT NULL,
  outcome     TEXT NOT NULL,   -- 'running' | 'success' | 'mfa_handoff' | 'failed' | 'timeout' | 'origin_mismatch'
  detail      TEXT             -- never contains secrets
);
```

Recipes live as JSON files shipped with the app, referenced by `recipe_id`. Schema changes go through numbered migrations tracked by `schema_version`.

## Site recipes

Each login is a small JSON recipe: ordered steps, plus rules that say what "logged in" and "2FA prompt" look like. One generic recipe covers simple forms; awkward sites get their own.

```json
{
  "id": "generic-form",
  "timeout_ms": 30000,
  "steps": [
    { "action": "goto" },
    { "action": "wait",  "selector": "input[type=password]", "timeout_ms": 15000 },
    { "action": "fill",  "selector": "input[autocomplete=username], input[type=email], input[type=text]", "value": "{username}" },
    { "action": "fill",  "selector": "input[type=password]", "value": "{password}" },
    { "action": "click", "selector": "button[type=submit], input[type=submit]" }
  ],
  "success": { "any": [ { "url_not_contains": "/login" } ] },
  "mfa":     { "any": [ { "selector": "input[autocomplete=one-time-code]" },
                        { "text": "verification code" } ] }
}
```

- **Step actions (v1):** `goto`, `wait` (selector visible), `fill`, `click`, `select` (dropdowns such as the Proxmox realm), `press` (a key, such as Enter).
- **Placeholders:** only `{username}` and `{password}`. The password is substituted inside the Rust engine at fill time and is never written into the recipe or logs.
- **Detection rules:** `selector`, `text` (visible text contains), `url_contains`, `url_not_contains`. After submit, the engine waits until either the success or the mfa rule matches, or the timeout fires.
- **Per-site recipes to write later** (selectors must be read from each real login page, not guessed): UniFi, Proxmox VE, Proxmox Backup Server, Synology DSM, iDRAC, Huawei switch web UI, Ruckus Unleashed.

## User interface

Four screens in the Tauri window, built with Vue 3 + Vite (pending confirmation, decision 7).

| Screen | What it shows | Actions |
|---|---|---|
| Site list (home) | Sites grouped by `group_name`, search box, last-used time | Open, Edit, Delete |
| Add / edit site | Name, group, login URL, username, password (write-only), recipe, self-signed toggle, profile mode, window mode | Save, Test login |
| Session panel | Live status per open site: Launching, Filling, Waiting for 2FA, Logged in, Failed | Release window, Close window, Retry |
| Settings | Browser path (Edge), data folder, idle lock (if master password is on) | Change, Export site list (no passwords) |

The password field on the edit screen is always empty when opened. Leaving it empty keeps the stored password; typing a new one replaces it.

## Testing plan

All development testing runs against a local mock login site, so no real admin console is touched until the final pilot.

**Mock site** (`crates/mock-site`): an axum server on `https://localhost:8443` with a self-signed certificate.

| Page | Simulates | Expected app result |
|---|---|---|
| `/simple` | One-page username + password form | Logged in |
| `/two-step` | Username, Next, then password | Logged in (needs its own recipe) |
| `/spa` | React-style controlled form, like UniFi | Logged in |
| `/mfa` | Login, then a 6-digit code page (test code 123456) | Pauses with "Waiting for 2FA"; logs in after the code is typed |
| `/slow` | Form appears after 5 seconds | Waits, then logs in |
| `/wrong-password` | Always rejects | Failed, with a clear message |
| `/redirect-away` | Redirects to a different port before the form | Stops with origin mismatch; nothing filled |
| `/no-success` | Logs in but never shows the success marker | Times out, window handed over |

**Automated tests**
- **Unit tests:**
  - encrypt/decrypt round trip
  - wrong associated data fails
  - tampered ciphertext fails
  - DPAPI wrap/unwrap (`#[cfg(windows)]`)
  - origin parsing and normalization
- **Integration tests:** each mock page above, run headless with `cargo test`.
  - In the `/mfa` test, the test code types `123456` through the driver. The engine itself never does.
- **Log redaction test:** fill a known password and assert the log doesn't contain it.
- **CI:** a Windows runner, because DPAPI and Edge exist there. Cache the Playwright driver with `PLAYWRIGHT_DRIVER_CACHE_DIR`.

**Manual pilot:** after every mock test passes, one real site: UniFi at 10.100.100.18:8443.

## Milestones

Each step has a check you can see before moving on. If a step fails, stop and fix it there.

1. **M0 — Environment check.** Rust (MSVC toolchain), Tauri CLI, Node.js (only for building the Vue frontend) and WebView2 installed.
   - Done when:
     - `cargo tauri dev` opens an empty app window.
     - `spike find-edge` prints a path.
     - `cargo build` downloads the Playwright driver (or the manual fallback works).
     - `edge://policy` doesn't set `RemoteDebuggingAllowed` to false.
2. **M1 — Browser spike** (`spike/`, no Tauri). Launches Edge, attaches, logs in to a plain and a controlled form, and tests released and managed modes.
   - Done when:
     - `spike run` passes in both modes on Windows, including the isolation checks.
     - The released window stays usable after the spike exits.
3. **M2 — Mock login site.** The axum server with the eight test pages.
   - Done when each page works by hand in Edge.
4. **M3 — Vault.** SQLite schema, DPAPI-wrapped key, encrypt/decrypt, unit tests.
   - Done when all vault tests pass and the `.db` file shows no plaintext password.
5. **M4 — Tauri shell.** Site list and add/edit screens wired to the vault.
   - Done when sites survive an app restart and no command returns a password.
6. **M5 — Login engine.** Recipe runner, origin check, success detection, wired to the Open button.
   - Done when `/simple`, `/two-step`, `/spa`, `/slow`, `/wrong-password` and `/redirect-away` behave as the test table says.
7. **M6 — 2FA handoff and window modes.** MFA detection, pause, managed and released modes.
   - Done when `/mfa` and `/no-success` behave as expected in both modes.
8. **M7 — Real pilot and hardening.** UniFi recipe, log redaction check, installer build that bundles the Playwright driver.
   - Done when one-click login to UniFi works on the admin's PC from the installed app.

## Open decisions

| # | Decision | Options | Status / recommendation | Needed before |
|---|---|---|---|---|
| 1 | Browser driver | playwright-rs · chromiumoxide · Node.js sidecar | **Decided: playwright-rs**, behind the `BrowserDriver` trait | M1 |
| 2 | What "local web UI" means | A mock test site · the real device web UIs | Mock site first, real UIs only in the pilot | M2 |
| 3 | Window after login | Managed · released · per-site setting | Per-site setting, default managed | M6 |
| 4 | Master password on top of DPAPI | Yes, unlock on start · no, DPAPI only | No for the prototype, revisit before sharing the app | M3 |
| 5 | Which browser | Installed Edge · installed Chrome · downloaded Chromium | **Decided: installed Edge** | M1 |
| 6 | Session cookies | Fresh profile each launch · persistent profile per site | Fresh by default, persistent as a per-site option | M5 |
| 7 | Frontend | Vue 3 + Vite · plain HTML/JS | Vue 3 + Vite | M4 |
| 8 | Encrypt usernames too | Yes · no | No (keeps search and display simple) | M3 |
| 9 | Edge's implicit sign-in in the login window | Accept it (sync and extensions are off) · set `ImplicitSignInEnabled` = 0 for the user (also affects new profiles in the admin's own Edge) · detect it and warn | Confirmed by the third Windows run (`account_info[0].email`). Recommendation: accept it and show a warning in the session panel when the window is signed in | M5 |

## Risks

| Risk | Effect | Mitigation |
|---|---|---|
| Debug port open while a window is attached or released | Any local program could control that browser while the port is open | Edge binds it to 127.0.0.1 on a random port; default to managed mode; a released window keeps the port until it closes |
| playwright-rs is a community crate, pre-1.0 | Breaking API changes on upgrade | Pin the exact version; keep driver code behind one trait |
| Edge auto-updates; the bundled Playwright server is pinned | A newer Edge may stop working with the old driver | Upgrade the crate every few months; re-run the mock suite after Edge updates |
| Company policy blocks remote debugging | The app can't log in at all | Checked in M0; manual login still works |
| Edge signs the new profile in to the Windows account, syncs it, installs synced extensions | Extensions read the page while the password is typed; console URLs sync to the account; Edge may save the password | Isolation switches + seeded preferences; the app checks for extra tabs and profile state, and warns when a machine policy defeats isolation |
| HTTPS-inspecting proxy breaks the driver download at build time | `cargo build` can't fetch the driver | Manual driver folder + `PLAYWRIGHT_DRIVER_PATH` (documented in `spike/README.md`) |
| Driver's Node process handles passwords | One more local process sees them | Child process with no network use; never `trace`-log playwright-rs |
| A site's login page changes | Recipe stops working | Timeout falls back to handing over the window; recipe is a small JSON fix |
| Redirect or spoofed page | Password typed into the wrong site | Exact origin check before every fill |
| Accepting self-signed certificates | A man-in-the-middle on the LAN goes unnoticed | Per-site flag only; later: remember the certificate fingerprint and warn if it changes |
| Malware running as the admin's Windows user | Can unwrap the DPAPI key | Inherent to any local vault; a master password (decision 4) raises the bar |
| Persistent profiles keep session cookies on disk | Someone at the unlocked PC can reuse a session | Fresh profile by default |
