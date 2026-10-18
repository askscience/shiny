//! Load the explicitly built image plugin against an isolated database.
mod support;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn loads_the_image_plugin() {
    let app = support::TestApp::new().await;
    app.stage_plugin("image");
    let state = &app.state;
    let installed = app.load_plugins().await;
    assert_eq!(installed, ["image"]);

    let names = state.plugins.tools().list();
    for tool in ["image_list", "image_get", "image_edit", "image_delete"] {
        assert!(
            names.iter().any(|t| t == tool),
            "{tool} registered; got {names:?}"
        );
    }
    // The layers era is gone: no layer tools, no flatten, and no
    // AI-edit-sounding aliases (`edit_image` used to swallow intent meant for
    // core `generate_image`).
    for gone in [
        "image_layer_list",
        "image_layer_add",
        "image_layer_delete",
        "image_flatten",
        "add_layer",
        "flatten_image",
        "edit_image",
        "transform_image",
    ] {
        assert!(
            !names.iter().any(|t| t == gone),
            "{gone} should be gone; got {names:?}"
        );
    }
    // Reaching here means the manifest, cdylib and *all* migrations (including
    // 005_minimal_layers and 006_drop_layers) applied cleanly, since a
    // migration failure aborts install before tools are registered.
    println!("SURVIVED");
}
