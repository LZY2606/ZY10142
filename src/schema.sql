CREATE TABLE IF NOT EXISTS import_batches (
    batch_id INTEGER PRIMARY KEY,
    imported_at_ns INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS records (
    record_id TEXT PRIMARY KEY,
    batch_id INTEGER NOT NULL REFERENCES import_batches(batch_id),
    device_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    local_time_ns INTEGER NOT NULL,
    sample_rate_hz REAL,
    digital_changes_json TEXT NOT NULL DEFAULT '[]',
    phasor_features_json TEXT NOT NULL DEFAULT '[]',
    raw_json TEXT NOT NULL,
    UNIQUE(device_id, fingerprint)
);

CREATE TABLE IF NOT EXISTS record_events (
    event_id TEXT PRIMARY KEY,
    record_id TEXT NOT NULL REFERENCES records(record_id),
    key TEXT NOT NULL,
    kind TEXT NOT NULL,
    local_time_ns INTEGER NOT NULL,
    uncertainty_ns INTEGER NOT NULL DEFAULT 0 CHECK(uncertainty_ns >= 0),
    channel TEXT,
    label TEXT,
    UNIQUE(record_id, key)
);

CREATE TABLE IF NOT EXISTS cases (
    case_id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    created_at_ns INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS case_candidates (
    case_id INTEGER NOT NULL REFERENCES cases(case_id),
    record_id TEXT NOT NULL REFERENCES records(record_id),
    added_at_ns INTEGER NOT NULL,
    PRIMARY KEY(case_id, record_id)
);

CREATE TABLE IF NOT EXISTS published_membership (
    record_id TEXT PRIMARY KEY REFERENCES records(record_id),
    case_id INTEGER NOT NULL REFERENCES cases(case_id),
    version_id INTEGER NOT NULL,
    published_at_ns INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS anchors (
    anchor_key TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    corrected_time_ns INTEGER,
    uncertainty_ns INTEGER NOT NULL DEFAULT 0 CHECK(uncertainty_ns >= 0),
    trusted INTEGER NOT NULL DEFAULT 1 CHECK(trusted IN (0, 1))
);

CREATE TABLE IF NOT EXISTS anchor_events (
    anchor_key TEXT NOT NULL REFERENCES anchors(anchor_key),
    event_id TEXT NOT NULL REFERENCES record_events(event_id),
    PRIMARY KEY(anchor_key, event_id),
    UNIQUE(event_id)
);

CREATE TABLE IF NOT EXISTS versions (
    version_id INTEGER PRIMARY KEY,
    case_id INTEGER NOT NULL REFERENCES cases(case_id),
    version_no INTEGER NOT NULL,
    note TEXT NOT NULL,
    created_at_ns INTEGER NOT NULL,
    published_at_ns INTEGER,
    UNIQUE(case_id, version_no)
);

CREATE TABLE IF NOT EXISTS version_records (
    version_id INTEGER NOT NULL REFERENCES versions(version_id),
    record_id TEXT NOT NULL REFERENCES records(record_id),
    PRIMARY KEY(version_id, record_id)
);

CREATE TABLE IF NOT EXISTS clock_segments (
    segment_id INTEGER PRIMARY KEY,
    version_id INTEGER NOT NULL REFERENCES versions(version_id),
    device_id TEXT NOT NULL,
    segment_index INTEGER NOT NULL,
    start_ns INTEGER,
    end_ns INTEGER,
    intercept_num TEXT NOT NULL,
    intercept_den TEXT NOT NULL,
    slope_num TEXT NOT NULL,
    slope_den TEXT NOT NULL,
    uncertainty_ns INTEGER NOT NULL CHECK(uncertainty_ns >= 0),
    UNIQUE(version_id, device_id, segment_index)
);

CREATE TABLE IF NOT EXISTS version_anchors (
    version_id INTEGER NOT NULL REFERENCES versions(version_id),
    anchor_key TEXT NOT NULL REFERENCES anchors(anchor_key),
    event_id TEXT NOT NULL REFERENCES record_events(event_id),
    device_id TEXT NOT NULL,
    local_time_ns INTEGER NOT NULL,
    corrected_time_ns INTEGER NOT NULL,
    uncertainty_ns INTEGER NOT NULL,
    PRIMARY KEY(version_id, anchor_key, event_id)
);

CREATE TABLE IF NOT EXISTS reviews (
    review_id INTEGER PRIMARY KEY,
    version_id INTEGER NOT NULL REFERENCES versions(version_id),
    conclusion TEXT NOT NULL,
    created_at_ns INTEGER NOT NULL
);
