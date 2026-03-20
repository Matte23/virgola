use crate::state::State;
use glib::BoxedAnyObject;
use gtk4::{
    Box as GtkBox, ColumnView, ColumnViewColumn, EditableLabel, ListItem, NoSelection, Orientation,
    ScrolledWindow, SignalListItemFactory, gio, glib, prelude::*,
};
use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::rc::Rc;

pub struct Table {
    pub scrolled: ScrolledWindow,
    pub column_view: ColumnView,
    current_store: RefCell<Option<gio::ListStore>>,
    // Called whenever a cell edit sets state.dirty = true.
    on_dirty: RefCell<Option<Rc<dyn Fn()>>>,
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

impl Table {
    pub fn new() -> Self {
        let column_view = ColumnView::new(None::<NoSelection>);
        column_view.set_show_column_separators(true);
        column_view.set_show_row_separators(true);

        let scrolled = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Automatic)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .child(&column_view)
            .build();

        Self {
            scrolled,
            column_view,
            current_store: RefCell::new(None),
            on_dirty: RefCell::new(None),
        }
    }

    pub fn set_on_dirty(&self, f: Rc<dyn Fn()>) {
        *self.on_dirty.borrow_mut() = Some(f);
    }

    /// Force all visible cells to rebind so search highlights are updated.
    ///
    /// Uses `ListStore::splice` to replace items with themselves, which emits
    /// `items-changed(0, n, n)` and triggers unbind→bind on all visible cells
    /// WITHOUT swapping the model — so the scroll adjustment is never touched
    /// and the `gtk_adjustment_configure` assertion cannot fire.
    pub fn refresh_matches(&self) {
        if let Some(store) = self.current_store.borrow().as_ref() {
            let n = store.n_items();
            if n > 0 {
                let items: Vec<glib::Object> = (0..n).filter_map(|i| store.item(i)).collect();
                store.splice(0, n, &items);
            }
        }
    }

    /// Scroll the ColumnView to make `row` visible.
    ///
    /// Deferred to the next idle cycle so that the layout pass triggered by
    /// `refresh_matches` completes before we touch the scroll adjustment
    /// (prevents the `gtk_adjustment_configure` critical assertion).
    pub fn scroll_to_match(&self, row: usize) {
        let cv = self.column_view.clone();
        glib::idle_add_local_once(move || {
            cv.scroll_to(row as u32, None, gtk4::ListScrollFlags::FOCUS, None);
        });
    }

    pub fn load(&self, state: Rc<RefCell<State>>) {
        // TODO: `load()` tears down and rebuilds ALL columns every time it is
        //       called, even when only cell values changed (e.g. after a save).
        //       Separate "set schema" (add/remove columns) from "set data"
        //       (replace the ListStore items) so only the necessary part is
        //       rebuilt.

        // Remove existing columns
        let cols: Vec<ColumnViewColumn> = (0..self.column_view.columns().n_items())
            .filter_map(|i| {
                self.column_view
                    .columns()
                    .item(i)?
                    .downcast::<ColumnViewColumn>()
                    .ok()
            })
            .collect();
        for col in cols {
            self.column_view.remove_column(&col);
        }

        let st = state.borrow();
        if st.headers.is_empty() {
            self.column_view.set_model(None::<NoSelection>.as_ref());
            *self.current_store.borrow_mut() = None;
            return;
        }

        let store = gio::ListStore::new::<BoxedAnyObject>();
        for row in &st.rows {
            store.append(&BoxedAnyObject::new(row.clone()));
        }

        let headers = st.headers.clone();
        let ncols = headers.len();

        // Compute a content-aware width for each column: sample the header text
        // plus the first 50 data rows, multiply max char count by an approximate
        // pixel width, and clamp to a sensible range.
        const SAMPLE_ROWS: usize = 50;
        const CHAR_PX: i32 = 8; // rough average for the default GTK4 font
        const PAD_PX: i32 = 16; // cell padding
        const MIN_PX: i32 = 60;
        const MAX_PX: i32 = 300;
        let col_widths: Vec<i32> = (0..ncols)
            .map(|ci| {
                let max_chars = std::iter::once(headers[ci].len())
                    .chain(
                        st.rows
                            .iter()
                            .take(SAMPLE_ROWS)
                            .map(|r| r.get(ci).map(String::len).unwrap_or(0)),
                    )
                    .max()
                    .unwrap_or(0) as i32;
                (max_chars * CHAR_PX + PAD_PX).clamp(MIN_PX, MAX_PX)
            })
            .collect();

        drop(st);

        *self.current_store.borrow_mut() = Some(store.clone());

        let selection = NoSelection::new(Some(store.clone()));
        self.column_view.set_model(Some(&selection));

        // Snapshot the on_dirty callback once per load so it can be cloned
        // cheaply into each column's factory closures.
        let on_dirty_cb: Option<Rc<dyn Fn()>> = self.on_dirty.borrow().clone();

        for col_idx in 0..ncols {
            let factory = SignalListItemFactory::new();

            // Each cell is a Box > EditableLabel.
            // We apply search-highlight CSS classes to the Box, which has
            // a reliable CSS background rendering.
            factory.connect_setup(|_, obj| {
                let list_item = obj
                    .downcast_ref::<ListItem>()
                    .expect("setup object should be a ListItem");
                let cell_box = GtkBox::new(Orientation::Horizontal, 0);
                cell_box.set_hexpand(true);
                let label = EditableLabel::new("");
                label.set_hexpand(true);
                cell_box.append(&label);
                list_item.set_child(Some(&cell_box));
            });

            // Per-column side-table: widget pointer → SignalHandlerId.
            // Shared between bind and unbind closures for the same factory.
            // Eliminates the need for unsafe set_data / steal_data.
            let handler_map: Rc<RefCell<HashMap<usize, glib::SignalHandlerId>>> =
                Rc::new(RefCell::new(HashMap::new()));

            factory.connect_bind({
                let state = Rc::clone(&state);
                let store = store.clone();
                let handler_map = handler_map.clone();
                let on_dirty_cb = on_dirty_cb.clone();
                move |_, obj| {
                    let list_item = obj
                        .downcast_ref::<ListItem>()
                        .expect("bind object should be a ListItem");
                    let pos = list_item.position() as usize;

                    let cell_box = list_item
                        .child()
                        .expect("list item has no child")
                        .downcast::<GtkBox>()
                        .expect("child should be a GtkBox");
                    let label = cell_box
                        .first_child()
                        .expect("cell_box has no child")
                        .downcast::<EditableLabel>()
                        .expect("first child should be an EditableLabel");

                    // ── 1. Set cell text BEFORE connecting the changed handler ──
                    //
                    // GTK emits `changed` synchronously during set_text().  By
                    // connecting the handler only AFTER the text is set, the
                    // programmatic write is invisible to our handler — no need
                    // for an in_bind guard flag.
                    {
                        let item = list_item
                            .item()
                            .expect("list item has no model item")
                            .downcast::<BoxedAnyObject>()
                            .expect("model item should be BoxedAnyObject");
                        let row: Ref<Vec<String>> = item.borrow();
                        label.set_text(row.get(col_idx).map(String::as_str).unwrap_or(""));
                    }

                    // ── 2. Apply search highlighting ──────────────────────────
                    {
                        let st = state.borrow();
                        let is_current = st
                            .search
                            .current_match
                            .and_then(|i| st.search.matches_ordered.get(i))
                            .is_some_and(|&m| m == (pos, col_idx));
                        let is_match = st.search.matches.contains(&(pos, col_idx));

                        if is_current {
                            cell_box.add_css_class("search-match-current");
                            cell_box.remove_css_class("search-match");
                        } else if is_match {
                            cell_box.add_css_class("search-match");
                            cell_box.remove_css_class("search-match-current");
                        } else {
                            cell_box.remove_css_class("search-match");
                            cell_box.remove_css_class("search-match-current");
                        }
                    }

                    // ── 3. Connect edit handler (after set_text — safe) ───────
                    //
                    // TODO: data is written to two places: `state.rows` (master)
                    //       AND the `BoxedAnyObject` inside the ListStore (a
                    //       clone made at load time).  These can drift.  The
                    //       ListStore item should be the single source of truth,
                    //       with `state.rows` rebuilt on save, or vice-versa.
                    let state_c = Rc::clone(&state);
                    let store_c = store.clone();
                    let on_dirty_c = on_dirty_cb.clone();
                    let handler_id = label.connect_changed(move |lbl| {
                        let new_val = lbl.text().to_string();
                        {
                            let mut st = state_c.borrow_mut();
                            if let Some(row) = st.rows.get_mut(pos) {
                                while row.len() <= col_idx {
                                    row.push(String::new());
                                }
                                row[col_idx] = new_val.clone();
                                st.dirty = true;
                            }
                        }
                        if let Some(f) = &on_dirty_c {
                            f();
                        }
                        if let Some(obj) = store_c
                            .item(pos as u32)
                            .and_then(|o| o.downcast::<BoxedAnyObject>().ok())
                        {
                            let mut row: RefMut<Vec<String>> = obj.borrow_mut();
                            while row.len() <= col_idx {
                                row.push(String::new());
                            }
                            row[col_idx] = new_val;
                        }
                    });

                    // Store by widget pointer — safe, typed, no GObject internals.
                    let key = label.as_ptr() as usize;
                    handler_map.borrow_mut().insert(key, handler_id);
                }
            });

            factory.connect_unbind({
                let handler_map = handler_map.clone();
                move |_, obj| {
                    let list_item = obj
                        .downcast_ref::<ListItem>()
                        .expect("unbind object should be a ListItem");
                    let cell_box = list_item
                        .child()
                        .expect("list item has no child")
                        .downcast::<GtkBox>()
                        .expect("child should be a GtkBox");
                    let label = cell_box
                        .first_child()
                        .expect("cell_box has no child")
                        .downcast::<EditableLabel>()
                        .expect("first child should be an EditableLabel");

                    cell_box.remove_css_class("search-match");
                    cell_box.remove_css_class("search-match-current");

                    let key = label.as_ptr() as usize;
                    if let Some(id) = handler_map.borrow_mut().remove(&key) {
                        label.disconnect(id);
                    }
                }
            });

            let column = ColumnViewColumn::new(Some(&headers[col_idx]), Some(factory));
            column.set_resizable(true);
            column.set_fixed_width(col_widths[col_idx]);
            self.column_view.append_column(&column);
        }
    }
}
