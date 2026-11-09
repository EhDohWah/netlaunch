# NetLaunch app (M4–M6)

The desktop app: a Tauri 2 window with a Vue 3 + Vite frontend, wired to the vault (`crates/vault`) and the login engine (`crates/engine`, `crates/driver-pw`).

- `src/`: the Vue frontend.
  - `App.vue` switches between the site list and the form.
  - `components/SiteList.vue` and `components/SiteForm.vue` are the two screens.
  - `api.js` calls the Rust commands.
- `src-tauri/`: the Rust side (crate `netlaunch-app`, binary `netlaunch`).
  - `main.rs` holds the commands.
  - `sites.rs` holds the site-list logic and tests.
  - `logins.rs` builds what Open needs from the vault and passes the open windows' news on (events for the panel, results for `login_log`). The windows themselves live in `crates/sessions`.
- `src/components/SessionPanel.vue` is the "Open windows" panel.
  - `capabilities/default.json` grants only Tauri's core defaults.

## Run it on Windows

Needs Rust, Node.js 20+, and the Tauri CLI (`cargo install tauri-cli --version "^2" --locked`).

```powershell
cd app
npm install
cargo tauri dev
```

**Keep the Tauri versions in step.** The Rust crate `tauri` and the npm packages `@tauri-apps/api` and `@tauri-apps/cli` must share a minor version. All three are pinned to 2.11, because 2.12 needs Rust 1.90. Upgrade them together, or `cargo tauri dev` prints a version-mismatch error.

**Use the MSVC toolchain on Windows.** With the GNU toolchain (MinGW), the build warns `.rsrc merge failure: multiple non-default manifests`. The app still runs, but MSVC is what Tauri supports, and the M7 installer should be built with it. To switch:
1. Install Visual Studio Build Tools with "Desktop development with C++".
2. Run `rustup default stable-x86_64-pc-windows-msvc`.

The first run compiles Tauri and takes a few minutes. The window opens with the site list. The vault file is created at `%LOCALAPPDATA%\NetLaunch\vault.db`, and its path is shown at the bottom of the window.

## Check by hand (M4 "done when")

1. **Add a site.** Add two or three, with groups. Use the mock site's test account if you like, for example `https://localhost:8443/simple/login` with `mock-admin` / `Mock-Pa55-2026`.
2. **Restart.** Close the window, run `cargo tauri dev` again, and check the sites are still there.
3. **Edit without a password.** Edit a site, change only its name, leave the password empty and save. The stored password is kept; from M5, Open will prove it.
4. **Delete.** Delete a site: the button asks for confirmation first.
5. **No password comes back.** Press F12 for the developer tools, go to the Network tab, and repeat steps 1–4. No response contains a password. The Rust tests check the same thing on every build.

## Check by hand (M5 "done when")

Open logs in to each mock scenario. You need two terminals in the repo folder.

1. Start the mock site, and leave it running:

   ```powershell
   cargo run -p netlaunch-mock-site
   ```

2. Start the app in the second terminal:

   ```powershell
   cd app
   cargo tauri dev
   ```

3. Add one site per row below. For every one:
   - **Login page URL:** `https://localhost:8443/<scenario>/login`
   - **Username / password:** `mock-admin` / `Mock-Pa55-2026`
   - **Self-signed certificate:** on (the mock site makes its own certificate). The form ticks it for you on `https://localhost`. If you leave it off, Open stops with "Edge does not trust the certificate…" and offers **Trust this certificate and open again**.
   - **Recipe:** Generic, except `/two-step`, which needs "Two-step form"

4. Press **Open** on each, and compare what you get:

   | Scenario | Under the site, after Open | In the Edge window |
   |---|---|---|
   | `/simple` | Logged in | "Welcome, mock-admin" |
   | `/two-step` | Logged in | "Welcome, mock-admin" |
   | `/spa` | Logged in | "Welcome, mock-admin" |
   | `/slow` | "Opening the login page…" for about 5 s, then Logged in | "Welcome, mock-admin" |
   | `/wrong-password` | Failed: the site rejected the login | The form, with "Invalid username or password" |
   | `/redirect-away` | Wrong site: the page is on `https://localhost:8444` | The lookalike form, **empty** |
   | `/mfa` (bonus) | Waiting for 2FA | The code page; type `123456` yourself |
   | `/no-success` (bonus) | Timed out after 30 s | "You are signed in, but…" |

5. **Nothing reached the lookalike.** After `/redirect-away`, open `https://localhost:8443/` in your own browser. At the bottom it must say **Input events: 0 · submits: 0**.
6. **Windows close with the app.** Close NetLaunch: every Edge window it opened closes too. Sites set to "Hand the window over to me" stay open.

If a line doesn't match, send the text under the site and what the Edge window shows.

## Check by hand (M6 "done when")

Keep the mock site running, and use the sites from the M5 check.

1. **`/mfa`, managed.** Open it. Under **Open windows** it says "Waiting for 2FA". Type `123456` in the Edge window and press Verify. Within a second the row says **Logged in · Logged in after 2FA**, and the window stays.
2. **`/mfa`, released.** Edit the site, choose "Hand the window over to me", and Open. Same as above, but once you've typed the code the row disappears from Open windows, and the Edge window stays open on its own.
3. **`/no-success`, both modes.** Managed: the row says **Yours to finish** after 30 s, and the window stays. Released: same message under the site, and the window is handed over at once.
4. **Buttons.** On a managed window, try **Release** (the row goes, the window stays), **Close** (the window closes) and **Retry** (it closes and logs in again).
5. **Closing by hand.** Close a managed Edge window yourself: its row disappears within a second or two.
6. **Signed-in warning.** On your PC, Edge signs new windows in to your Windows account. Within about a minute of opening a site, its row should show the warning "Edge signed this window in to your Windows account…". Tell me if it never appears.
7. **Exit.** Close NetLaunch with managed windows open: they close too.

**Every attempt is recorded** in the vault's `login_log` table with its outcome and message, never the password.

## Commands

| Command | Input | Returns |
|---|---|---|
| `vault_status` | none | `{ ok, path, error }` |
| `list_sites` | none | `Site[]`, which has no password field |
| `add_site` | `{ site }`, password required | the saved `Site` |
| `update_site` | `{ id, site }`, empty or null password keeps the stored one | the saved `Site` |
| `delete_site` | `{ id }` | nothing |
| `list_recipes` | none | `{ id, name }[]`, the built-in recipes |
| `open_site` | `{ id }` | `{ outcome, message, warnings, fix }`, when the login has finished. `outcome` is `success`, `mfa_handoff`, `failed`, `timeout` or `origin_mismatch`. `fix` is `"trust_certificate"` when Edge rejected the site's certificate, else null |
| `trust_certificate` | `{ id }` | the saved `Site`, with its self-signed setting ticked |
| `list_sessions` | none | `SessionView[]`: `{ site_id, site_name, state, message, warnings, window_mode }` |
| `release_window` | `{ id }` | nothing; the window is handed over |
| `close_window` | `{ id }` | nothing; the window is closed |

Events:
- `session-changed`: a `SessionView`, whenever a window's state changes. `state` is `launching`, `loading`, `filling`, `waiting` (the login running), then `waiting_for_2fa`, `logged_in`, `handed_over`, `failed` or `stopped`.
- `session-ended`: `{ site_id, why }`, where `why` is `closed`, `released`, `window_closed` or `no_window`.

On Windows the vault key is protected with DPAPI. On other systems the app builds and starts, but shows "NetLaunch runs on Windows only".
