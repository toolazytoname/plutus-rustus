//! Plutus-Rustus — funded-address key-space collider library.
//!
//! The CLI in `main.rs` is a thin operator surface over these modules. Private
//! keys are written only to a local findings file; notifications never include
//! secret material.

pub mod bloom;
pub mod config;
pub mod db;
pub mod engine;
pub mod hit;
pub mod notify;
pub mod status;

pub use config::Config;
pub use db::Db;
