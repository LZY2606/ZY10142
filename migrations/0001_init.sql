CREATE TABLE IF NOT EXISTS devices (
    id TEXT PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    sample_rate_hz INTEGER NOT NULL CHECK (sample_rate_hz > 0)
);

CREATE TABLE IF NOT EXISTS import_batches (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL REFERENCES devices(id),
    fingerprint TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS records (
    id TEXT PRIMARY KEY,
    batch_id TEXT NOT NULL REFERENCES import_batches(id),
    device_id TEXT NOT NULL REFERENCES devices(id),
    import_seq INTEGER NOT NULL,
    source_seq INTEGER NOT NULL,
    source_name TEXT NOT NULL,
    sample_rate_hz INTEGER NOT NULL CHECK (sample_rate_hz > 0),
    start_local_ns INTEGER,
    fingerprint TEXT NOT NULL,
    UNIQUE(device_id, fingerprint),
    UNIQUE(device_id, import_seq)
);

CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    record_id TEXT NOT NULL REFERENCES records(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL REFERENCES devices(id),
    record_import_seq INTEGER NOT NULL,
    seq INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    label TEXT NOT NULL,
    local_ns INTEGER NOT NULL,
    frequency_hz REAL,
    phasor_magnitude REAL,
    phasor_angle_deg REAL,
    UNIQUE(record_id, seq)
);

CREATE INDEX IF NOT EXISTS idx_events_capture
ON events(device_id, record_import_seq, seq);

CREATE TABLE IF NOT EXISTS disturbances (
    id TEXT PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('open', 'closed'))
);

CREATE TABLE IF NOT EXISTS memberships (
    disturbance_id TEXT NOT NULL REFERENCES disturbances(id),
    record_id TEXT NOT NULL REFERENCES records(id),
    status TEXT NOT NULL CHECK (status IN ('candidate', 'published')),
    published_version_id TEXT,
    PRIMARY KEY (disturbance_id, record_id)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_one_published_record
ON memberships(record_id)
WHERE status = 'published';

CREATE TABLE IF NOT EXISTS alignment_versions (
    id TEXT PRIMARY KEY,
    disturbance_id TEXT NOT NULL REFERENCES disturbances(id),
    version_no INTEGER NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    published_ns INTEGER NOT NULL,
    UNIQUE(disturbance_id, version_no)
);

CREATE TABLE IF NOT EXISTS alignment_segments (
    id TEXT PRIMARY KEY,
    version_id TEXT NOT NULL REFERENCES alignment_versions(id) ON DELETE CASCADE,
    device_id TEXT NOT NULL REFERENCES devices(id),
    ordinal INTEGER NOT NULL,
    start_event_id TEXT NOT NULL REFERENCES events(id),
    end_event_id TEXT NOT NULL REFERENCES events(id),
    start_record_seq INTEGER NOT NULL,
    start_event_seq INTEGER NOT NULL,
    end_record_seq INTEGER NOT NULL,
    end_event_seq INTEGER NOT NULL,
    start_local_ns INTEGER NOT NULL,
    end_local_ns INTEGER NOT NULL,
    start_corrected_ns INTEGER NOT NULL,
    end_corrected_ns INTEGER NOT NULL,
    start_radius_ns INTEGER NOT NULL DEFAULT 0 CHECK (start_radius_ns >= 0),
    end_radius_ns INTEGER NOT NULL DEFAULT 0 CHECK (end_radius_ns >= 0),
    trusted INTEGER NOT NULL CHECK (trusted IN (0, 1)),
    UNIQUE(version_id, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_segments_version
ON alignment_segments(version_id);

CREATE TABLE IF NOT EXISTS event_timings (
    event_id TEXT NOT NULL REFERENCES events(id),
    version_id TEXT NOT NULL REFERENCES alignment_versions(id) ON DELETE CASCADE,
    segment_id TEXT NOT NULL REFERENCES alignment_segments(id),
    corrected_ns INTEGER NOT NULL,
    min_ns INTEGER NOT NULL,
    max_ns INTEGER NOT NULL,
    PRIMARY KEY (event_id, version_id),
    CHECK (min_ns <= corrected_ns AND corrected_ns <= max_ns)
);

CREATE TABLE IF NOT EXISTS alignment_records (
    version_id TEXT NOT NULL REFERENCES alignment_versions(id) ON DELETE CASCADE,
    record_id TEXT NOT NULL REFERENCES records(id),
    PRIMARY KEY (version_id, record_id)
);
