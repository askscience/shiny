//! The plugin builder's Python module.

use super::Readiness;
use crate::services::ask::AskLine;

/// Whether this version ships the Python plugin kit: the library a Python
/// plugin speaks the plugin protocol with, and its template. Until it does,
/// the choice card says so instead of offering a language that leads nowhere.
const KIT_SHIPPED: bool = false;

/// Python plugins need the interpreter (the LiteAura package depends on it,
/// so on an installed system it is always there) and the plugin kit.
pub fn readiness() -> Readiness {
    assess(super::rust::on_path("python3", &[]), KIT_SHIPPED)
}

fn assess(interpreter: bool, kit: bool) -> Readiness {
    let mut missing = Vec::new();
    if !interpreter {
        missing.push(AskLine::keyed("chat.ask.missing.python", "Python 3"));
    }
    if !kit {
        missing.push(AskLine::keyed(
            "chat.ask.missing.pythonKit",
            "Python plugin kit (not in this version yet)",
        ));
    }
    Readiness { missing, fetchable: false }
}

#[cfg(test)]
mod tests {
    #[test]
    fn python_needs_the_interpreter_and_the_kit() {
        assert!(super::assess(true, true).ready());
        let without = super::assess(false, true);
        assert_eq!(without.missing_text(), vec!["Python 3"]);
        assert!(!without.fetchable, "the system package is not ours to install");
        let no_kit = super::assess(true, false);
        assert_eq!(no_kit.missing.len(), 1);
        assert!(!no_kit.ready());
    }
}
