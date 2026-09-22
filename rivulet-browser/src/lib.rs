//! Native browser-source backend for Rivulet.
//!
//! **Status: spike / proof of concept (M6.9, issue #215).** This crate
//! evaluates embedding a real platform webview ([`wry`])
//! as a [`BrowserSourceBackend`], replacing the deterministic
//! [`SyntheticBrowserBackend`] from `rivulet-core` where real pixels are
//! needed.
//!
//! The spike targets Windows (WebView2) first; on other platforms the native
//! backend does not exist yet and the crate exposes no code, so building the
//! workspace in CI does not require Linux/macOS webview system packages.
#![cfg_attr(not(windows), allow(dead_code))]

pub use rivulet_core::browser_source::{BrowserInput, BrowserSourceError};

#[cfg(windows)]
mod wry_backend;

#[cfg(windows)]
pub use wry_backend::WryBrowserBackend;
