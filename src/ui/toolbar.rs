use adw::HeaderBar;
use gtk::{
    Align, Box as GtkBox, Button, DropDown, Label, MenuButton, Orientation, Popover, Separator,
    ToggleButton, prelude::*,
};

/// Single source of truth for the separator dropdown.
/// Index in this array == dropdown position == what `current_separator()` reads.
/// `None` as the byte value means "Custom…" — triggers a dialog.
const SEPARATORS: &[(&str, Option<u8>)] = &[
    ("Comma (,)", Some(b',')),
    ("Semicolon (;)", Some(b';')),
    ("Tab (\\t)", Some(b'\t')),
    ("Pipe (|)", Some(b'|')),
    ("Custom…", None),
];

/// Dropdown index of the "Custom…" entry.  Exported so `mod.rs` can reference
/// it without duplicating the magic number.
pub const CUSTOM_SEP_IDX: u32 = (SEPARATORS.len() - 1) as u32;

/// Single source of truth for the encoding dropdown.
/// Index in this array == dropdown position == what `current_encoding()` reads.
/// Each entry is (label, encoding, with_bom).
pub static ENCODINGS: &[(&str, &encoding_rs::Encoding, bool)] = &[
    ("UTF-8", encoding_rs::UTF_8, false),
    ("UTF-8 with BOM", encoding_rs::UTF_8, true),
    ("Windows-1252", encoding_rs::WINDOWS_1252, false),
    ("Shift-JIS", encoding_rs::SHIFT_JIS, false),
    ("GB18030", encoding_rs::GB18030, false),
    ("UTF-16 LE", encoding_rs::UTF_16LE, true),
    ("UTF-16 BE", encoding_rs::UTF_16BE, true),
];

pub struct Toolbar {
    pub header_bar: HeaderBar,
    pub open_btn: Button,
    pub save_btn: Button,
    pub sep_dropdown: DropDown,
    pub enc_dropdown: DropDown,
    pub about_btn: Button,
    pub menu_popover: Popover,
    pub search_btn: ToggleButton,
}

impl Default for Toolbar {
    fn default() -> Self {
        Self::new()
    }
}

impl Toolbar {
    pub fn new() -> Self {
        let header_bar = HeaderBar::new();

        let open_btn = Button::from_icon_name("document-open-symbolic");
        open_btn.set_tooltip_text(Some("Open CSV"));

        let save_btn = Button::from_icon_name("document-save-symbolic");
        save_btn.set_tooltip_text(Some("Save CSV"));

        // ── Separator row inside popover ──────────────────────────────────
        let sep_labels: Vec<&str> = SEPARATORS.iter().map(|&(label, _)| label).collect();
        let sep_dropdown = DropDown::from_strings(&sep_labels);
        sep_dropdown.set_selected(0);
        sep_dropdown.set_hexpand(true);

        let sep_row = GtkBox::new(Orientation::Horizontal, 8);
        sep_row.set_margin_top(4);
        sep_row.set_margin_bottom(4);
        let sep_label = Label::new(Some("Separator"));
        sep_label.set_halign(Align::Start);
        sep_label.set_hexpand(true);
        sep_row.append(&sep_label);
        sep_row.append(&sep_dropdown);

        // ── Encoding row inside popover ───────────────────────────────────
        let enc_labels: Vec<&str> = ENCODINGS.iter().map(|&(label, _, _)| label).collect();
        let enc_dropdown = DropDown::from_strings(&enc_labels);
        enc_dropdown.set_selected(0);
        enc_dropdown.set_hexpand(true);

        let enc_row = GtkBox::new(Orientation::Horizontal, 8);
        enc_row.set_margin_top(4);
        enc_row.set_margin_bottom(4);
        let enc_label = Label::new(Some("Encoding"));
        enc_label.set_halign(Align::Start);
        enc_label.set_hexpand(true);
        enc_row.append(&enc_label);
        enc_row.append(&enc_dropdown);

        // ── About button ──────────────────────────────────────────────────
        let about_btn = Button::with_label("About Virgola");
        about_btn.set_has_frame(false);

        // ── Popover ───────────────────────────────────────────────────────
        // Two sections: "Settings" (separator + encoding) and a separator
        // line before the "About" action so the two concerns are distinct.
        let settings_label = Label::new(Some("Settings"));
        settings_label.set_halign(Align::Start);
        settings_label.add_css_class("heading");
        settings_label.set_margin_top(4);
        settings_label.set_margin_bottom(2);

        let popover_box = GtkBox::new(Orientation::Vertical, 4);
        popover_box.set_margin_top(8);
        popover_box.set_margin_bottom(8);
        popover_box.set_margin_start(8);
        popover_box.set_margin_end(8);
        popover_box.add_css_class("virgola-menu");
        popover_box.append(&settings_label);
        popover_box.append(&sep_row);
        popover_box.append(&enc_row);
        popover_box.append(&Separator::new(Orientation::Horizontal));
        popover_box.append(&about_btn);

        let menu_popover = Popover::new();
        menu_popover.set_child(Some(&popover_box));

        let menu_btn = MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_popover(Some(&menu_popover));

        let search_btn = ToggleButton::new();
        search_btn.set_icon_name("system-search-symbolic");
        search_btn.set_tooltip_text(Some("Search (Ctrl+F)"));

        header_bar.pack_start(&open_btn);
        header_bar.pack_start(&save_btn);
        header_bar.pack_end(&menu_btn);
        header_bar.pack_end(&search_btn);

        Self {
            header_bar,
            open_btn,
            save_btn,
            sep_dropdown,
            enc_dropdown,
            about_btn,
            menu_popover,
            search_btn,
        }
    }

    /// Returns `Some(byte)` for preset separators, `None` for "Custom…".
    pub fn current_separator(&self) -> Option<u8> {
        SEPARATORS
            .get(self.sep_dropdown.selected() as usize)
            .and_then(|&(_, byte)| byte)
    }

    /// Returns the dropdown index for a known separator byte, or `None` if the
    /// byte is not in the preset list (would need "Custom…").
    pub fn index_of_separator(sep: u8) -> Option<u32> {
        SEPARATORS
            .iter()
            .position(|&(_, byte)| byte == Some(sep))
            .map(|i| i as u32)
    }

    /// Returns the encoding and BOM flag for the currently selected entry.
    pub fn current_encoding(&self) -> (&'static encoding_rs::Encoding, bool) {
        let idx = self.enc_dropdown.selected() as usize;
        let (_, enc, bom) = ENCODINGS[idx];
        (enc, bom)
    }

    /// Returns the dropdown index whose encoding name and BOM flag match, or 0
    /// (UTF-8) if the encoding is not in the preset list.
    pub fn index_of_encoding(enc: &'static encoding_rs::Encoding, bom: bool) -> u32 {
        ENCODINGS
            .iter()
            .position(|&(_, e, b)| e.name() == enc.name() && b == bom)
            .unwrap_or(0) as u32
    }
}
