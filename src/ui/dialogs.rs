use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, Entry, Label, Orientation, Window, glib,
    prelude::*,
};
use std::cell::RefCell;
use std::rc::Rc;

/// Show a modal dialog asking for a single separator character.
/// Calls `on_result` with `Some(byte)` on confirmation, `None` on cancel.
pub fn show_custom_separator_dialog<F: FnOnce(Option<u8>) + 'static>(
    parent: &ApplicationWindow,
    on_result: F,
) {
    let dialog = Window::builder()
        .title("Custom Separator")
        .transient_for(parent)
        .modal(true)
        .default_width(280)
        .resizable(false)
        .build();

    let vbox = GtkBox::new(Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);

    let label = Label::new(Some("Enter separator character:"));

    // max_length(1) limits to one Unicode character, but only ASCII bytes
    // (< 128) are valid CSV delimiters.  We validate in the OK handler.
    let entry = Entry::builder()
        .max_length(1)
        .placeholder_text("e.g.  |  or  ;")
        .build();

    let btn_box = GtkBox::new(Orientation::Horizontal, 6);
    btn_box.set_halign(Align::End);
    let cancel_btn = Button::with_label("Cancel");
    let ok_btn = Button::with_label("OK");
    btn_box.append(&cancel_btn);
    btn_box.append(&ok_btn);

    vbox.append(&label);
    vbox.append(&entry);
    vbox.append(&btn_box);
    dialog.set_child(Some(&vbox));

    // Wrap callback in Rc<RefCell<Option<F>>> so it can be consumed once.
    let callback: Rc<RefCell<Option<F>>> = Rc::new(RefCell::new(Some(on_result)));

    // Clear the error indicator whenever the user types.
    entry.connect_changed(|e| {
        e.remove_css_class("error");
    });

    // ── OK handler (shared by Enter key and the OK button) ───────────────
    //
    // Wrapped in Rc<dyn Fn()> so it can be cloned into two signal connections
    // without needing the closure itself to be Clone.
    let try_confirm: Rc<dyn Fn()> = Rc::new({
        let cb = callback.clone();
        let entry_ok = entry.clone();
        let dlg = dialog.clone();
        move || {
            let text = entry_ok.text();
            let bytes = text.as_bytes();

            if bytes.is_empty() {
                // Nothing typed yet — mark the field and wait.
                entry_ok.add_css_class("error");
                return;
            }

            // Only ASCII printable characters are valid CSV delimiters.
            let byte = bytes[0];
            if !(32..128).contains(&byte) || byte == 127 {
                entry_ok.add_css_class("error");
                return;
            }

            entry_ok.remove_css_class("error");
            if let Some(f) = cb.borrow_mut().take() {
                f(Some(byte));
            }
            dlg.close();
        }
    });

    // Enter key inside the entry confirms — same as clicking OK.
    entry.connect_activate({
        let try_confirm = try_confirm.clone();
        move |_| try_confirm()
    });

    ok_btn.connect_clicked({
        let try_confirm = try_confirm.clone();
        move |_| try_confirm()
    });

    let cb = callback.clone();
    let dlg = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        if let Some(f) = cb.borrow_mut().take() {
            f(None);
        }
        dlg.close();
    });

    let cb = callback.clone();
    dialog.connect_close_request(move |_| {
        if let Some(f) = cb.borrow_mut().take() {
            f(None);
        }
        glib::Propagation::Proceed
    });

    dialog.present();
}
