//! Desktop entry point. All behaviour lives in the library target so it can
//! be tested without a window server.
fn main() {
    tokenbuddy_desktop_lib::run();
}
