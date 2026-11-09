# Mock login site (M2)

A fake admin console for testing NetLaunch without touching real devices. It runs two HTTPS servers on `127.0.0.1`, both with a new self-signed certificate on every start:

- **Main site:** `https://localhost:8443`, with one login scenario per path.
- **Lookalike site:** `https://localhost:8444`, where `/redirect-away` sends you. It's a copy of the login page on another port. It counts typing and submits, but never records what was typed.

The test account is fake: **`mock-admin` / `Mock-Pa55-2026`**, MFA code **`123456`**.

## Run it

```powershell
cargo run -p netlaunch-mock-site
```

Options: `--port N`, `--lookalike-port N`, `--slow-delay-ms MS`. Port `0` picks a free port. Stop it with Ctrl+C.

Edge warns about the certificate, which is called "NetLaunch mock site". Choose **Advanced**, then **Continue to localhost (unsafe)**. The lookalike site on port 8444 needs the same click the first time you reach it.

## Check each page by hand (M2 "done when")

Open `https://localhost:8443/` in Edge. The index lists every scenario. Sign in with the test account unless a step says otherwise.

| Scenario | What to do | What you should see |
|---|---|---|
| `/simple` | Sign in | "Welcome, mock-admin" at `/simple/home` |
| | Sign in with a wrong password | Red "Invalid username or password", still on `/simple/login` |
| `/two-step` | Enter the username, click **Next**, then the password | Password page at `/two-step/login/password`, then "Welcome" at `/two-step/home` |
| `/spa` | Wait for the form (about 0.3 s), sign in | "Welcome" appears without the page reloading; the address changes to `/spa/home` |
| | Reload `/spa/home` | Still signed in |
| `/mfa` | Sign in | Code page asking for the "verification code", still under `/login` |
| | Enter `000000` | "That code didn't work" |
| | Enter `123456` | "Welcome" at `/mfa/home` |
| `/slow` | Open it | "Loading the sign-in form…" for 5 seconds, then the form |
| | Sign in | "Welcome" at `/slow/home` |
| `/wrong-password` | Sign in with the right password | Always "Invalid username or password" |
| `/redirect-away` | Open it | The address changes to `https://localhost:8444/...`, and the header says "(lookalike, different port)" |
| | Type in a field, then go back to the index | Input events: 1 or more. Use **Reset counters** afterwards. NetLaunch must leave these at 0. |
| `/no-success` | Sign in | "You are signed in, but this page never shows the success marker", at `/no-success/login/signed-in` |

## Tests

```powershell
cargo test -p netlaunch-mock-site
```

The route tests (`tests/routes.rs`) call the routers directly, without TLS or a browser. In-page behavior, like the SPA's URL change, the slow form and the lookalike's counter, was checked by driving every scenario in headless Chromium. From M5, the engine's integration tests do that in `cargo test`, starting the site with `netlaunch_mock_site::serve` on free ports.

For engine tests: `GET /_mock/status` returns `{"input_events": n, "submits": n}` for the lookalike site, and `POST /_mock/reset` clears it.
