//! Desktop entry point. All behaviour lives in the library target so it can
//! be tested without a window server.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    tokenbuddy_desktop_lib::run();
}
