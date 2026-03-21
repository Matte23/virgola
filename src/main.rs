mod csv_handler;
mod state;
mod ui;

use adw::prelude::*;
use adw::{Application, gio};

const APP_ID: &str = "com.github.virgola";

fn main() {
    gio::resources_register_include!("virgola.gresource")
        .expect("failed to register GResource bundle");

    let app = Application::builder()
        .application_id(APP_ID)
        // HANDLES_OPEN allows files to be passed on the command line and via
        // "Open With" from the desktop environment (%f in the .desktop file).
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
