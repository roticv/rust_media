//! Raw FFI bindings to libvpx.
//!
//! Generated at build time by bindgen against the system's vpx headers,
//! so there is never an ABI version mismatch.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(clippy::all)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
