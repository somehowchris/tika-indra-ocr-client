//! Bounded Tika HTTP admission and a Linux Java process-group supervisor.
pub mod commands;
pub mod configuration;
pub mod error;
mod listener;
mod process;
mod proxy;
pub mod service;
