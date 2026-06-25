#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
mod app;
#[cfg(target_os = "windows")]
mod clipboard;
#[cfg(target_os = "windows")]
mod ocr;
#[cfg(target_os = "windows")]
mod overlay;
#[cfg(target_os = "windows")]
mod screen;
#[cfg(target_os = "windows")]
mod window_manager;

#[cfg(target_os = "windows")]
fn main() {
    app::run();
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("QuietTools supports Windows only");
}
