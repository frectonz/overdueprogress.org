CREATE TABLE submission_edits (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    submission_id INTEGER NOT NULL REFERENCES submissions(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    author TEXT NOT NULL,
    email TEXT NOT NULL,
    link TEXT NOT NULL,
    edit_kind TEXT NOT NULL,
    reverted_from INTEGER REFERENCES submission_edits(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX submission_edits_sub ON submission_edits (submission_id, id DESC);
