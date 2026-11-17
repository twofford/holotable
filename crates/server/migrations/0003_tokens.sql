-- Tokens on scenes, and the campaign's active scene (milestone M4).

CREATE TABLE tokens (
    id          INTEGER PRIMARY KEY,
    -- Deleting a scene deletes its tokens.
    scene_id    INTEGER NOT NULL REFERENCES scenes (id) ON DELETE CASCADE,
    name        TEXT    NOT NULL,
    -- The picture to draw. An image in use can't be deleted (no ON DELETE).
    asset_id    INTEGER REFERENCES assets (id),
    -- Top-left corner, in the scene's grid squares.
    x           REAL    NOT NULL,
    y           REAL    NOT NULL,
    size        INTEGER NOT NULL CHECK (size BETWEEN 1 AND 4),
    -- The player who may move it. If their account is deleted, nobody owns it.
    owner_id    INTEGER REFERENCES users (id) ON DELETE SET NULL,
    -- SQLite has no boolean type: 0 or 1.
    hidden      INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1)),
    created_at  INTEGER NOT NULL
);
CREATE INDEX tokens_scene_id ON tokens (scene_id);
CREATE INDEX tokens_asset_id ON tokens (asset_id);

-- The scene the GM has put everyone on (MAP-010). NULL: none chosen.
ALTER TABLE campaigns ADD COLUMN active_scene_id INTEGER REFERENCES scenes (id) ON DELETE SET NULL;
