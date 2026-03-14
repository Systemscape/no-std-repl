#![no_std]
#![doc = include_str!("../README.md")]

mod line_buffer;
mod parts;

pub use line_buffer::{LineBuffer, Outcome};
pub use parts::Parts;
