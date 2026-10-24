fn main() {
    // App commands get generated allow-/deny- permissions, so each one has
    // to be granted in capabilities/. The bar and the strips are; pages get
    // none.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "go",
            "back",
            "forward",
            "reload",
            "new_stranger",
            "side_by_side",
            "set_language",
            "set_location",
            "close_stranger",
            "save_setup",
            "open_setup",
            "delete_setup",
            "compare",
            "save_compare",
        ]),
    ))
    .expect("tauri build step failed");
}
