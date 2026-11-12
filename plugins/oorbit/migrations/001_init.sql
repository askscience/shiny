-- Oorbit plugin: one planet per user. The whole simulation state is one JSON
-- document (oorbit_sim::Planet), advanced to "now" and written back on every
-- read and action; the simulation is deterministic, so this is all it needs.
CREATE TABLE IF NOT EXISTS oorbit_planets (
    user_id TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
