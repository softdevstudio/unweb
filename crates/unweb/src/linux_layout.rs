//! Window layout on Linux.
//!
//! Tauri 2.12 puts a window's child webviews in a vertical GTK box and, on
//! Linux, ignores the positions and sizes asked for. So the widgets are
//! packed by hand: the bar keeps a fixed height at the top, and below it a
//! GTK grid holds one box per stranger, its strip (fixed height) above its
//! page. GTK then keeps it that way on every resize.
//!
//! Tauri appends each new webview to the window's box, so a stranger's two
//! widgets are the unplaced ones that arrive after it is added (strip
//! first, then page). They are remembered by stranger, on the main thread.

use std::cell::RefCell;
use std::collections::HashMap;

use gtk::prelude::*;

use crate::window::{grid, BAR_HEIGHT, STRIP_HEIGHT};

thread_local! {
    /// Each stranger's window and its (strip, page) widgets.
    static TILES: RefCell<HashMap<String, (String, gtk::Widget, gtk::Widget)>> = RefCell::new(HashMap::new());
    /// Each window's grid.
    static GRIDS: RefCell<HashMap<String, gtk::Grid>> = RefCell::new(HashMap::new());
}

fn detach(w: &gtk::Widget) {
    if let Some(parent) = w.parent() {
        if let Ok(c) = parent.downcast::<gtk::Container>() {
            c.remove(w);
        }
    }
}

/// Packs the bar and the strangers `tiles` (in order) of `window`. Runs on
/// the main thread and doesn't wait for it, so it can be called from
/// anywhere. Call it after every stranger added or closed.
pub fn arrange(window: &tauri::Window, tiles: Vec<String>) -> Result<(), String> {
    let w = window.clone();
    window
        .run_on_main_thread(move || {
            if let Err(e) = pack(&w, &tiles) {
                eprintln!("unweb: window layout: {e}");
            }
        })
        .map_err(|e| e.to_string())
}

fn pack(window: &tauri::Window, tiles: &[String]) -> Result<(), String> {
    let label = window.label().to_string();
    let vbox = window.default_vbox().map_err(|e| e.to_string())?;
    let kids = vbox.children();
    let bar = kids.first().ok_or("the window has no bar")?.clone();
    bar.set_size_request(-1, BAR_HEIGHT as i32);
    vbox.set_child_packing(&bar, false, false, 0, gtk::PackType::Start);

    let grid_widget = GRIDS.with(|g| {
        g.borrow_mut()
            .entry(label.clone())
            .or_insert_with(|| {
                let grid = gtk::Grid::new();
                grid.set_row_homogeneous(true);
                grid.set_column_homogeneous(true);
                grid.set_hexpand(true);
                grid.set_vexpand(true);
                grid
            })
            .clone()
    });
    if grid_widget.parent().is_none() {
        vbox.pack_start(&grid_widget, true, true, 0);
    }

    // Webviews added since the last time: the window box's children after
    // the bar that aren't the grid, two per new stranger.
    let fresh: Vec<gtk::Widget> = kids
        .iter()
        .skip(1)
        .filter(|k| !k.is::<gtk::Grid>())
        .cloned()
        .collect();
    let mut fresh = fresh.into_iter();
    TILES.with(|t| {
        let mut t = t.borrow_mut();
        for tile in tiles {
            if !t.contains_key(tile) {
                if let (Some(strip), Some(page)) = (fresh.next(), fresh.next()) {
                    t.insert(tile.clone(), (label.clone(), strip, page));
                }
            }
        }
    });

    // Empty the grid and place every stranger again, in order.
    for child in grid_widget.children() {
        if let Ok(b) = child.clone().downcast::<gtk::Box>() {
            for w in b.children() {
                b.remove(&w);
            }
        }
        grid_widget.remove(&child);
    }
    let (cols, _) = grid(tiles.len());
    TILES.with(|t| {
        let mut t = t.borrow_mut();
        for (i, tile) in tiles.iter().enumerate() {
            let Some((_, strip, page)) = t.get(tile).cloned() else {
                continue;
            };
            detach(&strip);
            detach(&page);
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 0);
            strip.set_size_request(-1, STRIP_HEIGHT as i32);
            cell.pack_start(&strip, false, false, 0);
            cell.pack_start(&page, true, true, 0);
            grid_widget.attach(&cell, (i % cols) as i32, (i / cols) as i32, 1, 1);
        }
        // Strangers of this window that closed: forget their widgets.
        t.retain(|k, (w, strip, page)| {
            let keep = *w != label || tiles.contains(k);
            if !keep {
                detach(strip);
                detach(page);
            }
            keep
        });
    });
    vbox.show_all();
    Ok(())
}

/// Forgets a window's grid once the window is gone.
pub fn forget_window(window: &tauri::Window) {
    let label = window.label().to_string();
    let _ = window.run_on_main_thread(move || {
        GRIDS.with(|g| {
            g.borrow_mut().remove(&label);
        });
        TILES.with(|t| t.borrow_mut().retain(|_, (w, _, _)| *w != label));
    });
}
