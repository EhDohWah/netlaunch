# M1 browser spike

A throwaway command-line program that answers the M1 questions before any app code is written:

1. Can NetLaunch launch the installed Edge itself and attach to it with playwright-rs?
2. Does `Locator::fill` work on a plain form **and** on a React-style controlled form?
3. Does the log stay free of the password at the log level we plan to use?
4. **Released mode:** after disconnecting, does Edge stay open and usable, even after the program exits?
5. **Managed mode:** when we close it, is every Edge process for that profile gone?
6. **Isolation:** is the new profile free of your Windows account, sync, extensions and password saving?

The program starts its own small web server on `127.0.0.1` with the two test forms (`pages/`), so nothing on the network is touched. The test account is fake.

## Run it on Windows

From the repository root in PowerShell:

```powershell
cargo build -p netlaunch-spike
.\target\debug\spike.exe find-edge
```

The first build downloads the Playwright driver (about 130 MB). If it fails, see "If the driver download fails" below.

Each run spends 15 seconds watching the window after login (`--observe`), so sign-in, sync and extension tabs have time to show up.

### Run 1: managed mode

```powershell
.\target\debug\spike.exe run --mode managed
```

An Edge window opens and logs in. Wait for "press Enter", then press Enter: Edge quits cleanly and the spike reads the profile it left behind.

- **Check:** the window has no favorites bar, no profile picture in the toolbar, and only the spike's tab.

### Run 2: released mode

```powershell
.\target\debug\spike.exe run
```

An Edge window opens, logs in, then the program exits.

- **Check:** the Edge window is still open and you can click around in it.
- Then close it yourself.

### Run 3: without isolation (for comparison)

```powershell
.\target\debug\spike.exe run --mode managed --no-isolation
```

This is how the first Windows run launched Edge. On your PC, expect it to FAIL some of the isolation checks: an account signed in, user extensions, extra tabs, password saving. That shows the checks catch the problem.

### What to send back

Each run ends with a block that starts with `===== M1 spike summary`. Copy those blocks, plus what you saw for each **Check** above.

## Options

```
spike run [--mode released|managed] [--release-via close|shutdown]
          [--close-after SECONDS] [--observe SECONDS] [--no-isolation]
          [--log-level info|debug|trace]
          [--browser PATH] [--browser-arg ARG]... [--headless]
          [--driver-path DIR]
```

- **`--log-level trace` is expected to FAIL** the "password absent from log" check. It shows why the real app must never log playwright-rs at `trace`. Only the fake test password is involved.
- Each run's log is written to `%TEMP%\netlaunch-spike\spike-<time>.log`.
- Released runs leave their profile folder in `%TEMP%\netlaunch-spike\` so the open window keeps working. Delete the folder after closing the window.

## If the driver download fails

The build prints `Failed to assemble Playwright driver` when something blocks the download, for example an office proxy that inspects HTTPS. To assemble the driver folder by hand:

1. Make a folder, e.g. `C:\pw-driver`.
2. Download `https://registry.npmjs.org/playwright-core/-/playwright-core-1.64.0.tgz` and extract it into that folder with Windows' built-in `tar -xzf playwright-core-1.64.0.tgz -C C:\pw-driver`. You should get `C:\pw-driver\package\cli.js`.
3. Download `https://nodejs.org/dist/v24.21.0/node-v24.21.0-win-x64.zip` and copy `node.exe` from it to `C:\pw-driver\node.exe`.
4. Run with `--driver-path C:\pw-driver`.

The `1.64.0` and `24.21.0` versions match playwright-rs 0.20.1. Use the same versions, or the driver and the crate may disagree.

## Results so far

### Windows 11, Edge 155.0.4283.62 (Nov 4, 2026, first version of the spike)

Every check passed in released and managed mode: debug port, attach, both form logins, password absent from the `info` log, Edge surviving a release, and managed close leaving nothing behind.

The screenshots showed what the checks didn't cover. The "fresh" profile wasn't isolated:
- It was signed in to the Windows account, with the account's favorites bar and profile picture.
- Synced extensions (a translator, Wappalyzer) were installed and opened their own tabs.
- Edge showed its password key icon after the login.

The spike now launches Edge with isolation switches and seeded preferences (`edge::isolation_args`, `edge::seed_preferences`), and checks for all of this. A Windows run of the new version is pending.

### Linux container, Chromium 153, headless, hand-assembled driver

| Check | Result |
|---|---|
| Debug port, attach, both form logins | PASS |
| Password absent from log at `info` / `debug` | PASS |
| Password absent from log at `trace` | **FAIL, as expected**: the password appears in the log |
| Released via `Browser::close` or driver shutdown | PASS: browser alive, logged-in tab still open after the spike exited |
| Managed: clean quit via CDP `Browser.close`, profile deletable | PASS |
| Isolation checks (extra tabs, account, extensions, password saving) | PASS with isolation. Without it only "password saving off" fails, because the container has no account to sync. |
