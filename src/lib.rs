//! Slipway library: the domain modules behind the `slipway` MCP
//! server binary. Kept separate from the binary so the gate suites
//! test the domain (config firewall, artifact census, certification
//! batteries, signing-manifest machinery) without a transport.

#![forbid(unsafe_code)]

pub mod android;
pub mod artifact;
pub mod certify;
pub mod client;
pub mod config;
pub mod exec;
pub mod guard;
pub mod model;
pub mod receipt;
pub mod release;
pub mod server;
pub mod signing;
pub mod tools;
