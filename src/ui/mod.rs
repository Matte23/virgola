pub mod dialogs;
pub mod sidebar;
pub mod table;
pub mod toolbar;

use crate::csv_handler;
use crate::state::{Direction, State};
use adw::{
    AboutDialog, AlertDialog, ApplicationWindow, OverlaySplitView, ResponseAppearance, ToolbarView,
    gio, glib, prelude::*,
};
use dialogs::show_custom_separator_dialog;
use gtk::{
    Align, Box as GtkBox, Button, CssProvider, EventControllerKey, FileDialog, License,
    Orientation, PackType, SearchBar, SearchEntry,
};
use sidebar::{CUSTOM_SEP_IDX, Sidebar};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use table::Table;
use toolbar::Toolbar;

// ── Shared UI context ─────────────────────────────────────────────────────────

/// Bundles every `Rc`-wrapped handle that signal handlers share.
///
/// All fields are cheap to clone (GTK objects use GObject reference counting;
/// `Rc<_>` increments a counter).  Passing one `ctx.clone()` into each
/// closure replaces 5–7 individual `Rc::clone` calls at each call site.
struct UiContext {
    state: Rc<RefCell<State>>,
    table: Rc<Table>,
    toolbar: Rc<Toolbar>,
    sidebar: Rc<Sidebar>,
    window: ApplicationWindow,
    sep_prev_idx: Rc<Cell<u32>>,
    sep_reverting: Rc<Cell<bool>>,
    enc_reverting: Rc<Cell<bool>>,
}

impl Clone for UiContext {
    fn clone(&self) -> Self {
        Self {
            state: Rc::clone(&self.state),
            table: Rc::clone(&self.table),
            toolbar: Rc::clone(&self.toolbar),
            sidebar: Rc::clone(&self.sidebar),
            window: self.window.clone(),
            sep_prev_idx: Rc::clone(&self.sep_prev_idx),
            sep_reverting: Rc::clone(&self.sep_reverting),
            enc_reverting: Rc::clone(&self.enc_reverting),
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn build_ui(app: &adw::Application, initial_path: Option<PathBuf>, extra_files: usize) {
    // ── CSS ───────────────────────────────────────────────────────────────────
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

    let toolbar = Rc::new(Toolbar::new());
    let table = Rc::new(Table::new());
    let sidebar = Rc::new(Sidebar::new());

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

    // ── Layout: OverlaySplitView with right sidebar ───────────────────────────
    let vbox = GtkBox::new(Orientation::Vertical, 0);
    vbox.append(&search_bar);
    vbox.append(&table.scrolled);

    let split_view = OverlaySplitView::new();
    split_view.set_sidebar_position(PackType::End);
    split_view.set_show_sidebar(false);
    split_view.set_min_sidebar_width(220.0);
    split_view.set_content(Some(&vbox));
    split_view.set_sidebar(Some(&sidebar.container));

    let toolbar_view = ToolbarView::new();
    toolbar_view.add_top_bar(&toolbar.header_bar);
    toolbar_view.set_content(Some(&split_view));
    window.set_content(Some(&toolbar_view));

    let ctx = UiContext {
        state: Rc::new(RefCell::new(State::new())),
        table,
        toolbar,
        sidebar,
        window,
        sep_prev_idx: Rc::new(Cell::new(0)),
        sep_reverting: Rc::new(Cell::new(false)),
        enc_reverting: Rc::new(Cell::new(false)),
    };

    // ── on_dirty: update title and re-enable save when a cell is edited ───────
    {
        let c = ctx.clone();
        ctx.table.set_on_dirty(Rc::new(move || {
            let st = c.state.borrow();
            update_title(&c.toolbar.window_title, st.path.as_deref(), true);
            c.toolbar.save_btn.set_sensitive(true);
        }));
    }

    // ── Sidebar toggle button ↔ split view ───────────────────────────────────
    {
        let sv = split_view.clone();
        ctx.toolbar.sidebar_btn.connect_toggled(move |btn| {
            sv.set_show_sidebar(btn.is_active());
        });
    }
    {
        let btn = ctx.toolbar.sidebar_btn.clone();
        split_view.connect_show_sidebar_notify(move |sv| {
            btn.set_active(sv.shows_sidebar());
        });
    }

    setup_open_handler(ctx.clone());
    setup_save_handler(ctx.clone());
    setup_about_handler(ctx.clone());
    setup_separator_handler(ctx.clone());
    setup_encoding_handler(ctx.clone());
    setup_search_visibility(ctx.clone(), &search_bar, &search_entry);
    setup_search_entry(&search_entry, ctx.clone());
    setup_navigation_buttons(&prev_btn, &next_btn, ctx.clone());

    // ── CLI / desktop: open file passed by the caller ────────────────────────
    //
    // `initial_path` comes from the GIO `open` signal (CLI arg or "Open With"
    // from the file manager).  The window is presented after this block so any
    // error dialog already has a valid parent.
    if let Some(path) = initial_path {
        let sep = csv_handler::detect_separator(&path);
        if let Some(idx) = Sidebar::index_of_separator(sep) {
            ctx.sep_reverting.set(true);
            ctx.sidebar.sep_row.set_selected(idx);
            ctx.sep_reverting.set(false);
            ctx.sep_prev_idx.set(idx);
        }
        load_csv_into_state(path, sep, None, &ctx);
    }

    ctx.window.present();

    if extra_files > 0 {
        show_message_dialog(
            &ctx.window,
            "Only One File at a Time",
            &format!(
                "{extra_files} additional {} ignored. \
                 Virgola opens one file per window.",
                if extra_files == 1 {
                    "file was"
                } else {
                    "files were"
                }
            ),
        );
    }
}

// ── Signal handler setup ──────────────────────────────────────────────────────

fn setup_open_handler(ctx: UiContext) {
    let open_btn = ctx.toolbar.open_btn.clone();
    open_btn.connect_clicked(move |_| {
        let do_open = {
            let ctx = ctx.clone();
            move || {
                let ctx2 = ctx.clone();
                let dialog = make_open_dialog();
                dialog.open(Some(&ctx.window), gio::Cancellable::NONE, move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        let ctx2 = ctx2.clone();
                        glib::spawn_future_local(async move {
                            let path_bg = path.clone();
                            let sep = gio::spawn_blocking(move || {
                                csv_handler::detect_separator(&path_bg)
                            })
                            .await
                            .expect("blocking task panicked");
                            // Silently update the separator dropdown to match
                            // the detected separator so the UI stays in sync.
                            if let Some(idx) = Sidebar::index_of_separator(sep) {
                                ctx2.sep_reverting.set(true);
                                ctx2.sidebar.sep_row.set_selected(idx);
                                ctx2.sep_reverting.set(false);
                                ctx2.sep_prev_idx.set(idx);
                            }
                            // Encoding is auto-detected inside load_csv_into_state
                            // and the enc_dropdown is synced there.
                            load_csv_into_state(path, sep, None, &ctx2);
                        });
                    }
                });
            }
        };

        if ctx.state.borrow().dirty {
            confirm_discard(&ctx.window, do_open);
        } else {
            do_open();
        }
    });
}

fn setup_save_handler(ctx: UiContext) {
    let save_btn = ctx.toolbar.save_btn.clone();
    save_btn.connect_clicked(move |btn| {
        let path = ctx.state.borrow().path.clone();
        let btn = btn.clone();
        if let Some(path) = path {
            let (sep, headers, rows, encoding, encoding_bom) = {
                let st = ctx.state.borrow();
                (
                    st.separator,
                    st.headers.clone(),
                    st.rows.clone(),
                    st.encoding,
                    st.encoding_bom,
                )
            };
            let ctx2 = ctx.clone();
            glib::spawn_future_local(async move {
                let result = gio::spawn_blocking(move || {
                    csv_handler::write_csv(&path, sep, &headers, &rows, encoding, encoding_bom)
                        .map(|()| path)
                })
                .await
                .expect("blocking task panicked");
                match result {
                    Ok(path) => {
                        ctx2.state.borrow_mut().dirty = false;
                        update_title(&ctx2.toolbar.window_title, Some(&path), false);
                        btn.set_sensitive(false);
                    }
                    Err(e) => {
                        show_message_dialog(&ctx2.window, "Could not save file", &e.to_string())
                    }
                }
            });
        } else {
            // No path yet — show Save As dialog, pre-filled with a name.
            let current_path = ctx.state.borrow().path.clone();
            let dialog = make_save_dialog(current_path.as_deref());
            let ctx2 = ctx.clone();
            dialog.save(Some(&ctx.window), gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    let (sep, headers, rows, encoding, encoding_bom) = {
                        let st = ctx2.state.borrow();
                        (
                            st.separator,
                            st.headers.clone(),
                            st.rows.clone(),
                            st.encoding,
                            st.encoding_bom,
                        )
                    };
                    let ctx3 = ctx2.clone();
                    glib::spawn_future_local(async move {
                        let result = gio::spawn_blocking(move || {
                            csv_handler::write_csv(
                                &path,
                                sep,
                                &headers,
                                &rows,
                                encoding,
                                encoding_bom,
                            )
                            .map(|()| path)
                        })
                        .await
                        .expect("blocking task panicked");
                        match result {
                            Ok(path) => {
                                let mut st = ctx3.state.borrow_mut();
                                st.dirty = false;
                                st.path = Some(path.clone());
                                drop(st);
                                update_title(&ctx3.toolbar.window_title, Some(&path), false);
                                btn.set_sensitive(false);
                            }
                            Err(e) => show_message_dialog(
                                &ctx3.window,
                                "Could not save file",
                                &e.to_string(),
                            ),
                        }
                    });
                }
            });
        }
    });
}

fn setup_about_handler(ctx: UiContext) {
    let action = gio::SimpleAction::new("about", None);
    let window = ctx.window.clone();
    action.connect_activate(move |_, _| {
        let about = AboutDialog::builder()
            .application_name("Virgola")
            .developer_name("Matte23")
            .version(env!("CARGO_PKG_VERSION"))
            .comments("A simple CSV viewer and editor")
            .license_type(License::Gpl30)
            .issue_url("https://github.com/Matte23/virgola/issues")
            .copyright("© 2026 Matteo Schiff")
            .website("https://github.com/Matte23/virgola")
            .build();
        about.present(Some(&window));
    });
    ctx.window.add_action(&action);
}

fn setup_separator_handler(ctx: UiContext) {
    let sep_dropdown = ctx.sidebar.sep_row.clone();
    sep_dropdown.connect_selected_notify({
        let prev_idx = Rc::clone(&ctx.sep_prev_idx);
        let reverting = Rc::clone(&ctx.sep_reverting);
        move |dd| {
            if reverting.get() {
                return;
            }
            match ctx.sidebar.current_separator() {
                Some(sep) => {
                    prev_idx.set(dd.selected());
                    apply_separator(&ctx, sep);
                }
                None => {
                    let ctx2 = ctx.clone();
                    let dd_c = dd.clone();
                    let prev = prev_idx.get();
                    let reverting_c = reverting.clone();
                    let prev_idx_c = prev_idx.clone();
                    show_custom_separator_dialog(&ctx.window, move |maybe_sep| match maybe_sep {
                        Some(sep) => {
                            prev_idx_c.set(CUSTOM_SEP_IDX);
                            apply_separator(&ctx2, sep);
                        }
                        None => {
                            reverting_c.set(true);
                            dd_c.set_selected(prev);
                            reverting_c.set(false);
                        }
                    });
                }
            }
        }
    });
}

fn setup_encoding_handler(ctx: UiContext) {
    let enc_dropdown = ctx.sidebar.enc_row.clone();
    enc_dropdown.connect_selected_notify(move |_| {
        if ctx.enc_reverting.get() {
            return;
        }
        let (enc, bom) = ctx.sidebar.current_encoding();
        apply_encoding(&ctx, enc, bom);
    });
}

/// Wires the search toggle button, the Ctrl+F keyboard shortcut, and the
/// search bar's close (Escape) signal so they all stay in sync.
fn setup_search_visibility(ctx: UiContext, search_bar: &SearchBar, search_entry: &SearchEntry) {
    // Toggle button → open/close bar
    {
        let search_bar_c = search_bar.clone();
        let search_entry_c = search_entry.clone();
        ctx.toolbar.search_btn.connect_toggled(move |btn| {
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
        ctx.window.add_controller(ctrl);
    }

    // Bar closed via Escape → sync toggle button and clear highlights
    {
        let search_bar = search_bar.clone();
        search_bar.connect_notify_local(Some("search-mode-enabled"), move |bar, _| {
            let active = bar.is_search_mode();
            ctx.toolbar.search_btn.set_active(active);
            if !active {
                ctx.state.borrow_mut().clear_search();
                ctx.table.refresh_matches(&ctx.state.borrow());
            }
        });
    }
}

/// Wires the search entry's `search-changed` signal with a 150 ms debounce.
///
/// Empty queries clear highlights immediately; non-empty queries are delayed
/// so rapid keystrokes don't scan the full dataset on every character.
fn setup_search_entry(entry: &SearchEntry, ctx: UiContext) {
    let debounce: Rc<Cell<Option<glib::SourceId>>> = Rc::new(Cell::new(None));
    entry.connect_search_changed(move |entry| {
        if let Some(id) = debounce.take() {
            id.remove();
        }
        let text = entry.text().to_string();
        if text.is_empty() {
            ctx.state.borrow_mut().update_search("");
            ctx.table.refresh_matches(&ctx.state.borrow());
        } else {
            let ctx2 = ctx.clone();
            let debounce2 = Rc::clone(&debounce);
            let id =
                glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                    debounce2.take(); // clear before glib auto-removes the source
                    let first_row = ctx2.state.borrow_mut().update_search(&text);
                    ctx2.table.refresh_matches(&ctx2.state.borrow());
                    if let Some(row) = first_row {
                        ctx2.table.scroll_to_match(row);
                    }
                });
            debounce.set(Some(id));
        }
    });
}

fn setup_navigation_buttons(prev_btn: &Button, next_btn: &Button, ctx: UiContext) {
    {
        let ctx2 = ctx.clone();
        prev_btn.connect_clicked(move |_| navigate(&ctx2, Direction::Prev));
    }
    next_btn.connect_clicked(move |_| navigate(&ctx, Direction::Next));
}

// ── Action helpers ────────────────────────────────────────────────────────────

fn navigate(ctx: &UiContext, dir: Direction) {
    let row = ctx.state.borrow_mut().step_match(dir);
    ctx.table.refresh_matches(&ctx.state.borrow());
    if let Some(row) = row {
        ctx.table.scroll_to_match(row);
    }
}

fn apply_separator(ctx: &UiContext, sep: u8) {
    let (dirty, has_file) = {
        let st = ctx.state.borrow();
        (st.dirty, st.path.is_some())
    };

    let do_apply = {
        let ctx = ctx.clone();
        move || {
            // Store the chosen separator (also used for future opens).
            ctx.state.borrow_mut().separator = sep;
            // Keep the current encoding — only the separator changed.
            let (path, enc, bom) = {
                let st = ctx.state.borrow();
                (st.path.clone(), st.encoding, st.encoding_bom)
            };
            if let Some(path) = path {
                load_csv_into_state(path, sep, Some((enc, bom)), &ctx);
            }
        }
    };

    if dirty && has_file {
        confirm_discard(&ctx.window, do_apply);
    } else {
        do_apply();
    }
}

fn apply_encoding(ctx: &UiContext, enc: &'static encoding_rs::Encoding, bom: bool) {
    let (dirty, has_file) = {
        let st = ctx.state.borrow();
        (st.dirty, st.path.is_some())
    };

    let do_apply = {
        let ctx = ctx.clone();
        move || {
            let (path, sep) = {
                let st = ctx.state.borrow();
                (st.path.clone(), st.separator)
            };
            if let Some(path) = path {
                // Re-read the file with the explicitly chosen encoding.
                load_csv_into_state(path, sep, Some((enc, bom)), &ctx);
            } else {
                // No file open yet — just store the encoding preference.
                let mut st = ctx.state.borrow_mut();
                st.encoding = enc;
                st.encoding_bom = bom;
            }
        }
    };

    if dirty && has_file {
        confirm_discard(&ctx.window, do_apply);
    } else {
        do_apply();
    }
}

// ── UI helpers ────────────────────────────────────────────────────────────────

/// Load a CSV file into state and refresh the table.
///
/// Pass `encoding_hint = None` to auto-detect the encoding (the usual case
/// when first opening a file).  Pass `Some((enc, bom))` to force a specific
/// encoding — used when re-reading after a separator or encoding change.
///
/// Shows a warning dialog if the file has rows with inconsistent column counts,
/// and an error dialog on read failure.
fn load_csv_into_state(
    path: PathBuf,
    sep: u8,
    encoding_hint: Option<(&'static encoding_rs::Encoding, bool)>,
    ctx: &UiContext,
) {
    let ctx = ctx.clone();
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || {
            csv_handler::read_csv(&path, sep, encoding_hint).map(|csv| (path, csv))
        })
        .await
        .expect("blocking task panicked");

        match result {
            Ok((path, csv)) => {
                let had_jagged = csv.had_jagged_rows;
                {
                    let mut st = ctx.state.borrow_mut();
                    st.path = Some(path.clone());
                    st.separator = sep;
                    st.encoding = csv.encoding;
                    st.encoding_bom = csv.encoding_bom;
                    st.headers = csv.headers;
                    st.rows = csv.rows;
                    st.dirty = false;
                    st.clear_search();
                }
                // Sync encoding dropdown to the (possibly auto-detected) encoding.
                {
                    let st = ctx.state.borrow();
                    let idx = Sidebar::index_of_encoding(st.encoding, st.encoding_bom);
                    ctx.enc_reverting.set(true);
                    ctx.sidebar.enc_row.set_selected(idx);
                    ctx.enc_reverting.set(false);
                }
                ctx.table.load(Rc::clone(&ctx.state));
                update_title(&ctx.toolbar.window_title, Some(&path), false);
                ctx.toolbar.save_btn.set_sensitive(false);
                if had_jagged {
                    show_message_dialog(
                        &ctx.window,
                        "Inconsistent Column Count",
                        "Some rows have fewer columns than the header row.\n\
                         Missing fields are displayed as empty cells and will be \
                         padded when the file is saved.",
                    );
                }
            }
            Err(e) => show_message_dialog(&ctx.window, "Could not open file", &e.to_string()),
        }
    });
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
/// Title = filename (with `*` suffix when dirty), subtitle = parent directory.
fn update_title(title_widget: &adw::WindowTitle, path: Option<&Path>, dirty: bool) {
    let name = path
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Untitled".to_string());
    let suffix = if dirty { "*" } else { "" };
    title_widget.set_title(&format!("{name}{suffix}"));
    let subtitle = path
        .and_then(|p| p.parent())
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    title_widget.set_subtitle(&subtitle);
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
