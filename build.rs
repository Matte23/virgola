fn main() {
    glib_build_tools::compile_resources(
        &["resources"],
        "resources/virgola.gresource.xml",
        "virgola.gresource",
    );
}
