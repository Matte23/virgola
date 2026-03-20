mod csv_handler;
mod state;
mod ui;

use gtk4::prelude::*;
use gtk4::{gio, Application};

const APP_ID: &str = "com.github.virgola";

fn main() {
    // TODO: load a GResource bundle instead of hardcoding CSS strings and icon
    //       names scattered across the UI modules.

    let app = Application::builder()
        .application_id(APP_ID)
        // HANDLES_OPEN allows files to be passed on the command line and via
        // "Open With" from the desktop environment (%f in the .desktop file).
        // Without this flag GLib rejects file arguments at startup.
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    // No file argument — just open the window.
    app.connect_activate(|app| ui::build_ui(app, None));

    // One or more files from the CLI or desktop environment.
    // We only handle the first file; the rest are silently ignored for now.
    app.connect_open(|app, files, _hint| {
        let path = files.first().and_then(|f| f.path());
        ui::build_ui(app, path);
    });

    app.run();
}
