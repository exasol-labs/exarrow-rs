//! Connection management for Exasol database connections.
//!
//! This module provides connection parameter parsing, authentication,
//! and session management functionality.
//!

pub mod auth;
pub mod params;
pub mod session;

pub use auth::{AuthenticationHandler, Credentials};
pub use params::{ConnectionBuilder, ConnectionParams};
pub use session::{Session, SessionConfig};
