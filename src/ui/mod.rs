pub mod dialogs;
pub mod table;
pub mod toolbar;

use crate::csv_handler;
use crate::state::{Direction, State};
use adw::{
    AboutDialog, AlertDialog, ApplicationWindow, ResponseAppearance, ToolbarView, gio, glib,
    prelude::*,
};
use dialogs::show_custom_separator_dialog;
use gtk::{
    Align, Box as GtkBox, Button, CssProvider, DropDown, EventControllerKey, FileDialog, License,
    Orientation, Popover, SearchBar, SearchEntry, ToggleButton,
};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use table::Table;
use toolbar::{CUSTOM_SEP_IDX, Toolbar};

pub fn build_ui(app: &adw::Application, initial_path: Option<std::path::PathBuf>) {
    // ── CSS for search highlighting ───────────────────────────────────────────
    let css = CssProvider::new();
    css.load_from_resource("/com/github/virgola/style.css");
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("no default display"),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
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

    let toolbar_view = ToolbarView::new();
    toolbar_view.add_top_bar(&toolbar.header_bar);
    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&search_bar);
    vbox.append(&table.scrolled);
    toolbar_view.set_content(Some(&vbox));
    window.set_content(Some(&toolbar_view));

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

    // Shared state for silently syncing the separator dropdown (used by both
    // the open button and the separator dropdown handlers below).
    let sep_prev_idx: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    let sep_reverting: Rc<Cell<bool>> = Rc::new(Cell::new(false));

    setup_open_handler(
        &toolbar.open_btn,
        Rc::clone(&state),
        Rc::clone(&table),
        Rc::clone(&toolbar),
        &window,
        Rc::clone(&sep_prev_idx),
        Rc::clone(&sep_reverting),
    );
    setup_save_handler(&toolbar.save_btn, Rc::clone(&state), &window);
    setup_about_handler(&toolbar.about_btn, &toolbar.menu_popover, &window);
    setup_separator_handler(
        &toolbar.sep_dropdown,
        Rc::clone(&state),
        Rc::clone(&table),
        Rc::clone(&toolbar),
        &window,
        Rc::clone(&sep_prev_idx),
        Rc::clone(&sep_reverting),
    );
    setup_search_visibility(
        &toolbar.search_btn,
        &search_bar,
        &search_entry,
        &window,
        Rc::clone(&state),
        Rc::clone(&table),
    );
    setup_search_entry(&search_entry, Rc::clone(&state), Rc::clone(&table));
    setup_navigation_buttons(&prev_btn, &next_btn, Rc::clone(&state), Rc::clone(&table));

    // ── CLI / desktop: open file passed by the caller ────────────────────────
    //
    // `initial_path` comes from the GIO `open` signal (CLI arg or "Open With"
    // from the file manager).  The window is presented after this block so any
    // error dialog already has a valid parent.
    if let Some(path) = initial_path {
        let sep = csv_handler::detect_separator(&path);
        if let Some(idx) = Toolbar::index_of_separator(sep) {
            sep_reverting.set(true);
            toolbar.sep_dropdown.set_selected(idx);
            sep_reverting.set(false);
            sep_prev_idx.set(idx);
        }
        load_csv_into_state(path, sep, &state, &table, &window, &toolbar.save_btn);
    }

    window.present();
}

// ── Signal handler setup ──────────────────────────────────────────────────────

fn setup_open_handler(
    open_btn: &Button,
    state: Rc<RefCell<State>>,
    table: Rc<Table>,
    toolbar: Rc<Toolbar>,
    window: &ApplicationWindow,
    sep_prev_idx: Rc<Cell<u32>>,
    sep_reverting: Rc<Cell<bool>>,
) {
    let window = window.clone();
    open_btn.connect_clicked(move |_| {
        let do_open = {
            let state = Rc::clone(&state);
            let table = Rc::clone(&table);
            let toolbar = Rc::clone(&toolbar);
            let window = window.clone();
            let sep_prev_idx = Rc::clone(&sep_prev_idx);
            let sep_reverting = Rc::clone(&sep_reverting);
            move || {
                let state2 = Rc::clone(&state);
                let table2 = Rc::clone(&table);
                let window_cb = window.clone();
                let save_btn_cb = toolbar.save_btn.clone();
                let sep_prev_idx = Rc::clone(&sep_prev_idx);
                let sep_reverting = Rc::clone(&sep_reverting);
                let dialog = make_open_dialog();
                dialog.open(Some(&window), gio::Cancellable::NONE, move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        let sep = csv_handler::detect_separator(&path);
                        // Silently update the dropdown to match the
                        // detected separator so the UI stays in sync.
                        if let Some(idx) = Toolbar::index_of_separator(sep) {
                            sep_reverting.set(true);
                            toolbar.sep_dropdown.set_selected(idx);
                            sep_reverting.set(false);
                            sep_prev_idx.set(idx);
                        }
                        load_csv_into_state(path, sep, &state2, &table2, &window_cb, &save_btn_cb);
                    }
                });
            }
        };

        if state.borrow().dirty {
            confirm_discard(&window, do_open);
        } else {
            do_open();
        }
    });
}

fn setup_save_handler(save_btn: &Button, state: Rc<RefCell<State>>, window: &ApplicationWindow) {
    let window = window.clone();
    save_btn.connect_clicked(move |btn| {
        let path = state.borrow().path.clone();
        let state_c = Rc::clone(&state);
        let window_c = window.clone();
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
            dialog.save(Some(&window), gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    let st = state_c.borrow();
                    match csv_handler::write_csv(&path, st.separator, &st.headers, &st.rows) {
                        Err(e) => {
                            drop(st);
                            show_message_dialog(&window_c, "Could not save file", &e.to_string());
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
            });
        }
    });
}

fn setup_about_handler(about_btn: &Button, menu_popover: &Popover, window: &ApplicationWindow) {
    let popover = menu_popover.clone();
    let window = window.clone();
    about_btn.connect_clicked(move |_| {
        popover.popdown();
        let about = AboutDialog::builder()
            .application_name("Virgola")
            .developer_name("Matte23")
            .version(env!("CARGO_PKG_VERSION"))
            .comments("A simple CSV viewer and editor")
            .license_type(License::Gpl30)
            .build();
        about.present(Some(&window));
    });
}

fn setup_separator_handler(
    sep_dropdown: &DropDown,
    state: Rc<RefCell<State>>,
    table: Rc<Table>,
    toolbar: Rc<Toolbar>,
    window: &ApplicationWindow,
    sep_prev_idx: Rc<Cell<u32>>,
    sep_reverting: Rc<Cell<bool>>,
) {
    let window = window.clone();
    let popover = toolbar.menu_popover.clone();
    sep_dropdown.connect_selected_notify({
        let prev_idx = sep_prev_idx.clone();
        let reverting = sep_reverting.clone();
        move |dd| {
            if reverting.get() {
                return;
            }
            match toolbar.current_separator() {
                Some(sep) => {
                    prev_idx.set(dd.selected());
                    apply_separator(&state, &table, &window, &toolbar.save_btn, sep);
                }
                None => {
                    let state_c = Rc::clone(&state);
                    let table_c = Rc::clone(&table);
                    let window_c = window.clone();
                    let save_btn_c = toolbar.save_btn.clone();
                    let dd_c = dd.clone();
                    let prev = prev_idx.get();
                    let reverting_c = reverting.clone();
                    let prev_idx_c = prev_idx.clone();
                    let popover_c = popover.clone();
                    show_custom_separator_dialog(&window, move |maybe_sep| match maybe_sep {
                        Some(sep) => {
                            prev_idx_c.set(CUSTOM_SEP_IDX);
                            apply_separator(&state_c, &table_c, &window_c, &save_btn_c, sep);
                        }
                        None => {
                            reverting_c.set(true);
                            dd_c.set_selected(prev);
                            reverting_c.set(false);
                        }
                    });
                    popover_c.popdown();
                }
            }
        }
    });
}

/// Wires the search toggle button, the Ctrl+F keyboard shortcut, and the
/// search bar's close (Escape) signal so they all stay in sync.
fn setup_search_visibility(
    search_btn: &ToggleButton,
    search_bar: &SearchBar,
    search_entry: &SearchEntry,
    window: &ApplicationWindow,
    state: Rc<RefCell<State>>,
    table: Rc<Table>,
) {
    // Toggle button → open/close bar
    {
        let search_bar_c = search_bar.clone();
        let search_entry_c = search_entry.clone();
        search_btn.connect_toggled(move |btn| {
            if btn.is_active() {
                open_search_bar(&search_bar_c, &search_entry_c);
            } else {
                search_bar_c.set_search_mode(false);
            }
        });
    }

    // Ctrl+F → open bar
    {
        let search_bar_c = search_bar.clone();
        let search_entry_c = search_entry.clone();
        let ctrl = EventControllerKey::new();
        ctrl.connect_key_pressed(move |_, key, _, modifiers| {
            if key == gtk::gdk::Key::f && modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                open_search_bar(&search_bar_c, &search_entry_c);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(ctrl);
    }

    // Bar closed via Escape → sync toggle button and clear highlights
    {
        let search_btn = search_btn.clone();
        search_bar.connect_notify_local(Some("search-mode-enabled"), move |bar, _| {
            let active = bar.is_search_mode();
            search_btn.set_active(active);
            if !active {
                state.borrow_mut().clear_search();
                table.refresh_matches(&state.borrow());
            }
        });
    }
}

/// Wires the search entry's `search-changed` signal with a 150 ms debounce.
///
/// Empty queries clear highlights immediately; non-empty queries are delayed
/// so rapid keystrokes don't scan the full dataset on every character.
fn setup_search_entry(entry: &SearchEntry, state: Rc<RefCell<State>>, table: Rc<Table>) {
    let debounce: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    entry.connect_search_changed(move |entry| {
        if let Some(id) = debounce.take() {
            id.remove();
        }
        let text = entry.text().to_string();
        let state = Rc::clone(&state);
        let table = Rc::clone(&table);
        if text.is_empty() {
            state.borrow_mut().update_search("");
            table.refresh_matches(&state.borrow());
        } else {
            let debounce2 = Rc::clone(&debounce);
            let id =
                glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                    debounce2.take(); // clear before glib auto-removes the source
                    let first_row = state.borrow_mut().update_search(&text);
                    table.refresh_matches(&state.borrow());
                    if let Some(row) = first_row {
                        table.scroll_to_match(row);
                    }
                });
            debounce.set(Some(id));
        }
    });
}

fn setup_navigation_buttons(
    prev_btn: &Button,
    next_btn: &Button,
    state: Rc<RefCell<State>>,
    table: Rc<Table>,
) {
    {
        let state = Rc::clone(&state);
        let table = Rc::clone(&table);
        prev_btn.connect_clicked(move |_| navigate(&state, &table, Direction::Prev));
    }
    {
        next_btn.connect_clicked(move |_| navigate(&state, &table, Direction::Next));
    }
}

// ── Action helpers ────────────────────────────────────────────────────────────

fn navigate(state: &Rc<RefCell<State>>, table: &Rc<Table>, dir: Direction) {
    let row = state.borrow_mut().step_match(dir);
    table.refresh_matches(&state.borrow());
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
            let path = state.borrow().path.clone();
            if let Some(path) = path {
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

// ── UI helpers ────────────────────────────────────────────────────────────────

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
    let dialog = AlertDialog::builder().heading(title).body(detail).build();
    dialog.add_response("ok", "OK");
    dialog.present(Some(window));
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
        .heading("Unsaved Changes")
        .body("You have unsaved changes. Discard them and continue?")
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("discard", "Discard");
    dialog.set_response_appearance("discard", ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.choose(Some(window), gio::Cancellable::NONE, move |response| {
        if response == "discard" {
            on_confirmed();
        }
    });
}

fn make_file_dialog(title: &str, filters: &[(&str, &[&str])]) -> FileDialog {
    let dialog = FileDialog::builder().title(title).build();
    let filter_store = gio::ListStore::new::<gtk::FileFilter>();
    for &(name, patterns) in filters {
        let f = gtk::FileFilter::new();
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
        &[
            ("CSV / TSV files", &["*.csv", "*.tsv"]),
            ("All files", &["*"]),
        ],
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
