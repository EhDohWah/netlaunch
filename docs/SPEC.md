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
2. Wrap it with DPAPI (`CryptProtectData`, current-user scope, `CRYPTPROTECT_UI_FORBIDDEN`, with fixed app entropy) and store the wrapped blob in `app_meta`.
   - `app_meta` also holds `key_protector` (`dpapi-user`) and `key_version` (`1`).
   - Opening a vault with a different kind of protector fails with a clear error rather than garbage.
3. On start: unwrap with DPAPI and hold the key in memory using the `secrecy` / `zeroize` crates (wiped on drop).
4. **No master password** (decision 4). The vault takes the key protector as a parameter (`KeyProtector` trait), so one can be added later without changing the vault.
5. Tests on machines without DPAPI use `InsecureTestProtector`. It is only available in tests and behind the `insecure-test-protector` feature, which the app never enables.

### Encrypting a password

- Algorithm: XChaCha20-Poly1305 (`chacha20poly1305` crate), with a fresh random 24-byte nonce on every save.
- Associated data: `"sites.password:" + site id`. A ciphertext copied onto another row, or reused for a different field later, fails to decrypt.

### Rules the code must follow

- **No command returns a password.** The frontend can send a password when saving, but nothing ever sends one back to it.
- **Origin check before every fill.** Before each `fill` (not once per login), the engine checks that the scheme, host and port of the frame holding the field match the saved origin exactly.
  - Origins are normalized with the `url` crate (lowercase host, default ports, IPv6 brackets), so `https://x` and `https://x:443` match.
  - A redirect to any other origin, or a password field inside a frame from another origin, stops the login.
  - **How (M5):** the driver's selectors never look inside iframes, so any field the engine can fill is in the main frame. The engine compares the main frame's URL, which playwright-rs tracks from the browser's navigation events (the page's own scripts can't fake it), with the origin of the saved login URL. A mismatch stops the login before the password is even read from the vault.
- **Self-signed exceptions stay per site.** Ignoring certificate errors applies only to the Edge process opened for that site, and only when the site has the self-signed flag on.
  - Most consoles on the admin's own network have self-signed certificates, so the add-site form ticks the flag for an `https` URL on a private address (localhost, `.local`, `.lan`, `.internal`, `.home.arpa`, 10/8, 172.16/12, 192.168/16, 169.254/16, IPv6 local). The admin can untick it, and it's never ticked for a public address.
  - When Edge rejects a certificate, the login stops with nothing typed and the window closes. The site offers "Trust this certificate and open again", which ticks the flag and retries.
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
  key   TEXT PRIMARY KEY,   -- 'schema_version', 'wrapped_data_key', 'key_protector', 'key_version'
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

Implementation notes (`crates/vault`, M3):
- SQLite is bundled into the app (`rusqlite` with `bundled`), so nothing has to be installed on the PC.
- **Pragmas:**
  - `foreign_keys = ON`, which the `login_log` cascade needs.
  - `secure_delete = ON`, so replaced and deleted ciphertexts are zeroed in the file.
  - `journal_mode = DELETE`, so there's one file on disk.
- **Timestamps** are ISO 8601 UTC with milliseconds, made by SQLite.
- **A newer schema is refused.** Opening a file with a `schema_version` newer than the build knows fails rather than risking damage.
- **Only the engine sees passwords.** `Vault::reveal_password` is the only way to get a password back, and only the login engine may call it. `Site` has no password field.

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
                        { "text": "verification code" } ] },
  "failure": { "any": [ { "selector": "[role=alert]" } ] }
}
```

- **Step actions (v1):** `goto`, `wait` (selector visible), `fill`, `click`, `select` (dropdowns such as the Proxmox realm), `press` (a key, such as Enter).
  - `goto` takes no URL: it always opens the site's saved login URL, so a recipe can't send the browser anywhere else. It must be the first step, and only the first.
  - Every action uses the **first visible** element that matches. Generic selectors can match several inputs, and hidden decoy fields are skipped.
- **Placeholders:** only `{username}` and `{password}`, each as the whole value. The password is substituted inside the Rust engine at fill time and is never written into the recipe or logs. Any other `{...}` is rejected when the recipe loads.
- **Detection rules:** `selector`, `text` (visible text contains, ignoring case), `url_contains`, `url_not_contains`.
  - After the last step, the engine checks the page every 250 ms until a rule matches or `timeout_ms` runs out. It checks `success` first, then `mfa`, then `failure`: a dashboard may have an alert banner, and a 2FA page may mention an error.
  - **URL rules only count on the site's own origin.** Otherwise "the URL no longer contains `/login`" would also be true of a browser error page, or of a redirect to another site.
  - **`failure` (added in M5)** says the site rejected the login. A failure rule that already matched before the last step (a banner that's always on the login page) is ignored afterwards.
- **Outcomes:** logged in; 2FA hand-over; rejected (`failed`); timed out; origin mismatch; an untrusted certificate (logged as `failed`); or an error (the page didn't load, the window was closed).
  - Network failures get a plain sentence from the browser's `net::ERR_...` code (refused, address not found, no answer, TLS problems), never Playwright's stack trace. Every outcome except a closed window ends with the window brought to the front.
- **Built-in recipes** are JSON files in `crates/engine/recipes/`, compiled into the app. They're checked when they load, and a site can only be saved with a recipe that exists.
  - `generic-form`: one page with username and password.
  - `two-step-form`: username, Next, then the password on a second page.
- **Per-site recipes to write later** (selectors must be read from each real login page, not guessed): UniFi, Proxmox VE, Proxmox Backup Server, Synology DSM, iDRAC, Huawei switch web UI, Ruckus Unleashed.

## User interface

Four screens in the Tauri window, built with Vue 3 + Vite (pending confirmation, decision 7).

| Screen | What it shows | Actions |
|---|---|---|
| Site list (home) | Sites grouped by `group_name`, search box, last-used time | Open, Edit, Delete |
| Add / edit site | Name, group, login URL, username, password (write-only), recipe, self-signed toggle, profile mode, window mode | Save, Test login |
| Session panel | Live status per open site: Launching, Filling, Waiting for 2FA, Logged in, Failed. A warning when the window is signed in to the Windows account (decision 9) | Release window, Close window, Retry |
| Settings | Browser path (Edge), data folder, idle lock (if master password is on) | Change, Export site list (no passwords) |

The password field on the edit screen is always empty when opened. Leaving it empty keeps the stored password; typing a new one replaces it.

## Testing plan

All development testing runs against a local mock login site, so no real admin console is touched until the final pilot.

**Mock site** (`crates/mock-site`, built in M2): two HTTPS servers on 127.0.0.1 with a self-signed certificate made at start-up.
- **Main site:** `https://localhost:8443`.
- **Lookalike site:** `https://localhost:8444`, used by `/redirect-away`.

Run it with `cargo run -p netlaunch-mock-site`. Engine tests start it in-process on free ports with `netlaunch_mock_site::serve`.

- **Fake test account:** `mock-admin` / `Mock-Pa55-2026`, MFA code `123456`. The index page lists the account and every scenario.
- **URL shape:**
  - Each scenario's login flow lives under `/<scenario>/login…`, and its dashboard under `/<scenario>/home` with a `#welcome` marker. So the generic recipe's `url_not_contains: "/login"` rule works everywhere.
  - Rejected logins show `<p id="error" role="alert">`.
- **The lookalike counts, never records.** It counts input events and submits, never the values. `GET /_mock/status` on the main site returns `{"input_events": n, "submits": n}`, which must stay 0 after `/redirect-away`.


| Page | Simulates | Expected app result |
|---|---|---|
| `/simple` | One-page username + password form | Logged in |
| `/two-step` | Username, Next, then password on a second page | Logged in (needs its own recipe) |
| `/spa` | React-style controlled form, like UniFi. Ignores `.value` set without input events; switches to `/spa/home` with `pushState`, without a page load | Logged in |
| `/mfa` | Login, then a 6-digit code page (test code 123456) | Pauses with "Waiting for 2FA"; logs in after the code is typed |
| `/slow` | Form added by script after 5 seconds (`?delay=ms` or `--slow-delay-ms` to change) | Waits, then logs in |
| `/wrong-password` | Always rejects | Failed, with a clear message |
| `/redirect-away` | Redirects to the lookalike site (another port, so another origin) before the form | Stops with origin mismatch; lookalike counters stay 0 |
| `/no-success` | Logs in, but the URL keeps `/login` and there's no `#welcome` | Times out, window handed over |

"Failed, with a clear message" needs a way to recognize a rejected login, so M5 added a `failure` rule to recipes (the generic ones use `[role=alert]`). Without it, a wrong password would look like a timeout.

**Automated tests**
- **Unit tests:**
  - encrypt/decrypt round trip
  - wrong associated data fails
  - tampered ciphertext fails
  - DPAPI wrap/unwrap (`#[cfg(windows)]`)
  - origin parsing and normalization
- **Integration tests** (`crates/driver-pw/tests/mock_site.rs`): each mock page above, run headless with `cargo test`.
  - In the `/mfa` test, the test code types `123456` through the driver. The engine itself never does.
  - They use `NETLAUNCH_TEST_BROWSER` if set, else the installed Edge, else a Chrome or Chromium in the usual Linux places. With no browser they skip, unless `NETLAUNCH_REQUIRE_BROWSER=1`, which CI sets.
- **Log redaction test:** fill a known password and assert the log doesn't contain it. Two copies:
  - the engine's own log, with a fake browser, at `trace`;
  - a real login, with the app's log filter at its most verbose (`trace`, capped at `debug` for playwright-rs). Raising playwright-rs to `trace` makes this test fail, so it does catch the leak.
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
   - Status: built. 13 route tests pass, and all eight scenarios pass when driven in headless Chromium. The by-hand check in Edge on Windows is pending.
4. **M3 — Vault.** SQLite schema, DPAPI-wrapped key, encrypt/decrypt, unit tests.
   - Done when all vault tests pass and the `.db` file shows no plaintext password.
   - Status: built. 28 tests pass on Linux, and 31 on Windows (cross-compiled, run under Wine), including DPAPI and a DPAPI vault whose file holds neither the passwords nor the raw key. The GitHub Actions run on `windows-latest` is the real-Windows check.
5. **M4 — Tauri shell.** Site list and add/edit screens wired to the vault.
   - Done when sites survive an app restart and no command returns a password.
   - Status: built (`app/`).
     - 7 command tests pass, including one that serializes every command's output and checks it holds no password.
     - The UI flows pass in Chromium against a stand-in backend.
     - The app cross-compiles for Windows.
     - **Done** (Nov 9, 2026): the app ran on the owner's PC and the site list survived a restart.
   - **Not in M4:** Open stays disabled until M5, and Test login and the Settings screen are left for later milestones.
6. **M5 — Login engine.** Recipe runner, origin check, success detection, wired to the Open button.
   - Done when `/simple`, `/two-step`, `/spa`, `/slow`, `/wrong-password` and `/redirect-away` behave as the test table says.
   - Status: built.
     - **Engine** (`crates/engine`): recipes, the `BrowserDriver` trait and the runner. 24 tests against a fake browser, including the origin check before every fill and the log redaction test.
     - **Driver** (`crates/driver-pw`): Edge launch, isolation and close, ported from the spike.
     - **Mock-site tests:** all six pages above pass in headless Chromium, and so do `/mfa` (hand-over) and `/no-success` (time-out), a self-signed site without the flag, and the redaction test with a real browser. CI runs them on Windows with Edge and on Linux with Chrome.
     - **App:** Open works. It shows each stage as it happens, then the result, and records each attempt in `login_log`. Managed windows close with the app; released ones are handed over after the login.
     - **Done** (Nov 9, 2026): Open works on the owner's PC, including the self-signed certificate flow.
   - **Not in M5:** watching a 2FA hand-over until it succeeds, the session panel (release, close, retry buttons) and the signed-in warning (decision 9) are M6.
7. **M6 — 2FA handoff and window modes.** MFA detection, pause, managed and released modes.
   - Done when `/mfa` and `/no-success` behave as expected in both modes.
   - Status: built (`crates/sessions`, the Open windows panel in the app).
     - **2FA:** after the hand-over, a watcher checks the page every second (only the `success` rule; it never types). When it matches, the status flips to Logged in and `login_log` is updated to `success`. After 10 minutes without it, the window is the admin's (`timeout`).
     - **Managed** windows stay until Close, Release, the admin closing the window (noticed within a second; a fresh profile is then deleted) or NetLaunch exiting.
     - **Released** windows are handed over once the login is finished: at once, or after the 2FA.
     - **Session panel** ("Open windows"): live status, Release (managed only), Close, Retry. A site with an open window can't be opened again until it's closed.
     - **Signed-in warning** (decision 9): for two minutes after launch the watcher reads the profile's preferences for an account address (`account_info[…].email`, only the path is kept) and adds a warning. Edge writes the file a few seconds after signing in.
     - **Tests** (`crates/sessions/tests/windows.rs`, real browser): `/mfa` and `/no-success` in both modes, a released window's Edge still running afterwards, an unfinished 2FA handed over, a window closed by hand, a site opened twice, and closing everything on exit.
     - Left for the owner: the by-hand check in `app/README.md`. The signed-in warning can only be seen on a PC where Edge signs in (the owner's).
8. **M7 — Real pilot and hardening.** UniFi recipe, log redaction check, installer build that bundles the Playwright driver.
   - Done when one-click login to UniFi works on the admin's PC from the installed app.

## Open decisions

| # | Decision | Options | Status / recommendation | Needed before |
|---|---|---|---|---|
| 1 | Browser driver | playwright-rs · chromiumoxide · Node.js sidecar | **Decided: playwright-rs**, behind the `BrowserDriver` trait | M1 |
| 2 | What "local web UI" means | A mock test site · the real device web UIs | Mock site first, real UIs only in the pilot | M2 |
| 3 | Window after login | Managed · released · per-site setting | **Built (M6):** per-site setting, default managed; Release turns a managed window into a released one | M6 |
| 4 | Master password on top of DPAPI | Yes, unlock on start · no, DPAPI only | **Decided: no** for the prototype; revisit before sharing the app | M3 |
| 5 | Which browser | Installed Edge · installed Chrome · downloaded Chromium | **Decided: installed Edge** | M1 |
| 6 | Session cookies | Fresh profile each launch · persistent profile per site | **Built (M5):** fresh by default, persistent as a per-site option (`%LOCALAPPDATA%\NetLaunch\profiles\<site id>`) | M5 |
| 7 | Frontend | Vue 3 + Vite · plain HTML/JS | **Decided: Vue 3 + Vite** (JavaScript, no UI library) | M4 |
| 8 | Encrypt usernames too | Yes · no | **Decided: no** (keeps search and display simple) | M3 |
| 9 | Edge's implicit sign-in in the login window | Accept it (sync and extensions are off) · set `ImplicitSignInEnabled` = 0 for the user (also affects new profiles in the admin's own Edge) · detect it and warn | **Decided: accept and warn.** Confirmed by the third Windows run (`account_info[0].email`). The session panel warns when the window is signed in; no policy is changed. **Built in M6** | M6 |

## Risks

| Risk | Effect | Mitigation |
|---|---|---|
| Debug port open while a window is attached or released | Any local program could control that browser while the port is open | Edge binds it to 127.0.0.1 on a random port; default to managed mode; a released window keeps the port until it closes |
| playwright-rs is a community crate, pre-1.0 | Breaking API changes on upgrade | Pin the exact version; keep driver code behind one trait |
| Edge auto-updates; the bundled Playwright server is pinned | A newer Edge may stop working with the old driver | Upgrade the crate every few months; re-run the mock suite after Edge updates |
| Company policy blocks remote debugging | The app can't log in at all | Checked in M0; manual login still works |
| Edge signs the new profile in to the Windows account, syncs it, installs synced extensions | Extensions read the page while the password is typed; console URLs sync to the account; Edge may save the password | Isolation switches + seeded preferences; the app checks for extra tabs and profile state, and warns when a machine policy defeats isolation |
| Building with the Windows GNU toolchain (MinGW) instead of MSVC | The linker reports "multiple non-default manifests" and keeps only one of the app's Windows manifests; `WebView2Loader.dll` must ship next to the exe | Works for development; switch to the MSVC toolchain before the M7 installer, which is what Tauri supports |
| HTTPS-inspecting proxy breaks the driver download at build time | `cargo build` can't fetch the driver | Manual driver folder + `PLAYWRIGHT_DRIVER_PATH` (documented in `spike/README.md`) |
| Driver's Node process handles passwords | One more local process sees them | Child process with no network use; never `trace`-log playwright-rs |
| A site's login page changes | Recipe stops working | Timeout falls back to handing over the window; recipe is a small JSON fix |
| Redirect or spoofed page | Password typed into the wrong site | Exact origin check before every fill |
| The page navigates between the origin check and the fill | Text typed into the new page | The window is milliseconds (check, then one driver call). The check runs before the username too, so a redirect that happens before the form is caught before anything is typed. Playwright can't make the check and the fill one step |
| Accepting self-signed certificates | A man-in-the-middle on the LAN goes unnoticed | Per-site flag only; later: remember the certificate fingerprint and warn if it changes |
| Malware running as the admin's Windows user | Can unwrap the DPAPI key | Inherent to any local vault; a master password (decision 4) raises the bar |
| Persistent profiles keep session cookies on disk | Someone at the unlocked PC can reuse a session | Fresh profile by default |
