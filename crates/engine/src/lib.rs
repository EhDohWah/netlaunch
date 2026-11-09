//! NetLaunch login engine (milestone M5).
//!
//! - [`recipe`]: the JSON recipes that say how to log in to a kind of site,
//!   and the built-in ones.
//! - [`driver`]: the [`BrowserDriver`] trait, the engine's only way to touch
//!   a browser. The engine never imports playwright-rs;
//!   `netlaunch-driver-pw` implements the trait with it.
//! - [`run`](mod@run): runs a recipe, checks the origin before every fill,
//!   and decides how the login went.
//!
//! The engine doesn't open the vault. Whoever calls [`run()`] passes a
//! closure that reads the password (`Vault::reveal_password`), and the
//! engine calls it at the moment the password field is filled.

pub mod driver;
pub mod recipe;
pub mod run;

#[cfg(test)]
mod fake;
#[cfg(test)]
mod tests;

pub use driver::{BrowserDriver, DriverError, DriverResult};
pub use recipe::{Recipe, builtin, builtin_list};
pub use run::{LoginTarget, Outcome, Progress, run};

/// The `tracing` filter for anything that runs the engine with
/// playwright-rs: `base` for everything (`info` normally), but never more
/// than `debug` for playwright-rs, because at `trace` it logs the JSON of
/// every request, including the text being typed into the password field.
///
/// Build the filter from this in code. Never take it from `RUST_LOG`.
pub fn log_directives(base: &str) -> String {
    let (base, playwright) = match base {
        "error" | "warn" | "info" | "debug" => (base, base),
        "trace" => ("trace", "debug"),
        _ => ("info", "info"),
    };
    // The more specific directive wins, so this caps playwright_rs even
    // when `base` is `trace`.
    format!("{base},playwright_rs={playwright}")
}

#[cfg(test)]
mod log_tests {
    #[test]
    fn playwright_never_gets_trace() {
        assert_eq!(super::log_directives("info"), "info,playwright_rs=info");
        assert_eq!(super::log_directives("trace"), "trace,playwright_rs=debug");
        assert_eq!(super::log_directives("anything"), "info,playwright_rs=info");
    }
}
