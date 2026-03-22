use adw::{HeaderBar, WindowTitle, gio};
use gtk::{Button, MenuButton, ToggleButton, prelude::*};

pub struct Toolbar {
    pub header_bar: HeaderBar,
    pub window_title: WindowTitle,
    pub open_btn: Button,
    pub save_btn: Button,
    pub sidebar_btn: ToggleButton,
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
        let window_title = WindowTitle::new("Untitled", "");
        header_bar.set_title_widget(Some(&window_title));

        let open_btn = Button::from_icon_name("document-open-symbolic");
        open_btn.set_tooltip_text(Some("Open CSV"));

        let save_btn = Button::from_icon_name("document-save-symbolic");
        save_btn.set_tooltip_text(Some("Save CSV"));

        let menu = gio::Menu::new();
        menu.append(Some("About Virgola"), Some("win.about"));

        let menu_btn = MenuButton::new();
        menu_btn.set_icon_name("open-menu-symbolic");
        menu_btn.set_menu_model(Some(&menu));

        let sidebar_btn = ToggleButton::new();
        sidebar_btn.set_icon_name("info-outline-symbolic");
        sidebar_btn.set_tooltip_text(Some("Document settings"));

        let search_btn = ToggleButton::new();
        search_btn.set_icon_name("system-search-symbolic");
        search_btn.set_tooltip_text(Some("Search (Ctrl+F)"));

        header_bar.pack_start(&open_btn);
        header_bar.pack_start(&save_btn);
        header_bar.pack_end(&menu_btn);
        header_bar.pack_end(&sidebar_btn);
        header_bar.pack_end(&search_btn);

        Self {
            header_bar,
            window_title,
            open_btn,
            save_btn,
            sidebar_btn,
            search_btn,
        }
    }
}
