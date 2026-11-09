# NetLaunch

One-click logins to internal admin consoles (UniFi, routers, switches, Proxmox, Synology DSM, iDRAC) for Windows.

NetLaunch stores each login encrypted, opens an automated Edge window, fills the login form, and hands the window to you at any 2FA prompt. It never reads or types the 2FA code.

**Status:** early development. M1 (browser spike), M2 (mock login site) and M3 (vault) are built; next is M4 (Tauri shell).

- [`docs/SPEC.md`](docs/SPEC.md): the spec — goals, architecture, security rules, milestones
- [`spike/README.md`](spike/README.md): how to run the M1 browser spike on Windows
- [`crates/mock-site/README.md`](crates/mock-site/README.md): the mock login site used for testing

## Build

Requires Rust 1.88+ and Microsoft Edge. The first build downloads the Playwright driver (about 130 MB).

```powershell
cargo build
cargo test
```
