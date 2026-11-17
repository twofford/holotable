-- Campaigns, members, invites, uploaded assets and scenes (milestone M3).

CREATE TABLE campaigns (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL,
    created_by  INTEGER NOT NULL REFERENCES users (id),
    created_at  INTEGER NOT NULL
);

-- Who belongs to which campaign, and as what. Roles are per campaign, so the
-- same user can be a GM here and a player there (spec: Tenancy).
CREATE TABLE memberships (
    campaign_id INTEGER NOT NULL REFERENCES campaigns (id) ON DELETE CASCADE,
    user_id     INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- CHECK makes the database itself refuse any other value.
    role        TEXT    NOT NULL CHECK (role IN ('gm', 'player')),
    joined_at   INTEGER NOT NULL,
    PRIMARY KEY (campaign_id, user_id)
);
CREATE INDEX memberships_user_id ON memberships (user_id);

CREATE TABLE invites (
    id          INTEGER PRIMARY KEY,
    campaign_id INTEGER NOT NULL REFERENCES campaigns (id) ON DELETE CASCADE,
    -- SHA-256 of the token in the link, like sessions (spec CMP-004).
    token_hash  BLOB    NOT NULL UNIQUE,
    role        TEXT    NOT NULL CHECK (role IN ('gm', 'player')),
    created_by  INTEGER NOT NULL REFERENCES users (id),
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    max_uses    INTEGER NOT NULL CHECK (max_uses >= 1),
    uses        INTEGER NOT NULL DEFAULT 0,
    revoked_at  INTEGER
);
CREATE INDEX invites_campaign_id ON invites (campaign_id);

-- Uploaded files. The bytes live on disk under the data folder; this table
-- holds what the server learned about them when they were checked.
CREATE TABLE assets (
    id            INTEGER PRIMARY KEY,
    campaign_id   INTEGER NOT NULL REFERENCES campaigns (id) ON DELETE CASCADE,
    kind          TEXT    NOT NULL CHECK (kind IN ('uvtt', 'image')),
    -- The uploader's file name, for display only (spec AST-006).
    original_name TEXT    NOT NULL,
    -- Bytes on disk, counted against the campaign's quota.
    size_bytes    INTEGER NOT NULL,
    image_format  TEXT    NOT NULL CHECK (image_format IN ('png', 'jpeg', 'webp')),
    width         INTEGER NOT NULL,
    height        INTEGER NOT NULL,
    -- 0 when the picture is served whole; otherwise how many tiles it was cut into.
    tile_count    INTEGER NOT NULL DEFAULT 0,
    created_by    INTEGER NOT NULL REFERENCES users (id),
    created_at    INTEGER NOT NULL
);
CREATE INDEX assets_campaign_id ON assets (campaign_id);

-- A scene is a map (a UVTT asset) plus its own settings. Later milestones add
-- tokens, door states and fog. Assets never change after upload, so a scene
-- reads walls, doors and lights from its asset instead of copying them.
CREATE TABLE scenes (
    id              INTEGER PRIMARY KEY,
    campaign_id     INTEGER NOT NULL REFERENCES campaigns (id) ON DELETE CASCADE,
    -- No ON DELETE here: an asset that a scene uses can't be deleted (AST-008).
    asset_id        INTEGER NOT NULL REFERENCES assets (id),
    name            TEXT    NOT NULL,
    -- The grid, in image pixels. Starts as the UVTT file's; the GM can adjust it.
    pixels_per_grid REAL    NOT NULL CHECK (pixels_per_grid > 0),
    grid_offset_x   REAL    NOT NULL DEFAULT 0,
    grid_offset_y   REAL    NOT NULL DEFAULT 0,
    created_at      INTEGER NOT NULL
);
CREATE INDEX scenes_campaign_id ON scenes (campaign_id);
CREATE INDEX scenes_asset_id ON scenes (asset_id);
