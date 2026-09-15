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
    comment    TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS entries_date ON entries(date);
CREATE TABLE IF NOT EXISTS session (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    date       TEXT NOT NULL,
    start_min  INTEGER NOT NULL,
    project_id INTEGER
);
INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', '1');
