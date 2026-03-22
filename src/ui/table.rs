use crate::state::State;
use glib::BoxedAnyObject;
use gtk::{
    Box as GtkBox, ColumnView, ColumnViewColumn, Entry, EventControllerKey, GestureClick, Label,
    ListItem, NoSelection, Orientation, Popover, ScrolledWindow, SignalListItemFactory, gio, glib,
    prelude::*,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

type OnDirty = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

fn apply_highlight(cell_box: &GtkBox, row: usize, col: usize, state: &State) {
    let is_current = state
        .search
        .current_match
        .and_then(|i| state.search.matches.get_index(i))
        .is_some_and(|&m| m == (row, col));
    let is_match = state.search.matches.contains(&(row, col));

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

pub struct Table {
    pub scrolled: ScrolledWindow,
    pub column_view: ColumnView,
    current_store: RefCell<Option<gio::ListStore>>,
    // Called whenever a cell edit commits and sets state.dirty = true.
    on_dirty: OnDirty,
    // Maps (row, col) → currently-bound cell box for direct CSS updates.
    cell_registry: Rc<RefCell<HashMap<(usize, usize), GtkBox>>>,
    // Single shared popover + entry used for all cell editing.
    // Only one cell can be edited at a time, so one Entry instance suffices.
    edit_popover: Popover,
    edit_entry: Entry,
    // Which (row, col) is currently open for editing, or None.
    editing_pos: Rc<Cell<Option<(usize, usize)>>>,
    // The state for the currently-loaded file, set in load().
    current_state: Rc<RefCell<Option<Rc<RefCell<State>>>>>,
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
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .child(&column_view)
            .build();

        let edit_entry = Entry::new();
        let edit_popover = Popover::new();
        edit_popover.set_child(Some(&edit_entry));
        edit_popover.set_has_arrow(false);

        let editing_pos: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
        let current_state: Rc<RefCell<Option<Rc<RefCell<State>>>>> = Rc::new(RefCell::new(None));
        let on_dirty: OnDirty = Rc::new(RefCell::new(None));
        let cell_registry: Rc<RefCell<HashMap<(usize, usize), GtkBox>>> =
            Rc::new(RefCell::new(HashMap::new()));

        // Commit the edit when the popover closes for any reason (Enter, click-
        // outside, or programmatic popdown).  Escape is handled separately below
        // and clears `editing_pos` first so this handler becomes a no-op.
        edit_popover.connect_closed({
            let edit_entry = edit_entry.clone();
            let editing_pos = editing_pos.clone();
            let current_state = current_state.clone();
            let cell_registry = cell_registry.clone();
            let on_dirty = on_dirty.clone();
            move |_| {
                let Some((row, col)) = editing_pos.take() else {
                    return;
                };
                let new_val = edit_entry.text().to_string();

                if let Some(state) = current_state.borrow().as_ref() {
                    let mut st = state.borrow_mut();
                    if let Some(r) = st.rows.get_mut(row) {
                        while r.len() <= col {
                            r.push(String::new());
                        }
                        r[col] = new_val.clone();
                        st.dirty = true;
                    }
                }

                // Update the visible label directly if the cell is still bound.
                if let Some(cell_box) = cell_registry.borrow().get(&(row, col))
                    && let Some(label) = cell_box
                        .first_child()
                        .and_then(|w| w.downcast::<Label>().ok())
                {
                    label.set_text(&new_val);
                }

                if let Some(f) = on_dirty.borrow().as_ref() {
                    f();
                }
            }
        });

        // Enter commits by closing the popover (triggers connect_closed above).
        edit_entry.connect_activate({
            let edit_popover = edit_popover.clone();
            move |_| {
                edit_popover.popdown();
            }
        });

        // Escape discards: clear editing_pos before popdown so connect_closed
        // exits early without writing anything back.
        let key_ctrl = EventControllerKey::new();
        key_ctrl.connect_key_pressed({
            let editing_pos = editing_pos.clone();
            let edit_popover = edit_popover.clone();
            move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    editing_pos.set(None);
                    edit_popover.popdown();
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
        });
        edit_entry.add_controller(key_ctrl);

        Self {
            scrolled,
            column_view,
            current_store: RefCell::new(None),
            on_dirty,
            cell_registry,
            edit_popover,
            edit_entry,
            editing_pos,
            current_state,
        }
    }

    pub fn set_on_dirty(&self, f: Rc<dyn Fn()>) {
        *self.on_dirty.borrow_mut() = Some(f);
    }

    /// Update CSS highlight classes on all currently-visible cells.
    ///
    /// Directly iterates the cell registry (bound widgets only) and applies
    /// the correct CSS class based on the current search state — no store
    /// splice or rebind required.
    pub fn refresh_matches(&self, state: &State) {
        for (&(row, col), cell_box) in self.cell_registry.borrow().iter() {
            apply_highlight(cell_box, row, col, state);
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
            cv.scroll_to(
                u32::try_from(row).unwrap_or(u32::MAX),
                None,
                gtk::ListScrollFlags::FOCUS,
                None,
            );
        });
    }

    pub fn load(&self, state: Rc<RefCell<State>>) {
        // `load()` tears down and rebuilds all columns and the ListStore.
        // This is intentional: all callers (open file, separator change) supply
        // a new schema, so a full rebuild is always correct.  Cell values are
        // NOT stored in the ListStore — bind closures read from `state.rows`
        // directly, so no partial-refresh path is needed.

        // Dismiss any in-progress edit from a previous file.
        self.editing_pos.set(None);
        self.edit_popover.popdown();
        if self.edit_popover.parent().is_some() {
            self.edit_popover.unparent();
        }

        *self.current_state.borrow_mut() = Some(Rc::clone(&state));

        // Remove existing columns
        let cols_model = self.column_view.columns();
        let cols: Vec<ColumnViewColumn> = (0..cols_model.n_items())
            .filter_map(|i| cols_model.item(i)?.downcast::<ColumnViewColumn>().ok())
            .collect();
        for col in cols {
            self.column_view.remove_column(&col);
        }

        let st = state.borrow();
        if st.headers.is_empty() {
            self.column_view.set_model(gtk::SelectionModel::NONE);
            *self.current_store.borrow_mut() = None;
            return;
        }

        let store = gio::ListStore::new::<BoxedAnyObject>();
        for _ in &st.rows {
            store.append(&BoxedAnyObject::new(()));
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

        self.cell_registry.borrow_mut().clear();
        *self.current_store.borrow_mut() = Some(store.clone());

        let selection = NoSelection::new(Some(store));
        self.column_view.set_model(Some(&selection));

        for col_idx in 0..ncols {
            let factory = self.build_column_factory(col_idx, Rc::clone(&state));
            let column = ColumnViewColumn::new(Some(&headers[col_idx]), Some(factory));
            column.set_resizable(true);
            column.set_fixed_width(col_widths[col_idx]);
            self.column_view.append_column(&column);
        }
    }

    /// Build the `SignalListItemFactory` for a single column.
    ///
    /// Each cell is a GtkBox containing a plain Label for display.
    /// Editing is handled by a single shared Popover+Entry that is
    /// re-parented to whichever cell the user clicks — only one Entry
    /// instance exists for the entire table regardless of row count.
    ///
    /// Per-factory side-table: widget-ptr → Rc<Cell<usize>> (current row).
    /// The GestureClick in setup captures the Rc; bind updates it.
    fn build_column_factory(
        &self,
        col_idx: usize,
        state: Rc<RefCell<State>>,
    ) -> SignalListItemFactory {
        let factory = SignalListItemFactory::new();

        let pos_map: Rc<RefCell<HashMap<usize, Rc<Cell<usize>>>>> =
            Rc::new(RefCell::new(HashMap::new()));

        factory.connect_setup({
            let edit_popover = self.edit_popover.clone();
            let edit_entry = self.edit_entry.clone();
            let editing_pos = self.editing_pos.clone();
            let current_state = self.current_state.clone();
            let pos_map = pos_map.clone();
            move |_, obj| {
                let list_item = obj
                    .downcast_ref::<ListItem>()
                    .expect("setup object should be a ListItem");

                let cell_box = GtkBox::new(Orientation::Horizontal, 0);
                cell_box.set_hexpand(true);
                let label = Label::new(None);
                label.set_hexpand(true);
                label.set_xalign(0.0);
                cell_box.append(&label);
                list_item.set_child(Some(&cell_box));

                // Track which row this recycled widget is currently showing.
                let pos_cell = Rc::new(Cell::new(0usize));
                let key = cell_box.as_ptr() as usize;
                pos_map.borrow_mut().insert(key, pos_cell.clone());

                let gesture = GestureClick::new();
                gesture.connect_released({
                    let edit_popover = edit_popover.clone();
                    let edit_entry = edit_entry.clone();
                    let editing_pos = editing_pos.clone();
                    let current_state = current_state.clone();
                    let cell_box = cell_box.clone();
                    move |_, _, _, _| {
                        // Commit any in-progress edit before opening a new one.
                        // popdown() is a no-op when already closed.
                        edit_popover.popdown();

                        let row = pos_cell.get();
                        let text = current_state
                            .borrow()
                            .as_ref()
                            .and_then(|s| {
                                s.borrow()
                                    .rows
                                    .get(row)
                                    .and_then(|r| r.get(col_idx))
                                    .cloned()
                            })
                            .unwrap_or_default();

                        editing_pos.set(Some((row, col_idx)));
                        edit_entry.set_text(&text);
                        edit_entry.select_region(0, -1);

                        if edit_popover.parent().is_some() {
                            edit_popover.unparent();
                        }
                        edit_popover.set_parent(&cell_box);
                        edit_popover.popup();
                        edit_entry.grab_focus();
                    }
                });
                cell_box.add_controller(gesture);
            }
        });

        factory.connect_bind({
            let pos_map = pos_map.clone();
            let cell_registry = Rc::clone(&self.cell_registry);
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
                    .downcast::<Label>()
                    .expect("first child should be a Label");

                // Update the position cell so the gesture sees the correct row.
                let key = cell_box.as_ptr() as usize;
                if let Some(pos_cell) = pos_map.borrow().get(&key) {
                    pos_cell.set(pos);
                }

                // `state.rows` is the single source of truth; the ListStore
                // holds only dummy placeholder items (one per row) for
                // virtual-scroll bookkeeping.
                let st = state.borrow();
                let text = st
                    .rows
                    .get(pos)
                    .and_then(|r| r.get(col_idx))
                    .map(String::as_str)
                    .unwrap_or("");
                label.set_text(text);
                apply_highlight(&cell_box, pos, col_idx, &st);
                drop(st);

                cell_registry
                    .borrow_mut()
                    .insert((pos, col_idx), cell_box.clone());
            }
        });

        factory.connect_unbind({
            let cell_registry = Rc::clone(&self.cell_registry);
            let edit_popover = self.edit_popover.clone();
            let editing_pos = self.editing_pos.clone();
            move |_, obj| {
                let list_item = obj
                    .downcast_ref::<ListItem>()
                    .expect("unbind object should be a ListItem");
                let pos = list_item.position() as usize;

                cell_registry.borrow_mut().remove(&(pos, col_idx));

                // If this cell is currently open for editing, commit and close.
                if editing_pos.get() == Some((pos, col_idx)) {
                    edit_popover.popdown();
                }
            }
        });

        factory.connect_teardown({
            let pos_map = pos_map.clone();
            let edit_popover = self.edit_popover.clone();
            move |_, obj| {
                let list_item = obj
                    .downcast_ref::<ListItem>()
                    .expect("teardown object should be a ListItem");

                if let Some(child) = list_item.child() {
                    let key = child.as_ptr() as usize;
                    pos_map.borrow_mut().remove(&key);

                    // Unparent the popover if it is anchored to this widget,
                    // otherwise it would hold a dangling parent reference.
                    if edit_popover
                        .parent()
                        .is_some_and(|p| p.as_ptr() as usize == key)
                    {
                        edit_popover.unparent();
                    }
                }
                list_item.set_child(gtk::Widget::NONE);
            }
        });

        factory
    }
}
