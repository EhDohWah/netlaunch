# M1 browser spike

A throwaway command-line program that answers the M1 questions before any app code is written:

1. Can NetLaunch launch the installed Edge itself and attach to it with playwright-rs?
2. Does `Locator::fill` work on a plain form **and** on a React-style controlled form?
3. Does the log stay free of the password at the log level we plan to use?
4. **Released mode:** after disconnecting, does Edge stay open and usable, even after the program exits?
5. **Managed mode:** when we close it, is every Edge process for that profile gone?

The program starts its own small web server on `127.0.0.1` with the two test forms (`pages/`), so nothing on the network is touched. The test account is fake.

## Run it on Windows

From the repository root in PowerShell:

```powershell
cargo build -p netlaunch-spike
.\target\debug\spike.exe find-edge
```

The first build downloads the Playwright driver (about 130 MB). If it fails, see "If the driver download fails" below.

### Run 1: released mode

```powershell
.\target\debug\spike.exe run
```

An Edge window opens, logs in to both forms, then the program exits.

- **Check:** the Edge window is still open and you can click around in it.
- Then close it yourself.

### Run 2: managed mode

```powershell
.\target\debug\spike.exe run --mode managed
```

An Edge window opens and logs in. Press Enter in the terminal and the window should close.

- **Check:** in Task Manager (Details tab), no `msedge.exe` with `netlaunch-spike` in its command line remains. Add the "Command line" column if needed.

### Run 3: the other release method

```powershell
.\target\debug\spike.exe run --release-via shutdown
```

This releases by only stopping the Playwright driver. Check the window stays open, as in run 1.

### What to send back

Each run ends with a block that starts with `===== M1 spike summary`. Copy those blocks, plus what you saw for each **Check** above.

## Options

```
spike run [--mode released|managed] [--release-via close|shutdown]
          [--close-after SECONDS] [--log-level info|debug|trace]
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

Run in a Linux container against Chromium 153, headless, with a hand-assembled driver:

| Check | Result |
|---|---|
| Debug port read from `DevToolsActivePort` | PASS |
| `connect_over_cdp` to a browser we launched | PASS (1 default context, existing tab reused) |
| Plain form login | PASS |
| Controlled form login | PASS |
| Password absent from log at `info` / `debug` | PASS |
| Password absent from log at `trace` | **FAIL, as expected**: the password appears in the log |
| Released via `Browser::close` | PASS: browser alive, logged-in tab still open after the spike exited |
| Released via driver shutdown only | PASS |
| Managed close: endpoint gone, profile folder deletable | PASS (5 runs in a row) |

Still to confirm on Windows with the installed Edge: all of the above, plus a visible (not headless) window staying usable after the spike exits.
