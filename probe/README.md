# probe

Opens a real login page in Edge, the way NetLaunch does (own profile, isolation switches), and prints what a recipe needs: the visible fields with their attributes, the buttons and their labels, headings and alerts, frames, and which of the built-in recipes' selectors match. **It never reads what's typed into a field.**

The spec says a site's selectors must be read from its real login page, never guessed. This is how.

```powershell
cargo run -p netlaunch-probe -- https://10.100.100.18:8443/ --self-signed
```

1. Edge opens on the login page, and the first snapshot prints.
2. Log in by hand in that window. Press Enter in the terminal on each new page (the 2FA page if there is one, then the dashboard) to print it too.
3. Type `q` and Enter to finish; the window closes.
4. Send everything between the `=====` lines.

The output includes the page addresses (without query strings), page titles and button labels, but no passwords or codes. Look it over before sending if the page shows anything you'd rather not share.

Options: `--self-signed` (accept a self-signed certificate), `--browser PATH`, `--headless`, `--once` (print the first page and exit).
