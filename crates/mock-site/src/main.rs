//! `mock-site`: runs the mock login site until Ctrl+C.
//!
//!     mock-site [--port 8443] [--lookalike-port 8444] [--slow-delay-ms 5000]

use std::process::ExitCode;

use netlaunch_mock_site::{MFA_CODE, MockConfig, PASSWORD, SCENARIOS, USERNAME, serve};

const USAGE: &str = "\
NetLaunch mock login site

USAGE:
    mock-site [--port N] [--lookalike-port N] [--slow-delay-ms MS]

OPTIONS:
    --port N            Main site port (default 8443; 0 = any free port)
    --lookalike-port N  Lookalike site port (default 8444)
    --slow-delay-ms MS  How long /slow waits before showing its form (default 5000)

Both sites listen on 127.0.0.1 only, with a new self-signed certificate on
every start. Edge will warn about it: choose Advanced, then Continue.";

fn parse(mut args: impl Iterator<Item = String>) -> Result<MockConfig, String> {
    let mut config = MockConfig::default();
    while let Some(raw) = args.next() {
        // Accept both `--name value` and `--name=value`.
        let (name, mut inline) = match raw.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (raw, None),
        };
        if name == "--help" || name == "-h" {
            return Err(String::new());
        }
        let value = inline.take().or_else(|| args.next()).ok_or_else(|| format!("{name} needs a value"))?;
        let number = |v: &str| v.parse::<u64>().map_err(|_| format!("{name}: '{v}' is not a number"));
        match name.as_str() {
            "--port" => config.port = u16::try_from(number(&value)?).map_err(|_| "--port is too large")?,
            "--lookalike-port" => {
                config.lookalike_port = u16::try_from(number(&value)?).map_err(|_| "--lookalike-port is too large")?
            }
            "--slow-delay-ms" => config.slow_delay_ms = number(&value)?,
            other => return Err(format!("unknown option '{other}'")),
        }
    }
    Ok(config)
}

#[tokio::main]
async fn main() -> ExitCode {
    let config = match parse(std::env::args().skip(1)) {
        Ok(config) => config,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("error: {message}\n");
            }
            eprintln!("{USAGE}");
            return if message.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE };
        }
    };

    let mock = match serve(config).await {
        Ok(mock) => mock,
        Err(e) => {
            eprintln!("error: could not start the mock site: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("NetLaunch mock site");
    println!("  main:      {}/", mock.base_url);
    println!("  lookalike: {}/", mock.lookalike_url);
    println!("  account:   {USERNAME} / {PASSWORD}   MFA code: {MFA_CODE}");
    println!();
    for (path, simulates, _) in SCENARIOS {
        println!("  {}/{path}/login  {simulates}", mock.base_url);
    }
    println!("\nSelf-signed certificate: Edge will warn; choose Advanced, then Continue.");
    println!("Press Ctrl+C to stop.");

    let _ = tokio::signal::ctrl_c().await;
    mock.shutdown();
    ExitCode::SUCCESS
}
