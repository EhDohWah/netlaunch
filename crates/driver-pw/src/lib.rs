//! NetLaunch's [`BrowserDriver`](netlaunch_engine::BrowserDriver) on
//! playwright-rs.
//!
//! - [`edge`]: find the installed Edge, launch it with the isolation
//!   switches and its own profile folder (ported from the M1 spike).
//! - [`session`]: attach over CDP, drive the login tab, then release or
//!   close the window.
//!
//! This is the only crate that imports playwright-rs.

pub mod edge;
pub mod session;

pub use edge::{BrowserKind, bundled_driver_dir, find_browser, find_edge, persistent_profile_dir, sweep_fresh_profiles, use_bundled_driver};
pub use session::{LaunchOptions, Profile, Session};
