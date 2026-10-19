CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    name TEXT,
    state TEXT NOT NULL,
    workspace_id TEXT,
    device_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    body TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_name ON sessions(name);
CREATE TABLE IF NOT EXISTS aliases (
    alias INTEGER PRIMARY KEY CHECK(alias > 0),
    session_id TEXT NOT NULL UNIQUE REFERENCES sessions(id)
);
CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    owner_session_id TEXT NOT NULL,
    removed INTEGER NOT NULL DEFAULT 0 CHECK(removed IN (0, 1)),
    body TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS edges (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    target TEXT NOT NULL,
    kind TEXT NOT NULL,
    body TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    sender TEXT,
    recipient TEXT NOT NULL,
    body TEXT,
    sent_at_ms INTEGER,
    delivered_at_ms INTEGER,
    read_at_ms INTEGER
);
CREATE TABLE IF NOT EXISTS applied_events (
    event_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    entry TEXT NOT NULL,
    UNIQUE(session_id, seq)
);
CREATE INDEX IF NOT EXISTS applied_callback ON applied_events(
    session_id, json_extract(entry, '$.type'), json_extract(entry, '$.body.callback_id'), seq
);

-- Equality on type and session also supplies sequence order for latest-event queries.
CREATE INDEX IF NOT EXISTS applied_type ON applied_events(
    json_extract(entry, '$.type'), session_id, seq
);
