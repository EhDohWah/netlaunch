# NetLaunch app (M4)

The desktop app: a Tauri 2 window with a Vue 3 + Vite frontend, wired to the vault (`crates/vault`).

- `src/`: the Vue frontend.
  - `App.vue` switches between the site list and the form.
  - `components/SiteList.vue` and `components/SiteForm.vue` are the two screens.
  - `api.js` calls the Rust commands.
- `src-tauri/`: the Rust side (crate `netlaunch-app`, binary `netlaunch`).
  - `main.rs` holds the commands.
  - `sites.rs` holds their logic and tests.
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

**Open** stays disabled until the login engine arrives in M5.

## Commands

| Command | Input | Returns |
|---|---|---|
| `vault_status` | none | `{ ok, path, error }` |
| `list_sites` | none | `Site[]`, which has no password field |
| `add_site` | `{ site }`, password required | the saved `Site` |
| `update_site` | `{ id, site }`, empty or null password keeps the stored one | the saved `Site` |
| `delete_site` | `{ id }` | nothing |

On Windows the vault key is protected with DPAPI. On other systems the app builds and starts, but shows "NetLaunch runs on Windows only".
