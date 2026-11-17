//! Messages exactly as a protocol-1 peer sends them. A plugin built today
//! keeps sending these for years; every later build of the host must still
//! read them (and a plugin must still read what a protocol-1 host sends).
//! Never edit these strings: add a new test for a new shape instead.
use shiny_plugin_protocol::*;

#[test]
fn a_v1_host_hello_decodes() {
    let hello: Hello = serde_json::from_str(
        r#"{"protocol":{"min":1,"max":1},"host_version":"0.2.8","config":{"server_host":"127.0.0.1","server_port":3000,"database_url":"sqlite://data/shiny.db"}}"#,
    )
    .unwrap();
    // While version 1 is supported, a v1 host and this build agree on it.
    assert!(negotiate(hello.protocol, VersionRange::current()).is_some());
}

#[test]
fn a_v1_plugin_welcome_decodes() {
    let welcome: Welcome = serde_json::from_str(
        r##"{"protocol":1,"manifest":{"name":"calculator","version":"0.1.1","api_level":1},"tools":[{"name":"calculator_eval","aliases":["calc"],"step_label":"Calculating…","doc_fragment":"- `calculator_eval`","step_ui":{"key":"steps.eval","params":{},"text":"Calculating…"}}],"routes":[{"method":"POST","path":"/api/calculator/eval","auth":"auth","handler_tag":"eval"}],"skills_md":"# Calculator","persona":"a calculator AI","context_lines":["Calculator: enabled"],"idle_stop_secs":600,"sdk":"0.1.0","error":null}"##,
    )
    .unwrap();
    assert_eq!(welcome.protocol, 1);
    assert_eq!(welcome.tools[0].aliases, vec!["calc"]);
    assert_eq!(welcome.routes[0].handler_tag, "eval");
    assert_eq!(welcome.idle_stop_secs, Some(600));
}

#[test]
fn a_v1_tool_call_and_reply_decode() {
    let call: ToolCall = serde_json::from_str(
        r#"{"tool":"hello","user_id":"u1","traveler_id":"u1","params":{"name":"Ada"},"context":{"lat":null,"lon":null,"heading":null,"lang":"it","ollama_model":null},"os_home":"/home/ada"}"#,
    )
    .unwrap();
    assert_eq!(call.os_home.as_deref(), Some("/home/ada"));
    let reply: ToolReply = serde_json::from_str(
        r#"{"outcome":{"action":"hello","result":"ok","data":{"reply":"Hello, Ada!"},"artifact":null,"extra_artifacts":[],"navigation":null},"error":null,"humanized":"Said hello to Ada","done_ui":null}"#,
    )
    .unwrap();
    assert_eq!(reply.outcome.unwrap().data["reply"], "Hello, Ada!");
    let failed: ToolReply = serde_json::from_str(
        r#"{"outcome":null,"error":{"kind":"bad_request","message":"name required"}}"#,
    )
    .unwrap();
    assert_eq!(failed.error.unwrap().kind, "bad_request");
}

#[test]
fn v1_acknowledgements_decode() {
    let done: Done = serde_json::from_str(r#"{"error":null}"#).unwrap();
    assert!(done.error.is_none());
    let user: UserRegistered = serde_json::from_str(r#"{"user_id":"u2"}"#).unwrap();
    assert_eq!(user.user_id, "u2");
    assert_eq!(paths::HELLO, "/_shiny/hello");
    assert_eq!(paths::TOOL, "/_shiny/tool");
    assert_eq!(ROUTE_TAG_HEADER, "x-shiny-route-tag");
    assert_eq!(SOCKET_ENV, "SHINY_PLUGIN_SOCKET");
}
