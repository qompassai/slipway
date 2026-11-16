//! `slipway-tui` — the fleet-pattern terminal UI for slipway, the
//! estate's Android build / sign / verify / install / release MCP.
//!
//! Shape (fleet TUI programme DESIGN §D2, mirroring phlow-tui):
//! the UI programs against a narrow [`facade::SlipwayFacade`]
//! trait; panels are pure constructors over screen line vectors;
//! a non-TTY line mode shares the same execute path as the
//! full-screen ratatui mode. There is deliberately **no Media
//! screen**: slipway's use case has no image/video surface.
//!
//! Honesty contracts carried by this crate:
//! - Step progress comes only from the server's own reporting.
//!   A running step that has produced no events shows **elapsed
//!   time**, never a fabricated percentage (there is no percent
//!   value anywhere in the step state).
//! - `sign`, `install`, and `release` are consequential. In plan
//!   mode every step waits for approval; in auto mode build and
//!   verify run freely, but a consequential step stops for a
//!   human approval unless slipway's own certification gates are
//!   all green (the member's own gates authorize, never the TUI).
//! - If the server dies mid-pipeline, the running and remaining
//!   steps are marked *unknown*, never success.
//!
//! # Limits
//! - Log buffer: [`app::LOG_LINES_MAX`] lines.
//! - Rendered lines: [`panels::LINE_CHARS_MAX`] chars each.
//! - Stdio frames: [`facade::RESPONSE_BYTES_MAX`] bytes.

#![forbid(unsafe_code)]

pub mod app;
pub mod facade;
pub mod panels;
pub mod pipeline;
pub mod screens;
pub mod triggers;

pub use app::App;
pub use facade::{FacadeError, FakeFacade, SlipwayFacade};
pub use pipeline::{Mode, Pipeline, StepKind};
pub use triggers::{Rule, TriggerEngine};
