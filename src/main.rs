mod csv_handler;
mod state;
mod ui;

use adw::prelude::*;
use adw::{Application, gio, gtk};

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

    app.connect_startup(|_| {
        gtk::IconTheme::for_display(&gtk::gdk::Display::default().expect("no default display"))
            .add_resource_path("/com/github/virgola/icons");
    });

    // No file argument — just open the window.
    app.connect_activate(|app| ui::build_ui(app, None, 0));

    // One or more files from the CLI or desktop environment.
    // Only the first file is opened; surplus files are reported to the user.
    app.connect_open(|app, files, _hint| {
        let path = files.first().and_then(|f| f.path());
        let extra = files.len().saturating_sub(1);
        ui::build_ui(app, path, extra);
    });

    app.run();
}
