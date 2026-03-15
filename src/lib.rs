//! Iron Babel - A cross-protocol API gateway
//! 
//! This crate provides a flexible and extensible API gateway that can translate
//! between different communication protocols, enabling seamless service communication
//! in heterogeneous environments.

pub mod admin;
pub mod config;
pub mod core;
pub mod error;
pub mod gateway;
pub mod protocols;
pub mod schema;
pub mod transform;
pub mod utils;

pub use error::{Error, Result}; 