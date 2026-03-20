pub mod dialogs;
pub mod table;
pub mod toolbar;

use crate::csv_handler;
use crate::state::{Direction, State};
use dialogs::show_custom_separator_dialog;
use gtk4::{
    gio, glib, prelude::*, AboutDialog, AlertDialog, Align, ApplicationWindow, Box as GtkBox,
    Button, CssProvider, EventControllerKey, FileDialog, License, Orientation, SearchBar,
    SearchEntry,
};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use table::Table;
use toolbar::{Toolbar, CUSTOM_SEP_IDX};

// TODO: `build_ui` has grown into a large setup function (~300 lines).  Break
//       each "── … ──" block into a separate named function or module so each
//       concern (search bar, file actions, separator logic) can be read and
//       tested independently.

pub fn build_ui(app: &gtk4::Application, initial_path: Option<std::path::PathBuf>) {
    // ── CSS for search highlighting ───────────────────────────────────────────
    // TODO: move CSS into a GResource file (style.css) instead of an inline
    //       string.  That also makes it easy to support a dark-mode variant.
    let css = CssProvider::new();
    css.load_from_string(
        ".search-match { background-color: rgba(255, 220, 0, 0.55); }
         .search-match-current { background-color: rgba(255, 140, 0, 0.75); }",
    );
    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().expect("no default display"),
        &css,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Virgola")
        .default_width(900)
        .default_height(600)
        .build();

    let state: Rc<RefCell<State>> = Rc::new(RefCell::new(State::new()));
    let toolbar = Rc::new(Toolbar::new());
    let table = Rc::new(Table::new());

    // Save starts insensitive — no file is open yet.
    toolbar.save_btn.set_sensitive(false);

    // ── Search bar ────────────────────────────────────────────────────────────
    let search_entry = SearchEntry::new();
    search_entry.set_hexpand(true);
    search_entry.set_width_request(280);

    let prev_btn = Button::from_icon_name("go-up-symbolic");
    prev_btn.set_tooltip_text(Some("Previous match"));
    let next_btn = Button::from_icon_name("go-down-symbolic");
    next_btn.set_tooltip_text(Some("Next match"));

    let search_box = GtkBox::new(Orientation::Horizontal, 4);
    search_box.set_halign(Align::Center);
    search_box.append(&search_entry);
    search_box.append(&prev_btn);
    search_box.append(&next_btn);

    let search_bar = SearchBar::new();
    search_bar.set_child(Some(&search_box));
    search_bar.connect_entry(&search_entry);

    let vbox = GtkBox::new(Orientation::Vertical, 0);
    window.set_titlebar(Some(&toolbar.header_bar));
    vbox.append(&search_bar);
    vbox.append(&table.scrolled);
    window.set_child(Some(&vbox));

    // ── on_dirty: update title and re-enable save when a cell is edited ───────
    {
        let window_d = window.clone();
        let state_d = Rc::clone(&state);
        let save_btn_d = toolbar.save_btn.clone();
        table.set_on_dirty(Rc::new(move || {
            let st = state_d.borrow();
            update_title(&window_d, st.path.as_deref(), true);
            save_btn_d.set_sensitive(true);
        }));
    }

    // ── Open button ───────────────────────────────────────────────────────────
    {
        let state = Rc::clone(&state);
        let table = Rc::clone(&table);
        let toolbar_rc = Rc::clone(&toolbar);
        let window_ref = window.clone();
        let open_btn = toolbar.open_btn.clone();
        open_btn.connect_clicked(move |_| {
            let do_open = {
                let state = Rc::clone(&state);
                let table = Rc::clone(&table);
                let toolbar_rc = Rc::clone(&toolbar_rc);
                let window_ref = window_ref.clone();
                move || {
                    let state2 = Rc::clone(&state);
                    let table2 = Rc::clone(&table);
                    let window_cb = window_ref.clone();
                    let save_btn_cb = toolbar_rc.save_btn.clone();
                    let dialog = make_open_dialog();
                    dialog.open(Some(&window_ref), gio::Cancellable::NONE, move |result| {
                        if let Ok(file) = result {
                            if let Some(path) = file.path() {
                                let sep = toolbar_rc
                                    .current_separator()
                                    .unwrap_or_else(|| state2.borrow().separator);
                                load_csv_into_state(
                                    path, sep, &state2, &table2, &window_cb, &save_btn_cb,
                                );
                            }
                        }
                    });
                }
            };

            if state.borrow().dirty {
                confirm_discard(&window_ref, do_open);
            } else {
                do_open();
            }
        });
    }

    // ── Save button ───────────────────────────────────────────────────────────
    {
        let state = Rc::clone(&state);
        let window_ref = window.clone();
        let save_btn = toolbar.save_btn.clone();
        save_btn.connect_clicked(move |btn| {
            let path = state.borrow().path.clone();
            let state_c = Rc::clone(&state);
            let window_c = window_ref.clone();
            let btn_c = btn.clone();
            if let Some(path) = path {
                let st = state_c.borrow();
                match csv_handler::write_csv(&path, st.separator, &st.headers, &st.rows) {
                    Err(e) => {
                        drop(st);
                        show_message_dialog(&window_c, "Could not save file", &e.to_string());
                    }
                    Ok(()) => {
                        drop(st);
                        state_c.borrow_mut().dirty = false;
                        update_title(&window_c, Some(&path), false);
                        btn_c.set_sensitive(false);
                    }
                }
            } else {
                // No path yet — show Save As dialog, pre-filled with a name.
                let current_path = state_c.borrow().path.clone();
                let dialog = make_save_dialog(current_path.as_deref());
                dialog.save(Some(&window_ref), gio::Cancellable::NONE, move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            let st = state_c.borrow();
                            match csv_handler::write_csv(
                                &path,
                                st.separator,
                                &st.headers,
                                &st.rows,
                            ) {
                                Err(e) => {
                                    drop(st);
                                    show_message_dialog(
                                        &window_c,
                                        "Could not save file",
                                        &e.to_string(),
                                    );
                                }
                                Ok(()) => {
                                    drop(st);
                                    let mut st = state_c.borrow_mut();
                                    st.dirty = false;
                                    st.path = Some(path.clone());
                                    update_title(&window_c, Some(&path), false);
                                    btn_c.set_sensitive(false);
                                }
                            }
                        }
                    }
                });
            }
        });
    }

    // ── About button ──────────────────────────────────────────────────────────
    {
        let popover = toolbar.menu_popover.clone();
        let window_ref = window.clone();
        toolbar.about_btn.connect_clicked(move |_| {
            popover.popdown();
            let about = AboutDialog::builder()
                .program_name("Virgola")
                .version(env!("CARGO_PKG_VERSION"))
                .comments("A simple CSV viewer and editor")
                .license_type(License::MitX11)
                .transient_for(&window_ref)
                .modal(true)
                .build();
            about.present();
        });
    }

    // ── Separator dropdown ────────────────────────────────────────────────────
    {
        let state = Rc::clone(&state);
        let table = Rc::clone(&table);
        let toolbar_rc = Rc::clone(&toolbar);
        let window_ref = window.clone();
        let popover = toolbar.menu_popover.clone();
        let prev_idx: Rc<Cell<u32>> = Rc::new(Cell::new(0));
        let reverting: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let sep_dropdown = toolbar.sep_dropdown.clone();
        sep_dropdown.connect_selected_notify({
            let prev_idx = prev_idx.clone();
            let reverting = reverting.clone();
            move |dd| {
                if reverting.get() {
                    return;
                }
                match toolbar_rc.current_separator() {
                    Some(sep) => {
                        prev_idx.set(dd.selected());
                        apply_separator(
                            &state, &table, &window_ref, &toolbar_rc.save_btn, sep,
                        );
                    }
                    None => {
                        let state_c = Rc::clone(&state);
                        let table_c = Rc::clone(&table);
                        let window_c = window_ref.clone();
                        let save_btn_c = toolbar_rc.save_btn.clone();
                        let dd_c = dd.clone();
                        let prev = prev_idx.get();
                        let reverting_c = reverting.clone();
                        let prev_idx_c = prev_idx.clone();
                        let popover_c = popover.clone();
                        show_custom_separator_dialog(
                            &window_ref,
                            move |maybe_sep| match maybe_sep {
                                Some(sep) => {
                                    prev_idx_c.set(CUSTOM_SEP_IDX);
                                    apply_separator(
                                        &state_c, &table_c, &window_c, &save_btn_c, sep,
                                    );
                                }
                                None => {
                                    reverting_c.set(true);
                                    dd_c.set_selected(prev);
                                    reverting_c.set(false);
                                }
                            },
                        );
                        popover_c.popdown();
                    }
                }
            }
        });
    }

    // ── Search toggle button + Ctrl+F ─────────────────────────────────────────
    {
        let search_bar_c = search_bar.clone();
        let search_entry_c = search_entry.clone();
        toolbar.search_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                open_search_bar(&search_bar_c, &search_entry_c);
            } else {
                search_bar_c.set_search_mode(false);
            }
        });
    }
    {
        let search_bar_c = search_bar.clone();
        let search_entry_c = search_entry.clone();
        let ctrl = EventControllerKey::new();
        ctrl.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk4::gdk::Key::f
                && modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK)
            {
                open_search_bar(&search_bar_c, &search_entry_c);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(ctrl);
    }

    // Sync toggle button when search bar closes via Escape
    {
        let search_btn = toolbar.search_btn.clone();
        let state = Rc::clone(&state);
        let table = Rc::clone(&table);
        search_bar.connect_notify_local(Some("search-mode-enabled"), move |bar, _| {
            let active = bar.is_search_mode();
            search_btn.set_active(active);
            if !active {
                state.borrow_mut().clear_search();
                table.refresh_matches();
            }
        });
    }

    // ── Search entry changed (debounced 150 ms) ───────────────────────────────
    //
    // For empty queries the highlights are cleared immediately; for non-empty
    // queries the search is delayed so rapid keystrokes don't scan the full
    // dataset on every character.
    {
        let state = Rc::clone(&state);
        let table = Rc::clone(&table);
        let debounce: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
        search_entry.connect_search_changed(move |entry| {
            if let Some(id) = debounce.take() {
                id.remove();
            }
            let text = entry.text().to_string();
            let state = Rc::clone(&state);
            let table = Rc::clone(&table);
            if text.is_empty() {
                state.borrow_mut().update_search("");
                table.refresh_matches();
            } else {
                let id = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(150),
                    move || {
                        let first_row = state.borrow_mut().update_search(&text);
                        table.refresh_matches();
                        if let Some(row) = first_row {
                            table.scroll_to_match(row);
                        }
                    },
                );
                debounce.set(Some(id));
            }
        });
    }

    // ── Prev / Next match buttons ─────────────────────────────────────────────
    {
        let state = Rc::clone(&state);
        let table = Rc::clone(&table);
        prev_btn.connect_clicked(move |_| navigate(&state, &table, Direction::Prev));
    }
    {
        let state = Rc::clone(&state);
        let table = Rc::clone(&table);
        next_btn.connect_clicked(move |_| navigate(&state, &table, Direction::Next));
    }

    // ── CLI / desktop: open file passed by the caller ────────────────────────
    //
    // `initial_path` comes from the GIO `open` signal (CLI arg or "Open With"
    // from the file manager).  The window is presented after this block so any
    // error dialog already has a valid parent.
    if let Some(path) = initial_path {
        let sep = state.borrow().separator;
        load_csv_into_state(path, sep, &state, &table, &window, &toolbar.save_btn);
    }

    window.present();
}

fn navigate(state: &Rc<RefCell<State>>, table: &Rc<Table>, dir: Direction) {
    let row = state.borrow_mut().step_match(dir);
    table.refresh_matches();
    if let Some(row) = row {
        table.scroll_to_match(row);
    }
}

fn apply_separator(
    state: &Rc<RefCell<State>>,
    table: &Rc<Table>,
    window: &ApplicationWindow,
    save_btn: &Button,
    sep: u8,
) {
    let (dirty, has_file) = {
        let st = state.borrow();
        (st.dirty, st.path.is_some())
    };

    let do_apply = {
        let state = Rc::clone(state);
        let table = Rc::clone(table);
        let window = window.clone();
        let save_btn = save_btn.clone();
        move || {
            // Always store the chosen separator (also used for future opens).
            state.borrow_mut().separator = sep;
            if let Some(path) = state.borrow().path.clone() {
                load_csv_into_state(path, sep, &state, &table, &window, &save_btn);
            }
        }
    };

    if dirty && has_file {
        confirm_discard(window, do_apply);
    } else {
        do_apply();
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Load a CSV file into state and refresh the table.
///
/// Shows a warning dialog if the file has rows with inconsistent column counts,
/// and an error dialog on read failure.  All three call sites (open button,
/// separator change, CLI arg) use this function.
fn load_csv_into_state(
    path: PathBuf,
    sep: u8,
    state: &Rc<RefCell<State>>,
    table: &Rc<Table>,
    window: &ApplicationWindow,
    save_btn: &Button,
) {
    match csv_handler::read_csv(&path, sep) {
        Ok(csv) => {
            {
                let mut st = state.borrow_mut();
                st.path = Some(path.clone());
                st.separator = sep;
                st.headers = csv.headers;
                st.rows = csv.rows;
                st.dirty = false;
                st.clear_search();
            }
            table.load(Rc::clone(state));
            update_title(window, Some(&path), false);
            save_btn.set_sensitive(false);
            if csv.had_jagged_rows {
                show_message_dialog(
                    window,
                    "Inconsistent Column Count",
                    "Some rows have fewer columns than the header row.\n\
                     Missing fields are displayed as empty cells and will be \
                     padded when the file is saved.",
                );
            }
        }
        Err(e) => show_message_dialog(window, "Could not open file", &e.to_string()),
    }
}

/// Open the search bar and focus the entry.  Used by both the toggle button
/// and the Ctrl+F key handler.
fn open_search_bar(bar: &SearchBar, entry: &SearchEntry) {
    bar.set_search_mode(true);
    entry.grab_focus();
}

/// Show a modal informational/error dialog with a bold title and a detail line.
fn show_message_dialog(window: &ApplicationWindow, title: &str, detail: &str) {
    let dialog = AlertDialog::builder()
        .message(title)
        .detail(detail)
        .modal(true)
        .build();
    dialog.show(Some(window));
}

/// Update the window title to reflect the open file and dirty state.
/// Format: `"filename.csv — Virgola"` or `"filename.csv* — Virgola"`
fn update_title(window: &ApplicationWindow, path: Option<&Path>, dirty: bool) {
    let name = path
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".to_string());
    let suffix = if dirty { "*" } else { "" };
    window.set_title(Some(&format!("{name}{suffix} — Virgola")));
}

/// Show a modal "Unsaved changes — Discard?" dialog.  Calls `on_confirmed` if
/// the user chooses to discard.
fn confirm_discard<F: FnOnce() + 'static>(window: &ApplicationWindow, on_confirmed: F) {
    let dialog = AlertDialog::builder()
        .message("Unsaved Changes")
        .detail("You have unsaved changes. Discard them and continue?")
        .modal(true)
        .build();
    dialog.set_buttons(&["Cancel", "Discard"]);
    dialog.set_cancel_button(0);
    dialog.set_default_button(0);
    dialog.choose(Some(window), gio::Cancellable::NONE, move |result| {
        if result == Ok(1) {
            on_confirmed();
        }
    });
}

fn make_file_dialog(title: &str, filters: &[(&str, &[&str])]) -> FileDialog {
    let dialog = FileDialog::builder().title(title).build();
    let filter_store = gio::ListStore::new::<gtk4::FileFilter>();
    for &(name, patterns) in filters {
        let f = gtk4::FileFilter::new();
        f.set_name(Some(name));
        for pattern in patterns {
            f.add_pattern(pattern);
        }
        filter_store.append(&f);
    }
    dialog.set_filters(Some(&filter_store));
    dialog
}

fn make_open_dialog() -> FileDialog {
    make_file_dialog(
        "Open CSV",
        &[("CSV / TSV files", &["*.csv", "*.tsv"]), ("All files", &["*"])],
    )
}

/// Build a Save dialog, pre-filling the filename and folder from `current_path`
/// if one is available, otherwise defaulting to "untitled.csv".
fn make_save_dialog(current_path: Option<&Path>) -> FileDialog {
    let dialog = make_file_dialog("Save CSV", &[("CSV files", &["*.csv"])]);

    let name = current_path
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "untitled.csv".to_string());
    dialog.set_initial_name(Some(&name));

    if let Some(parent) = current_path.and_then(|p| p.parent()) {
        dialog.set_initial_folder(Some(&gio::File::for_path(parent)));
    }

    dialog
}
