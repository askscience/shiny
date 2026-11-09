//! Print the document Capture renders for an HTML graphic (debugging).
use shiny_capture_plugin::editor::{
    project::{Overlay, Project},
    render,
};
fn main() {
    let html = std::fs::read_to_string(std::env::args().nth(1).expect("html file")).unwrap();
    let p = Project::default();
    let o = Overlay {
        html,
        dur: 2.0,
        ..Overlay::default()
    };
    print!("{}", render::document(&p, &o, 960, 216));
}
