//! The desktop entry point.
//!
//! Deliberately three lines. Everything the application does lives in the library, so
//! that the iOS entry point can drive exactly the same code rather than a copy of it.

// A release build must not open a console window behind the application.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    cairn_lib::run();
}
