# Traveler — skills contract

The plugin advertises [`skills/traveler-api-tools.md`](../skills/traveler-api-tools.md)
to the model. It is the JSON contract the LLM reads; every advertised tool also
has a `doc_fragment`, and the two are concatenated into the agent system prompt
(see [agent](../../../docs/core/agent.md)).

The file groups the verbs:

- **Trips** — `create_trip`, `list_trips`, `get_trip`, `get_active_trip`,
  `start_trip`, `end_trip`, `trip_stats`.
- **Locations** — `submit_location`, `list_locations`, `trip_route`.
- **Maps** — `map_search`, `map_reverse`, `map_route`, `navigate_to`,
  `map_poi`.
- **Diary** — `list_diary`, `get_diary`, `search_diary`, `generate_diary`.
- **Planning** — `plan_trip`.
- **Artifact cards** — `show_artifact`, `update_artifact`.

Each entry is shown as `{"action":"<name>","params":{…}}`, matching the strict
tool protocol in the system prompt. Since the model is told to emit **exactly
one raw JSON action per turn**, the skill keeps the contract compact.

The persona fragment set by the plugin is inserted as
`You are <ai_name>, <persona>.`; see
[`src/plugin.rs`](../src/plugin.rs).
