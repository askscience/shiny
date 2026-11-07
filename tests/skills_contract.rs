//! Every plugin's skill has to reach the AI whole, and has to be right.
//!
//! The model only knows a plugin's tools from its skill (the system prompt)
//! and from the tool names the continuation prompt lists. When a skill is
//! missing, leaves a tool out or names one that does not exist, the model
//! guesses (`word_create_document` for `doc_create`) and ends up reading the
//! skill files from the terminal. This loads the explicitly built cdylibs and
//! checks, for every plugin that registers tools:
//!
//!   1. it hands the host a skill (`skills_for` is not empty);
//!   2. the skill names every tool the plugin registers;
//!   3. every `{"action":"…"}` example in the skill is a tool that exists
//!      (the plugin's own, another plugin's, or a core action the core
//!      skill documents);
//!   4. the registry lists the plugin's tools by name (`names_owned_by`), the
//!      list the continuation prompt carries.
//!
//! Run with:
//!     cargo test --test skills_contract -- --nocapture

mod support;

use std::collections::BTreeSet;

/// The tool names a markdown text uses in `"action":"name"` examples.
fn action_examples(markdown: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = markdown;
    while let Some(at) = rest.find("\"action\"") {
        rest = &rest[at + "\"action\"".len()..];
        let after_colon = rest.trim_start().strip_prefix(':').map(str::trim_start);
        if let Some(quoted) = after_colon.and_then(|s| s.strip_prefix('"')) {
            let name: String = quoted
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() && !name.contains('<') {
                out.insert(name);
            }
        }
    }
    out
}

/// Whether the skill names `tool` as a word of its own (backticks, an action
/// example or plain prose), not as part of a longer name.
fn mentions(markdown: &str, tool: &str) -> bool {
    markdown.match_indices(tool).any(|(at, _)| {
        let before = markdown[..at].chars().next_back();
        let after = markdown[at + tool.len()..].chars().next();
        let word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        !word(before) && !word(after)
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_plugin_hands_the_ai_a_whole_and_correct_skill() {
    let app = support::TestApp::new().await;
    app.stage_all_plugins();
    let state = &app.state;
    let installed = app.load_plugins().await;
    let tools = state.plugins.tools();

    let core_skill = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/skills/core-assistant.md"),
    )
    .expect("core skill");
    let core_actions = action_examples(&core_skill);

    let mut problems: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for plugin in &installed {
        let owned = tools.names_owned_by(plugin);
        if owned.is_empty() {
            continue;
        }
        checked += 1;
        let skill = state.plugins.skills_for(plugin);
        if skill.trim().is_empty() {
            problems.push(format!("{plugin}: registers {} tools but hands over no skill", owned.len()));
            continue;
        }
        for tool in &owned {
            if !mentions(&skill, tool) {
                problems.push(format!("{plugin}: the skill never names its tool `{tool}`"));
            }
        }
        for example in action_examples(&skill) {
            if !tools.has(&example) && !core_actions.contains(&example) {
                problems.push(format!("{plugin}: the skill calls `{example}`, which is no tool"));
            }
        }
        println!("{plugin}: {} tools, skill {} bytes", owned.len(), skill.len());
    }

    assert!(checked > 0, "no plugin registered tools — did nothing load?");
    assert!(
        tools.names_owned_by("word").iter().any(|t| t == "doc_create"),
        "word's tool list must carry doc_create"
    );
    assert!(problems.is_empty(), "skill problems:\n{}", problems.join("\n"));
}

#[test]
fn reads_action_examples() {
    let md = r#"`{"action":"doc_create","params":{}}` and {"action": "doc_edit"} but not {"action":"<tool>"}"#;
    let found = action_examples(md);
    assert_eq!(found, BTreeSet::from(["doc_create".to_string(), "doc_edit".to_string()]));
    assert!(mentions("- `doc_create` — make one", "doc_create"));
    assert!(!mentions("doc_create_many", "doc_create"));
}
