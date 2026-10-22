#![forbid(unsafe_code)]

pub mod apply;
pub mod files;
pub mod model;
pub mod providers;
pub mod state;

pub mod analysis;
pub mod cli;
pub mod diagnostics;
pub mod evaluation;
pub mod gitlab;
pub mod observe;
pub mod privacy;
pub mod process;
pub mod proposals;
pub mod repository;
pub mod retention;
pub mod service;
pub mod telemetry;
pub mod wire;

pub mod web;
