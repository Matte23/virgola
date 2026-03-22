use adw::{ComboRow, PreferencesGroup, SwitchRow, prelude::*};
use gtk::{Box as GtkBox, Orientation, PolicyType, ScrolledWindow, StringList};

/// Single source of truth for the separator options.
/// Index == dropdown position == what `current_separator()` reads.
/// `None` means "Custom…" — triggers a dialog.
pub const SEPARATORS: &[(&str, Option<u8>)] = &[
    ("Comma (,)", Some(b',')),
    ("Semicolon (;)", Some(b';')),
    ("Tab (\\t)", Some(b'\t')),
    ("Pipe (|)", Some(b'|')),
    ("Custom…", None),
];

/// Dropdown index of the "Custom…" entry.  Exported so `mod.rs` can reference
/// it without duplicating the magic number.
pub const CUSTOM_SEP_IDX: u32 = (SEPARATORS.len() - 1) as u32;

/// Single source of truth for the encoding options.
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

pub struct Sidebar {
    /// The top-level widget to hand to `adw::OverlaySplitView`.
    pub container: ScrolledWindow,
    pub sep_row: ComboRow,
    pub header_row: SwitchRow,
    pub enc_row: ComboRow,
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}

impl Sidebar {
    pub fn new() -> Self {
        // ── Preferences group ─────────────────────────────────────────────
        let group = PreferencesGroup::new();
        group.set_title("Document format");

        let sep_model = StringList::new(&SEPARATORS.iter().map(|&(l, _)| l).collect::<Vec<_>>());
        let sep_row = ComboRow::new();
        sep_row.set_title("Separator");
        sep_row.set_model(Some(&sep_model));
        sep_row.set_selected(0);
        group.add(&sep_row);

        let header_row = SwitchRow::new();
        header_row.set_title("Has header");
        header_row.set_active(true);
        group.add(&header_row);

        let enc_model = StringList::new(&ENCODINGS.iter().map(|&(l, _, _)| l).collect::<Vec<_>>());
        let enc_row = ComboRow::new();
        enc_row.set_title("Encoding");
        enc_row.set_model(Some(&enc_model));
        enc_row.set_selected(0);
        group.add(&enc_row);

        // ── Container ─────────────────────────────────────────────────────
        let content = GtkBox::new(Orientation::Vertical, 0);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.append(&group);

        let container = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .child(&content)
            .build();

        Self {
            container,
            sep_row,
            header_row,
            enc_row,
        }
    }

    /// Returns `Some(byte)` for preset separators, `None` for "Custom…".
    pub fn current_separator(&self) -> Option<u8> {
        SEPARATORS
            .get(self.sep_row.selected() as usize)
            .and_then(|&(_, byte)| byte)
    }

    /// Returns the dropdown index for a known separator byte, or `None` if
    /// the byte is not in the preset list (would need "Custom…").
    pub fn index_of_separator(sep: u8) -> Option<u32> {
        SEPARATORS
            .iter()
            .position(|&(_, byte)| byte == Some(sep))
            .map(|i| i as u32)
    }

    /// Returns the encoding and BOM flag for the currently selected entry.
    pub fn current_encoding(&self) -> (&'static encoding_rs::Encoding, bool) {
        let idx = self.enc_row.selected() as usize;
        let (_, enc, bom) = ENCODINGS[idx];
        (enc, bom)
    }

    /// Returns the dropdown index whose encoding name and BOM flag match, or
    /// 0 (UTF-8) if the encoding is not in the preset list.
    pub fn index_of_encoding(enc: &'static encoding_rs::Encoding, bom: bool) -> u32 {
        ENCODINGS
            .iter()
            .position(|&(_, e, b)| e.name() == enc.name() && b == bom)
            .unwrap_or(0) as u32
    }
}
