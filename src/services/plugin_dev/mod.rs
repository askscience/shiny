//! The plugin builder: the commands the assistant builds plugins with.
//!
//! Every command is a fixed function of the core with fixed parameters: the
//! assistant picks a command, it never writes a shell line. The commands are
//! split in modules, and a conversation sees one language's module only:
//!
//! - **entry**: `plugin_dev_start`, the one command a chat knows before the
//!   user chose a language. It shows the Python / Rust card.
//! - **common**: workspace, checks, packaging, installation and diagnostics,
//!   the same whatever the language.
//! - **python**, **rust**: what only that language needs.
//!
//! The language is the **user's** choice, made once per conversation and kept
//! with it ([`state`]). [`scope_refusal`] is what enforces the split: a
//! command of the other language's module is refused by the core, whatever
//! the prompt says, the same way focus mode refuses another plugin's tools.

pub mod checks;
pub mod commands;
pub mod dialog;
pub mod package;
pub mod python;
pub mod rust;
pub mod state;
pub mod workspace;

use crate::services::ask::AskLine;

/// The language a conversation builds plugins in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Python,
    Rust,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::Python, Language::Rust];

    pub fn as_str(self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::Rust => "rust",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Language::Python => "Python",
            Language::Rust => "Rust",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "python" => Some(Language::Python),
            "rust" => Some(Language::Rust),
            _ => None,
        }
    }

    /// What this machine has, and lacks, for building in this language.
    pub fn readiness(self) -> Readiness {
        match self {
            Language::Python => python::readiness(),
            Language::Rust => rust::readiness(),
        }
    }
}

/// What building in one language needs on this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Readiness {
    /// What is missing, item by item. Empty means ready.
    pub missing: Vec<AskLine>,
    /// Whether what is missing can be fetched for the user (after they
    /// agree). False when it has to be installed by hand.
    pub fetchable: bool,
}

impl Readiness {
    pub fn ready(&self) -> bool {
        self.missing.is_empty()
    }

    pub fn missing_text(&self) -> Vec<String> {
        self.missing.iter().map(|line| line.text.clone()).collect()
    }
}

/// The module a builder command belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    Entry,
    Common,
    Only(Language),
}

/// Every builder command and its module. A command is added here and in its
/// module's dispatch; nothing else decides who may call it.
const COMMANDS: &[(&str, Module)] = &[
    ("plugin_dev_start", Module::Entry),
    ("plugin_dev_new", Module::Common),
    ("plugin_dev_list", Module::Common),
    ("plugin_dev_tree", Module::Common),
    ("plugin_dev_read", Module::Common),
    ("plugin_dev_write", Module::Common),
    ("plugin_dev_patch", Module::Common),
    ("plugin_dev_delete", Module::Common),
    ("check_manifest", Module::Common),
    ("check_web", Module::Common),
    ("check_i18n", Module::Common),
    ("check_migrations", Module::Common),
    ("check_conflicts", Module::Common),
    ("check_all", Module::Common),
    ("plugin_dev_pack", Module::Common),
    ("plugin_dev_install", Module::Common),
    ("plugin_dev_status", Module::Common),
    ("plugin_dev_logs", Module::Common),
    ("rust_doctor", Module::Only(Language::Rust)),
    ("rust_build", Module::Only(Language::Rust)),
];

/// The builder's command docs for one conversation: the common commands and
/// its language's, read from `web/skills/` at request time like the core
/// skill, so an edit needs no restart.
pub fn skill_docs(language: Language) -> String {
    let read = |file: &str| std::fs::read_to_string(format!("web/skills/{file}")).unwrap_or_default();
    let common = read("plugin-builder.md");
    let own = read(&format!("plugin-builder-{}.md", language.as_str()));
    [common.trim(), own.trim()].iter().filter(|part| !part.is_empty()).copied().collect::<Vec<_>>().join("\n\n")
}

/// The module of a builder command, `None` for any other action.
pub fn module_of(action: &str) -> Option<Module> {
    COMMANDS.iter().find(|(name, _)| *name == action).map(|(_, module)| *module)
}

/// The commands a conversation may call, given the language chosen in it.
pub fn commands_for(language: Option<Language>) -> Vec<&'static str> {
    COMMANDS
        .iter()
        .filter(|(_, module)| allowed(*module, language))
        .map(|(name, _)| *name)
        .collect()
}

fn allowed(module: Module, language: Option<Language>) -> bool {
    match (module, language) {
        (Module::Entry, _) => true,
        (Module::Common, Some(_)) => true,
        (Module::Only(wanted), Some(chosen)) => wanted == chosen,
        (_, None) => false,
    }
}

/// Why a builder command is refused in this conversation, if it is. `None`
/// for a command this conversation may call, and for any action that is not
/// a builder command.
pub fn scope_refusal(action: &str, language: Option<Language>) -> Option<String> {
    let module = module_of(action)?;
    if allowed(module, language) {
        return None;
    }
    Some(match (module, language) {
        (Module::Only(wanted), Some(chosen)) => format!(
            "{action} is a {} command and this chat builds plugins in {}. The language is the \
             user's choice for the whole chat and cannot change here: use this chat's commands, \
             or tell the user that a {} plugin needs a new chat.",
            wanted.label(),
            chosen.label(),
            wanted.label(),
        ),
        _ => format!(
            "{action} is not available yet: the user has not chosen a language for this chat. \
             Call plugin_dev_start first; it asks them."
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_without_a_language_knows_only_the_entry_command() {
        assert_eq!(commands_for(None), vec!["plugin_dev_start"]);
        assert_eq!(scope_refusal("plugin_dev_start", None), None);
        assert_eq!(scope_refusal("web_search", None), None, "not a builder command");
    }

    #[test]
    fn a_chat_sees_the_common_commands_and_its_own_languages() {
        let rust = commands_for(Some(Language::Rust));
        let python = commands_for(Some(Language::Python));
        for common in ["plugin_dev_new", "check_all", "plugin_dev_install"] {
            assert!(rust.contains(&common) && python.contains(&common), "{common}");
        }
        assert!(rust.contains(&"rust_build") && !python.contains(&"rust_build"));

        let refusal = scope_refusal("rust_build", Some(Language::Python)).expect("refused");
        assert!(refusal.contains("a Rust command") && refusal.contains("builds plugins in Python"), "{refusal}");
        assert_eq!(scope_refusal("rust_build", Some(Language::Rust)), None);
        let early = scope_refusal("plugin_dev_write", None).expect("refused");
        assert!(early.contains("plugin_dev_start"), "{early}");
    }

    #[test]
    fn modules_follow_the_language_chosen_in_the_chat() {
        for (module, language, expected) in [
            (Module::Entry, None, true),
            (Module::Entry, Some(Language::Rust), true),
            (Module::Common, None, false),
            (Module::Common, Some(Language::Python), true),
            (Module::Only(Language::Rust), Some(Language::Rust), true),
            (Module::Only(Language::Rust), Some(Language::Python), false),
            (Module::Only(Language::Python), Some(Language::Rust), false),
            (Module::Only(Language::Python), None, false),
        ] {
            assert_eq!(allowed(module, language), expected, "{module:?} with {language:?}");
        }
    }

    #[test]
    fn every_command_is_listed_once() {
        let mut names: Vec<&str> = COMMANDS.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before);
    }

    #[test]
    fn languages_parse_from_their_stored_form() {
        for language in Language::ALL {
            assert_eq!(Language::parse(language.as_str()), Some(language));
        }
        assert_eq!(Language::parse(" Rust "), Some(Language::Rust));
        assert_eq!(Language::parse("go"), None);
    }
}
