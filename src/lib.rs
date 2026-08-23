//! RMOPL Battle — deterministic 2D physics brawler.
//!
//! The simulation is fixed-point and reproducible bit-for-bit across platforms.
//! Rendering may use floats; nothing that feeds the simulation may.

pub mod constants;
pub mod math;
