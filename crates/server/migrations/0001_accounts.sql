-- Accounts and sign-in sessions (milestone M2).
--
-- Times are Unix seconds (INTEGER). SQLite has no date type, and plain numbers
-- are easy to compare and hard to get wrong across time zones.

CREATE TABLE users (
    id            INTEGER PRIMARY KEY,
    -- Stored lower-cased by the server; UNIQUE stops two accounts per address.
    email         TEXT    NOT NULL UNIQUE,
    -- An argon2id "PHC string": algorithm, parameters, salt and hash in one.
    password_hash TEXT    NOT NULL,
    display_name  TEXT    NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE sessions (
    -- SHA-256 of the token in the user's cookie. The token itself is never
    -- stored, so a copy of this database can't be used to sign in as anyone.
    token_hash  BLOB    PRIMARY KEY,
    -- Deleting a user deletes their sessions.
    user_id     INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL
);

CREATE INDEX sessions_user_id ON sessions (user_id);
CREATE INDEX sessions_expires_at ON sessions (expires_at);
