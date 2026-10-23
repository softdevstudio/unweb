fn main() {
    // App commands get generated allow-/deny- permissions, so each one has
    // to be granted in capabilities/. Only the bar is; the page gets none.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "go",
            "back",
            "forward",
            "reload",
            "new_stranger",
        ]),
    ))
    .expect("tauri build step failed");
}
