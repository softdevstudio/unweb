//! The stranger window's layout on Linux.
//!
//! Tauri 2.12 puts a window's child webviews in a vertical GTK box and, on
//! Linux, ignores the positions and sizes asked for, so the bar and the
//! page come out as two equal rows. Here the GTK widgets are packed by hand:
//! the bar keeps a fixed height and the page takes the rest. GTK then
//! keeps it that way on every resize.

use gtk::prelude::*;

use crate::window::BAR_HEIGHT;

/// Packs the bar and the page, on the main thread (this doesn't wait for
/// it, so it can be called from anywhere). Call once, after both were added
/// (bar first).
pub fn arrange(window: &tauri::Window) -> Result<(), String> {
    let w = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| {
                let vbox = w.default_vbox().map_err(|e| e.to_string())?;
                let kids = vbox.children();
                if kids.len() != 2 {
                    return Err(format!("expected 2 webviews, found {}", kids.len()));
                }
                kids[0].set_size_request(-1, BAR_HEIGHT as i32);
                vbox.set_child_packing(&kids[0], false, false, 0, gtk::PackType::Start);
                vbox.set_child_packing(&kids[1], true, true, 0, gtk::PackType::Start);
                vbox.show_all();
                Ok(())
            })();
            if let Err(e) = result {
                eprintln!("unweb: window layout: {e}");
            }
        })
        .map_err(|e| e.to_string())
}
