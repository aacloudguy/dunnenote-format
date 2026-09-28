-- SPDX-License-Identifier: MIT
-- Copyright (c) 2026 Dunne, Corp.
--
-- DunneNote Format schema, PRAGMA user_version = 18 (format_version "0.18.0").
-- GENERATED from a freshly created bundle by
-- crates/dunnenote-svc-file/tests/format_schema_dump.rs. Do not edit by hand.
-- Apply in one transaction to an empty database, then set PRAGMA user_version = 18.

CREATE TABLE blobs(
    hash        TEXT PRIMARY KEY NOT NULL
                  CHECK (length(hash) = 64)
                  CHECK (lower(hash) = hash)
                  CHECK (hash NOT GLOB '*[^0-9a-f]*'),
    size_bytes  INTEGER NOT NULL CHECK (size_bytes >= 0),
    refcount    INTEGER NOT NULL DEFAULT 0 CHECK (refcount >= 0),
    deleted_at  INTEGER          CHECK (deleted_at IS NULL OR deleted_at >= 0),
    created_at  INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at  INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT;

CREATE TABLE calendar_event_attendees(
    id         TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    event_id   TEXT NOT NULL REFERENCES calendar_events(id) ON DELETE CASCADE,
    ordinal    INTEGER NOT NULL CHECK (ordinal >= 0),
    value      TEXT NOT NULL DEFAULT '',
    cn         TEXT,
    role       TEXT,
    partstat   TEXT,
    rsvp       INTEGER NOT NULL DEFAULT 0 CHECK (rsvp IN (0,1)),
    created_at INTEGER NOT NULL CHECK (created_at > 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT;

CREATE TABLE "calendar_events"(
    id           TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    instance_id  TEXT NOT NULL REFERENCES "canvas_instances"(id) ON DELETE CASCADE,
    uid          TEXT,
    summary      TEXT NOT NULL DEFAULT '',
    location     TEXT NOT NULL DEFAULT '',
    description  TEXT NOT NULL DEFAULT '',
    start_utc    INTEGER NOT NULL,
    end_utc      INTEGER NOT NULL CHECK (end_utc >= start_utc),
    all_day      INTEGER NOT NULL DEFAULT 0 CHECK (all_day IN (0,1)),
    tzid         TEXT,
    created_at   INTEGER NOT NULL CHECK (created_at > 0),
    updated_at   INTEGER NOT NULL CHECK (updated_at >= created_at)
, source_ordinal INTEGER, url TEXT, organizer_value TEXT, organizer_cn TEXT, status TEXT, categories TEXT, rrule_text TEXT, attachments TEXT, dtstamp_utc INTEGER, last_modified_utc INTEGER, sequence_no INTEGER) STRICT;

CREATE TABLE "canvas_instances"(
    id             TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    page_id        TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    kind           TEXT NOT NULL CHECK (kind IN ('picture','calendar','database','spreadsheet','rich_text','sketch')),
    source_hash    TEXT NOT NULL REFERENCES blobs(hash),
    x              INTEGER NOT NULL,
    y              INTEGER NOT NULL,
    width          INTEGER NOT NULL CHECK (width > 0),
    height         INTEGER NOT NULL CHECK (height > 0),
    z_index        INTEGER NOT NULL DEFAULT 0,
    settings       TEXT NOT NULL DEFAULT '{}',
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    created_at     INTEGER NOT NULL CHECK (created_at > 0),
    updated_at     INTEGER NOT NULL CHECK (updated_at >= created_at),
    group_id       TEXT REFERENCES groups(id) ON DELETE SET NULL
, lifecycle TEXT NOT NULL DEFAULT 'active', lifecycle_reason TEXT, lifecycle_note TEXT, lifecycle_at INTEGER, z_minor INTEGER NOT NULL DEFAULT 0 CHECK (z_minor >= 0)) STRICT;

CREATE TABLE "dataset_columns"(
    id          TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    dataset_id  TEXT NOT NULL REFERENCES "datasets"(id) ON DELETE CASCADE,
    col_key     TEXT NOT NULL CHECK (length(col_key) > 0),
    name        TEXT NOT NULL DEFAULT '',
    type_hint   TEXT NOT NULL DEFAULT 'text'
                  CHECK (type_hint IN ('text','number','date','boolean','unknown')),
    position    INTEGER NOT NULL CHECK (position >= 0),
    created_at  INTEGER NOT NULL CHECK (created_at > 0),
    updated_at  INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT;

CREATE TABLE "dataset_rows"(
    id          TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    dataset_id  TEXT NOT NULL REFERENCES "datasets"(id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL CHECK (seq >= 0),
    cells       TEXT NOT NULL DEFAULT '{}'
                  CHECK (json_valid(cells) AND json_type(cells) = 'object'),
    created_at  INTEGER NOT NULL CHECK (created_at > 0),
    updated_at  INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT;

CREATE TABLE "datasets"(
    id              TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    instance_id     TEXT NOT NULL
                      REFERENCES "canvas_instances"(id) ON DELETE CASCADE,
    shape           TEXT NOT NULL DEFAULT 'table'
                      CHECK (shape IN ('table','list')),
    source_kind     TEXT NOT NULL,
    row_count       INTEGER NOT NULL DEFAULT 0 CHECK (row_count >= 0),
    schema_version  INTEGER NOT NULL CHECK (schema_version > 0),
    created_at      INTEGER NOT NULL CHECK (created_at > 0),
    updated_at      INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT;

CREATE TABLE groups(
    id              TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    page_id         TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    parent_group_id TEXT REFERENCES groups(id) ON DELETE CASCADE,
    settings        TEXT NOT NULL DEFAULT '{}',
    schema_version  INTEGER NOT NULL CHECK (schema_version > 0),
    created_at      INTEGER NOT NULL CHECK (created_at > 0),
    updated_at      INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT;

CREATE TABLE item_meta(
    id           TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    source_kind  TEXT NOT NULL CHECK (length(source_kind) > 0),
    source_id    TEXT NOT NULL CHECK (length(source_id) > 0),
    key          TEXT NOT NULL CHECK (length(key) > 0 AND length(key) <= 255),
    value_text   TEXT,
    value_folded TEXT,
    value_num    REAL,
    value_num2   REAL,
    source       TEXT NOT NULL DEFAULT 'user' CHECK (length(source) > 0),
    created_at   INTEGER NOT NULL CHECK (created_at > 0),
    updated_at   INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT;

CREATE TABLE item_tags(
    id           TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    tag_id       TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    source_kind  TEXT NOT NULL CHECK (length(source_kind) > 0),
    source_id    TEXT NOT NULL CHECK (length(source_id) > 0),
    created_at   INTEGER NOT NULL CHECK (created_at > 0)
) STRICT;

CREATE TABLE nodes(
    id           TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    kind         TEXT NOT NULL CHECK (kind IN ('notebook','group','page')),
    parent_id    TEXT REFERENCES nodes(id) ON DELETE CASCADE,
    name         TEXT NOT NULL CHECK (
                     length(name) > 0
                     AND length(name) <= 255
                     AND instr(name, char(0)) = 0
                     AND instr(name, char(10)) = 0
                     AND instr(name, char(13)) = 0
                 ),
    position     TEXT NOT NULL CHECK (length(position) > 0 AND length(position) <= 256),
    child_count  INTEGER NOT NULL DEFAULT 0 CHECK (child_count >= 0),
    is_template  INTEGER NOT NULL DEFAULT 0 CHECK (is_template IN (0,1)),
    created_at   INTEGER NOT NULL CHECK (created_at > 0),
    updated_at   INTEGER NOT NULL CHECK (updated_at >= created_at)
, is_archived INTEGER NOT NULL DEFAULT 0 CHECK (is_archived IN (0,1)), archive_reason TEXT, archive_note TEXT, archived_at INTEGER, settings TEXT) STRICT;

CREATE TABLE "rich_text_instances"(
    instance_id    TEXT PRIMARY KEY NOT NULL REFERENCES "canvas_instances"(id) ON DELETE CASCADE,
    data           TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    created_at     INTEGER NOT NULL CHECK (created_at > 0),
    updated_at     INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT;

CREATE TABLE search_index(
    id           INTEGER PRIMARY KEY,
    source_kind  TEXT NOT NULL,
    source_id    TEXT NOT NULL,
    scope        TEXT NOT NULL,
    field        TEXT NOT NULL,
    text         TEXT NOT NULL
) STRICT;

CREATE VIRTUAL TABLE search_index_fts USING fts5(
    source_kind UNINDEXED,
    scope UNINDEXED,
    field UNINDEXED,
    text,
    content='search_index',
    content_rowid='id',
    tokenize="unicode61 remove_diacritics 2 tokenchars '_'",
    prefix='2 3'
);

CREATE VIRTUAL TABLE search_index_trgm USING fts5(
    text,
    content='search_index',
    content_rowid='id',
    tokenize='trigram'
);

CREATE TABLE "sketch_instances"(
    instance_id    TEXT PRIMARY KEY NOT NULL REFERENCES "canvas_instances"(id) ON DELETE CASCADE,
    data           TEXT NOT NULL,
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    created_at     INTEGER NOT NULL CHECK (created_at > 0),
    updated_at     INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT;

CREATE TABLE tag_aliases(
    id            TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    alias_name    TEXT NOT NULL CHECK (
                      length(alias_name) > 0
                      AND length(alias_name) <= 255
                      AND instr(alias_name, char(0)) = 0
                      AND instr(alias_name, char(10)) = 0
                      AND instr(alias_name, char(13)) = 0
                  ),
    alias_folded  TEXT NOT NULL CHECK (length(alias_folded) > 0),
    tag_id        TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    created_at    INTEGER NOT NULL CHECK (created_at > 0)
) STRICT;

CREATE TABLE tags(
    id           TEXT PRIMARY KEY NOT NULL CHECK (length(id) = 36),
    name         TEXT NOT NULL CHECK (
                     length(name) > 0
                     AND length(name) <= 255
                     AND instr(name, char(0)) = 0
                     AND instr(name, char(10)) = 0
                     AND instr(name, char(13)) = 0
                 ),
    name_folded  TEXT NOT NULL CHECK (length(name_folded) > 0),
    color        TEXT,
    description  TEXT,
    created_at   INTEGER NOT NULL CHECK (created_at > 0),
    updated_at   INTEGER NOT NULL CHECK (updated_at >= created_at)
) STRICT;

CREATE INDEX blobs_gc_idx ON blobs(hash) WHERE refcount = 0;

CREATE INDEX calendar_event_attendees_event_id_idx
    ON calendar_event_attendees(event_id, ordinal);

CREATE INDEX calendar_events_instance_id_idx ON calendar_events(instance_id);

CREATE INDEX calendar_events_range_idx
    ON calendar_events(instance_id, start_utc, end_utc);

CREATE INDEX canvas_instances_group_id_idx ON canvas_instances(group_id);

CREATE INDEX canvas_instances_layer_idx ON canvas_instances(page_id, z_index, z_minor);

CREATE INDEX canvas_instances_lifecycle_idx ON canvas_instances(lifecycle);

CREATE INDEX canvas_instances_page_id_idx ON canvas_instances(page_id);

CREATE INDEX canvas_instances_source_hash_idx ON canvas_instances(source_hash);

CREATE INDEX dataset_columns_dataset_pos_idx ON dataset_columns(dataset_id, position);

CREATE UNIQUE INDEX dataset_columns_key_unique_idx ON dataset_columns(dataset_id, col_key);

CREATE INDEX dataset_rows_dataset_seq_idx ON dataset_rows(dataset_id, seq);

CREATE UNIQUE INDEX datasets_instance_id_unique_idx ON datasets(instance_id);

CREATE INDEX groups_page_id_idx ON groups(page_id);

CREATE INDEX groups_parent_group_id_idx ON groups(parent_group_id);

CREATE INDEX item_meta_key_folded_idx ON item_meta(key, value_folded);

CREATE INDEX item_meta_key_num_idx ON item_meta(key, value_num);

CREATE INDEX item_meta_source_idx ON item_meta(source_kind, source_id);

CREATE INDEX item_tags_source_idx ON item_tags(source_kind, source_id);

CREATE UNIQUE INDEX item_tags_tag_source_unique_idx
    ON item_tags(tag_id, source_kind, source_id);

CREATE INDEX nodes_kind_idx ON nodes(kind);

CREATE UNIQUE INDEX nodes_parent_position_unique_idx ON nodes(parent_id, position);

CREATE INDEX search_index_scope_idx ON search_index(scope);

CREATE UNIQUE INDEX search_index_source_field_unique_idx
    ON search_index(source_kind, source_id, field);

CREATE INDEX search_index_source_idx ON search_index(source_kind, source_id);

CREATE UNIQUE INDEX tag_aliases_folded_unique_idx ON tag_aliases(alias_folded);

CREATE INDEX tag_aliases_tag_id_idx ON tag_aliases(tag_id);

CREATE UNIQUE INDEX tags_name_folded_unique_idx ON tags(name_folded);

CREATE TRIGGER blobs_touch_updated_at
AFTER UPDATE ON blobs
WHEN NEW.updated_at IS OLD.updated_at AND NEW.updated_at <> unixepoch()
BEGIN
    UPDATE blobs SET updated_at = unixepoch() WHERE rowid = NEW.rowid;
END;

CREATE TRIGGER calendar_event_attendees_touch_updated_at
AFTER UPDATE ON calendar_event_attendees
WHEN NEW.updated_at IS OLD.updated_at AND NEW.updated_at <> unixepoch()
BEGIN
    UPDATE calendar_event_attendees SET updated_at = unixepoch() WHERE rowid = NEW.rowid;
END;

CREATE TRIGGER calendar_events_touch_updated_at
AFTER UPDATE ON calendar_events
WHEN NEW.updated_at IS OLD.updated_at AND NEW.updated_at <> unixepoch()
BEGIN
    UPDATE calendar_events SET updated_at = unixepoch() WHERE rowid = NEW.rowid;
END;

CREATE TRIGGER canvas_instances_after_delete
AFTER DELETE ON canvas_instances
BEGIN
    UPDATE blobs
        SET refcount   = refcount - 1,
            deleted_at = CASE WHEN refcount - 1 = 0 THEN unixepoch() ELSE deleted_at END,
            updated_at = unixepoch()
        WHERE hash = OLD.source_hash;
END;

CREATE TRIGGER canvas_instances_after_insert
AFTER INSERT ON canvas_instances
BEGIN
    UPDATE blobs
        SET refcount   = refcount + 1,
            deleted_at = NULL,
            updated_at = unixepoch()
        WHERE hash = NEW.source_hash;
END;

CREATE TRIGGER canvas_instances_guard_source_immutable
AFTER UPDATE OF source_hash ON canvas_instances
WHEN NEW.source_hash <> OLD.source_hash
BEGIN
    SELECT RAISE(ABORT, 'canvas_instances.source_hash is immutable');
END;

CREATE TRIGGER canvas_instances_touch_updated_at
AFTER UPDATE ON canvas_instances
WHEN NEW.updated_at IS OLD.updated_at AND NEW.updated_at <> unixepoch()
BEGIN
    UPDATE canvas_instances SET updated_at = unixepoch() WHERE rowid = NEW.rowid;
END;

CREATE TRIGGER dataset_columns_touch_updated_at
AFTER UPDATE ON dataset_columns
WHEN NEW.updated_at IS OLD.updated_at AND NEW.updated_at <> unixepoch()
BEGIN
    UPDATE dataset_columns SET updated_at = unixepoch() WHERE rowid = NEW.rowid;
END;

CREATE TRIGGER dataset_rows_touch_updated_at
AFTER UPDATE ON dataset_rows
WHEN NEW.updated_at IS OLD.updated_at AND NEW.updated_at <> unixepoch()
BEGIN
    UPDATE dataset_rows SET updated_at = unixepoch() WHERE rowid = NEW.rowid;
END;

CREATE TRIGGER datasets_touch_updated_at
AFTER UPDATE ON datasets
WHEN NEW.updated_at IS OLD.updated_at AND NEW.updated_at <> unixepoch()
BEGIN
    UPDATE datasets SET updated_at = unixepoch() WHERE rowid = NEW.rowid;
END;

CREATE TRIGGER groups_touch_updated_at
AFTER UPDATE ON groups
WHEN NEW.updated_at IS OLD.updated_at AND NEW.updated_at <> unixepoch()
BEGIN
    UPDATE groups SET updated_at = unixepoch() WHERE rowid = NEW.rowid;
END;

CREATE TRIGGER item_meta_touch_updated_at
AFTER UPDATE ON item_meta
WHEN NEW.updated_at IS OLD.updated_at AND NEW.updated_at <> unixepoch()
BEGIN
    UPDATE item_meta SET updated_at = unixepoch() WHERE rowid = NEW.rowid;
END;

CREATE TRIGGER nodes_child_count_delete
AFTER DELETE ON nodes
WHEN OLD.parent_id IS NOT NULL
BEGIN
    UPDATE nodes
        SET child_count = child_count - 1,
            updated_at  = unixepoch()
        WHERE id = OLD.parent_id;
END;

CREATE TRIGGER nodes_child_count_insert
AFTER INSERT ON nodes
WHEN NEW.parent_id IS NOT NULL
BEGIN
    UPDATE nodes
        SET child_count = child_count + 1,
            updated_at  = NEW.created_at
        WHERE id = NEW.parent_id;
END;

CREATE TRIGGER nodes_child_count_update_parent
AFTER UPDATE OF parent_id ON nodes
WHEN
    (OLD.parent_id IS NULL AND NEW.parent_id IS NOT NULL)
    OR (OLD.parent_id IS NOT NULL AND NEW.parent_id IS NULL)
    OR (OLD.parent_id IS NOT NULL
        AND NEW.parent_id IS NOT NULL
        AND OLD.parent_id != NEW.parent_id)
BEGIN
    UPDATE nodes
        SET child_count = child_count - 1,
            updated_at  = unixepoch()
        WHERE id = OLD.parent_id;
    UPDATE nodes
        SET child_count = child_count + 1,
            updated_at  = unixepoch()
        WHERE id = NEW.parent_id;
END;

CREATE TRIGGER nodes_parent_kind_check_insert
BEFORE INSERT ON nodes
BEGIN
    SELECT CASE
        WHEN NEW.kind = 'notebook' AND NEW.parent_id IS NOT NULL
            THEN RAISE(ABORT, 'parent_kind_violation: notebook must be root')
        WHEN NEW.kind = 'group' AND NEW.parent_id IS NULL
            THEN RAISE(ABORT, 'parent_kind_violation: group requires parent')
        WHEN NEW.kind = 'page' AND NEW.parent_id IS NULL
            THEN RAISE(ABORT, 'parent_kind_violation: page requires parent')
        WHEN NEW.parent_id IS NOT NULL
             AND (SELECT kind FROM nodes WHERE id = NEW.parent_id) = 'page'
            THEN RAISE(ABORT, 'parent_kind_violation: page is a leaf')
    END;
END;

CREATE TRIGGER nodes_parent_kind_check_update
BEFORE UPDATE OF parent_id, kind ON nodes
BEGIN
    SELECT CASE
        WHEN NEW.kind = 'notebook' AND NEW.parent_id IS NOT NULL
            THEN RAISE(ABORT, 'parent_kind_violation: notebook must be root')
        WHEN NEW.kind != 'notebook' AND NEW.parent_id IS NULL
            THEN RAISE(ABORT, 'parent_kind_violation: non-notebook requires parent')
        WHEN NEW.parent_id IS NOT NULL
             AND (SELECT kind FROM nodes WHERE id = NEW.parent_id) = 'page'
            THEN RAISE(ABORT, 'parent_kind_violation: page is a leaf')
    END;
END;

CREATE TRIGGER nodes_touch_updated_at
AFTER UPDATE ON nodes
WHEN NEW.updated_at IS OLD.updated_at AND NEW.updated_at <> unixepoch()
BEGIN
    UPDATE nodes SET updated_at = unixepoch() WHERE rowid = NEW.rowid;
END;

CREATE TRIGGER rich_text_instances_touch_updated_at
AFTER UPDATE ON rich_text_instances
WHEN NEW.updated_at IS OLD.updated_at AND NEW.updated_at <> unixepoch()
BEGIN
    UPDATE rich_text_instances SET updated_at = unixepoch() WHERE rowid = NEW.rowid;
END;

CREATE TRIGGER search_index_after_delete
AFTER DELETE ON search_index
BEGIN
    INSERT INTO search_index_fts(search_index_fts, rowid, source_kind, scope, field, text)
        VALUES ('delete', old.id, old.source_kind, old.scope, old.field, old.text);
    INSERT INTO search_index_trgm(search_index_trgm, rowid, text)
        VALUES ('delete', old.id, old.text);
END;

CREATE TRIGGER search_index_after_insert
AFTER INSERT ON search_index
BEGIN
    INSERT INTO search_index_fts(rowid, source_kind, scope, field, text)
        VALUES (new.id, new.source_kind, new.scope, new.field, new.text);
    INSERT INTO search_index_trgm(rowid, text)
        VALUES (new.id, new.text);
END;

CREATE TRIGGER search_index_after_update
AFTER UPDATE ON search_index
BEGIN
    INSERT INTO search_index_fts(search_index_fts, rowid, source_kind, scope, field, text)
        VALUES ('delete', old.id, old.source_kind, old.scope, old.field, old.text);
    INSERT INTO search_index_trgm(search_index_trgm, rowid, text)
        VALUES ('delete', old.id, old.text);
    INSERT INTO search_index_fts(rowid, source_kind, scope, field, text)
        VALUES (new.id, new.source_kind, new.scope, new.field, new.text);
    INSERT INTO search_index_trgm(rowid, text)
        VALUES (new.id, new.text);
END;

CREATE TRIGGER sketch_instances_touch_updated_at
AFTER UPDATE ON sketch_instances
WHEN NEW.updated_at IS OLD.updated_at AND NEW.updated_at <> unixepoch()
BEGIN
    UPDATE sketch_instances SET updated_at = unixepoch() WHERE rowid = NEW.rowid;
END;

CREATE TRIGGER tags_touch_updated_at
AFTER UPDATE ON tags
WHEN NEW.updated_at IS OLD.updated_at AND NEW.updated_at <> unixepoch()
BEGIN
    UPDATE tags SET updated_at = unixepoch() WHERE rowid = NEW.rowid;
END;

