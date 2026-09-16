CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS projects (
    id          INTEGER PRIMARY KEY,
    name        TEXT UNIQUE NOT NULL,
    color_index INTEGER NOT NULL,
    archived    INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS days (
    date  TEXT PRIMARY KEY,
    kind  TEXT NOT NULL,
    label TEXT
);
CREATE TABLE IF NOT EXISTS entries (
    id         INTEGER PRIMARY KEY,
    date       TEXT NOT NULL,
    start_min  INTEGER NOT NULL,
    end_min    INTEGER NOT NULL,
    project_id INTEGER NOT NULL REFERENCES projects(id),
    comment    TEXT NOT NULL DEFAULT '',
    -- The entry's explicit share of its session's break deduction, in minutes;
    -- NULL leaves it to the default rule (the last entry of the session).
    break_share INTEGER
);
CREATE INDEX IF NOT EXISTS entries_date ON entries(date);
CREATE TABLE IF NOT EXISTS session (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    date       TEXT NOT NULL,
    start_min  INTEGER NOT NULL,
    project_id INTEGER,
    -- 'working' while the clock runs on the project, 'break' while it is paused
    -- on it: the work so far is booked, the project is remembered.
    state      TEXT NOT NULL DEFAULT 'working'
);
INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', '2');
