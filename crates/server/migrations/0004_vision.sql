-- Vision, doors and explored fog (milestone M5).

-- Each token's sight (VIS-003). Existing tokens can see, without a limit.
ALTER TABLE tokens ADD COLUMN vision INTEGER NOT NULL DEFAULT 1 CHECK (vision IN (0, 1));
-- In feet; NULL means unlimited.
ALTER TABLE tokens ADD COLUMN vision_range REAL CHECK (vision_range IS NULL OR vision_range > 0);

-- Doors whose state differs from the map file, or has ever been changed.
-- `door` is the index in the map's `portals` list.
CREATE TABLE scene_doors (
    scene_id  INTEGER NOT NULL REFERENCES scenes (id) ON DELETE CASCADE,
    door      INTEGER NOT NULL CHECK (door >= 0),
    open      INTEGER NOT NULL CHECK (open IN (0, 1)),
    locked    INTEGER NOT NULL CHECK (locked IN (0, 1)),
    PRIMARY KEY (scene_id, door)
);

-- What each player has seen of each scene (VIS-005): one bit per cell of
-- `ExploredLayout::for_map`, at most 512 × 512 cells (32 KB).
CREATE TABLE scene_explored (
    scene_id  INTEGER NOT NULL REFERENCES scenes (id) ON DELETE CASCADE,
    user_id   INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    bits      BLOB    NOT NULL,
    PRIMARY KEY (scene_id, user_id)
);
