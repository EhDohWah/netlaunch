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

pub use edge::{find_edge, persistent_profile_dir};
pub use session::{LaunchOptions, Profile, Session};
