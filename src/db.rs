use crate::clock::{ClockSegment, map_events};
use crate::model::*;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ImportSummary {
    pub batch_id: String,
    pub inserted_records: usize,
    pub skipped_records: usize,
    pub already_present: bool,
}

pub struct Repository {
    db_path: String,
}

impl Repository {
    pub fn new(db_path: &str) -> Result<Self, String> {
        let repository = Self {
            db_path: db_path.to_string(),
        };
        let connection = repository.connect()?;
        repository.migrate(&connection)?;
        Ok(repository)
    }

    fn connect(&self) -> Result<Connection, String> {
        let connection = if self.db_path == ":memory:" {
            Connection::open_in_memory()
        } else {
            Connection::open(&self.db_path)
        }
        .map_err(|error| error.to_string())?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 PRAGMA journal_mode = WAL;
                 PRAGMA busy_timeout = 5000;",
            )
            .map_err(|error| error.to_string())?;
        Ok(connection)
    }

    fn migrate(&self, connection: &Connection) -> Result<(), String> {
        connection
            .execute_batch(include_str!("../migrations/0001_init.sql"))
            .map_err(|error| error.to_string())
    }

    pub fn connection(&self) -> Result<Connection, String> {
        let connection = self.connect()?;
        self.migrate(&connection)?;
        Ok(connection)
    }

    pub fn import_batch(&self, input: BatchInput) -> Result<ImportSummary, String> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let result = self.import_batch_tx(&tx, input);
        match result {
            Ok(summary) => {
                tx.commit().map_err(|error| error.to_string())?;
                Ok(summary)
            }
            Err(error) => Err(error),
        }
    }

    fn import_batch_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        input: BatchInput,
    ) -> Result<ImportSummary, String> {
        if input.device_code.trim().is_empty() || input.device_name.trim().is_empty() {
            return Err("device code and name are required".to_string());
        }
        if input.sample_rate_hz <= 0 {
            return Err("device sample rate must be positive".to_string());
        }
        if input.records.is_empty() {
            return Err("import batch must contain records".to_string());
        }
        let canonical = canonical_json(&input)?;
        let batch_fingerprint = format!("sha256:{}", hash_text(&canonical));
        let batch_id = stable_id("batch", &batch_fingerprint);
        let device_id = stable_id("device", &input.device_code);

        tx.execute(
            "INSERT OR IGNORE INTO devices(id, code, name, sample_rate_hz)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                device_id,
                input.device_code,
                input.device_name,
                input.sample_rate_hz
            ],
        )
        .map_err(|error| error.to_string())?;

        let existing_batch: Option<String> = tx
            .query_row(
                "SELECT id FROM import_batches WHERE id = ?1",
                params![batch_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT OR IGNORE INTO import_batches(id, device_id, fingerprint)
             VALUES (?1, ?2, ?3)",
            params![batch_id, device_id, batch_fingerprint],
        )
        .map_err(|error| error.to_string())?;

        let mut inserted = 0;
        let mut skipped = 0;
        for record in input.records {
            if record.source_name.trim().is_empty() || record.events.is_empty() {
                return Err("each record needs a source and events".to_string());
            }
            if record.sample_rate_hz <= 0 || record.seq < 0 {
                return Err("record sample rate and sequence must be valid".to_string());
            }
            let record_fingerprint = format!(
                "sha256:{}",
                hash_text(&canonical_json(&RecordFingerprint {
                    source_name: &record.source_name,
                    sample_rate_hz: record.sample_rate_hz,
                    start_local_ns: record.start_local_ns,
                    events: &record.events,
                })?)
            );
            let record_id = stable_id("record", &format!("{}|{}", device_id, record_fingerprint));
            let duplicate: Option<String> = tx
                .query_row(
                    "SELECT id FROM records WHERE device_id = ?1 AND fingerprint = ?2",
                    params![device_id, record_fingerprint],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if duplicate.is_some() {
                skipped += 1;
                continue;
            }
            tx.execute(
                "INSERT INTO records(id, batch_id, device_id, import_seq, source_seq, source_name,
                                     sample_rate_hz, start_local_ns, fingerprint)
                 VALUES (?1, ?2, ?3, COALESCE((SELECT MAX(import_seq) + 1 FROM records
                                              WHERE device_id = ?3), 0),
                         ?4, ?5, ?6, ?7, ?8)",
                params![
                    record_id,
                    batch_id,
                    device_id,
                    record.seq,
                    record.source_name,
                    record.sample_rate_hz,
                    record.start_local_ns,
                    record_fingerprint
                ],
            )
            .map_err(|error| error.to_string())?;
            for (index, event) in record.events.iter().enumerate() {
                if event.event_type.trim().is_empty() || event.label.trim().is_empty() {
                    return Err("event type and label are required".to_string());
                }
                let event_id = stable_id(
                    "event",
                    &format!("{}|{}|{}", record_id, index, event.local_ns),
                );
                tx.execute(
                    "INSERT INTO events(id, record_id, device_id, record_import_seq, seq,
                                        event_type, label, local_ns, frequency_hz,
                                        phasor_magnitude, phasor_angle_deg)
                     VALUES (?1, ?2, ?3,
                        (SELECT import_seq FROM records WHERE id = ?2),
                        ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        event_id,
                        record_id,
                        device_id,
                        i64::try_from(index).unwrap(),
                        event.event_type,
                        event.label,
                        event.local_ns,
                        event.frequency_hz,
                        event.phasor_magnitude,
                        event.phasor_angle_deg
                    ],
                )
                .map_err(|error| error.to_string())?;
            }
            inserted += 1;
        }

        Ok(ImportSummary {
            batch_id,
            inserted_records: inserted,
            skipped_records: skipped,
            already_present: existing_batch.is_some(),
        })
    }

    pub fn create_disturbance(&self, input: DisturbanceInput) -> Result<Disturbance, String> {
        if input.code.trim().is_empty() || input.title.trim().is_empty() {
            return Err("disturbance code and title are required".to_string());
        }
        let id = stable_id("disturbance", &input.code);
        let connection = self.connection()?;
        connection
            .execute(
                "INSERT INTO disturbances(id, code, title, status)
                 VALUES (?1, ?2, ?3, 'open')
                 ON CONFLICT(id) DO NOTHING",
                params![id, input.code, input.title],
            )
            .map_err(|error| error.to_string())?;
        self.get_disturbance(&connection, &id)?
            .ok_or_else(|| "disturbance missing".to_string())
    }

    pub fn add_candidate(&self, input: CandidateInput) -> Result<Membership, String> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        self.require_disturbance(&tx, &input.disturbance_id)?;
        self.require_record(&tx, &input.record_id)?;
        tx.execute(
            "INSERT INTO memberships(disturbance_id, record_id, status)
             VALUES (?1, ?2, 'candidate')
             ON CONFLICT(disturbance_id, record_id) DO UPDATE
             SET status = CASE WHEN memberships.status = 'published' THEN 'published' ELSE 'candidate' END",
            params![input.disturbance_id, input.record_id],
        )
        .map_err(|error| error.to_string())?;
        let membership = self.get_membership(&tx, &input.disturbance_id, &input.record_id)?;
        tx.commit().map_err(|error| error.to_string())?;
        membership.ok_or_else(|| "candidate missing".to_string())
    }

    pub fn publish_alignment(
        &self,
        input: PublishAlignmentInput,
        fixed_now_ns: Option<i64>,
    ) -> Result<AlignmentVersion, String> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let result = self.publish_alignment_tx(&tx, input, fixed_now_ns);
        match result {
            Ok(version) => {
                tx.commit().map_err(|error| error.to_string())?;
                Ok(version)
            }
            Err(error) => Err(error),
        }
    }

    fn publish_alignment_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        input: PublishAlignmentInput,
        fixed_now_ns: Option<i64>,
    ) -> Result<AlignmentVersion, String> {
        self.require_disturbance(tx, &input.disturbance_id)?;
        if input.record_ids.is_empty() {
            return Err("published alignment must contain records".to_string());
        }
        let mut record_ids = input.record_ids.clone();
        record_ids.sort();
        record_ids.dedup();
        if record_ids.len() != input.record_ids.len() {
            return Err("record list contains duplicates".to_string());
        }
        for record_id in &record_ids {
            let owner: Option<String> = tx
                .query_row(
                    "SELECT disturbance_id FROM memberships
                     WHERE record_id = ?1 AND status = 'published'
                     AND disturbance_id != ?2",
                    params![record_id, input.disturbance_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if owner.is_some() {
                return Err(format!(
                    "record {} already belongs to another published disturbance",
                    record_id
                ));
            }
        }
        let records = self.records_by_ids(tx, &record_ids)?;
        if records.len() != record_ids.len() {
            return Err("one or more records do not exist".to_string());
        }
        let events = self.events_for_records(tx, &record_ids)?;
        let version_no = next_version_no(tx, &input.disturbance_id)?;
        let version_id = stable_id(
            "version",
            &format!("{}|{}", input.disturbance_id, version_no),
        );

        let mut persisted_segments = Vec::new();
        let mut clock_segments = Vec::new();
        let mut trusted_count = 0;
        for (ordinal, segment_input) in input.segments.iter().enumerate() {
            let ordinal = i64::try_from(ordinal).map_err(|error| error.to_string())?;
            let start = self.event_anchor(tx, &segment_input.start_event_id, &record_ids)?;
            let end = self.event_anchor(tx, &segment_input.end_event_id, &record_ids)?;
            if start.device_id != segment_input.device_id
                || end.device_id != segment_input.device_id
            {
                return Err("segment endpoints must belong to the declared device".to_string());
            }
            if event_capture(&start) > event_capture(&end) {
                return Err("segment capture range is reversed".to_string());
            }
            if segment_input.start_radius_ns < 0 || segment_input.end_radius_ns < 0 {
                return Err("uncertainty radius cannot be negative".to_string());
            }
            let segment_id = stable_id(
                "segment",
                &format!(
                    "{}|{}|{}|{}",
                    version_id, ordinal, segment_input.start_event_id, segment_input.end_event_id
                ),
            );
            let segment = Segment {
                id: segment_id.clone(),
                device_id: segment_input.device_id.clone(),
                ordinal,
                start_event_id: segment_input.start_event_id.clone(),
                end_event_id: segment_input.end_event_id.clone(),
                start_capture: event_capture(&start),
                end_capture: event_capture(&end),
                start_local_ns: start.local_ns,
                end_local_ns: end.local_ns,
                start_corrected_ns: segment_input.start_corrected_ns,
                end_corrected_ns: segment_input.end_corrected_ns,
                start_radius_ns: segment_input.start_radius_ns,
                end_radius_ns: segment_input.end_radius_ns,
                trusted: segment_input.trusted,
            };
            persisted_segments.push(segment);
            if segment_input.trusted {
                trusted_count += 1;
            }
            clock_segments.push((
                segment_id,
                ClockSegment {
                    device_id: segment_input.device_id.clone(),
                    ordinal,
                    start: event_capture(&start),
                    end: event_capture(&end),
                    start_local_ns: start.local_ns,
                    end_local_ns: end.local_ns,
                    start_corrected_ns: segment_input.start_corrected_ns,
                    end_corrected_ns: segment_input.end_corrected_ns,
                    start_radius_ns: segment_input.start_radius_ns,
                    end_radius_ns: segment_input.end_radius_ns,
                    trusted: segment_input.trusted,
                },
            ));
        }
        if trusted_count == 0 {
            return Err("alignment needs at least one trusted segment".to_string());
        }
        let clock_only: Vec<ClockSegment> = clock_segments
            .iter()
            .map(|(_, segment)| segment.clone())
            .collect();
        let mapped = map_events(&events, &clock_only)?;

        let now_ns = fixed_now_ns.unwrap_or_else(current_ns);
        tx.execute(
            "INSERT INTO alignment_versions(id, disturbance_id, version_no, note, published_ns)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                version_id,
                input.disturbance_id,
                version_no,
                input.note,
                now_ns
            ],
        )
        .map_err(|error| error.to_string())?;
        for segment in &persisted_segments {
            self.insert_segment(tx, &version_id, segment)?;
        }
        for (event, mapping) in events.iter().zip(mapped.iter()) {
            let point = CapturePoint {
                record_seq: event.record_seq,
                event_seq: event.seq,
            };
            let covering_segments: Vec<_> = clock_segments
                .iter()
                .filter(|(_, candidate)| {
                    candidate.device_id == mapping.device_id
                        && candidate.start <= point
                        && point <= candidate.end
                })
                .collect();
            let (segment_id, _) = covering_segments
                .iter()
                .find(|(_, candidate)| candidate.trusted && candidate.start == point)
                .or_else(|| {
                    covering_segments
                        .iter()
                        .find(|(_, candidate)| candidate.trusted)
                })
                .ok_or_else(|| "mapped segment missing".to_string())?;
            tx.execute(
                "INSERT INTO event_timings(event_id, version_id, segment_id,
                                           corrected_ns, min_ns, max_ns)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    mapping.event_id,
                    version_id,
                    segment_id,
                    mapping.corrected_ns,
                    mapping.min_ns,
                    mapping.max_ns
                ],
            )
            .map_err(|error| error.to_string())?;
        }
        for record_id in &record_ids {
            tx.execute(
                "INSERT INTO alignment_records(version_id, record_id) VALUES (?1, ?2)",
                params![version_id, record_id],
            )
            .map_err(|error| error.to_string())?;
            tx.execute(
                "INSERT INTO memberships(disturbance_id, record_id, status, published_version_id)
                 VALUES (?1, ?2, 'published', ?3)
                 ON CONFLICT(disturbance_id, record_id) DO UPDATE SET
                    status = 'published', published_version_id = excluded.published_version_id",
                params![input.disturbance_id, record_id, version_id],
            )
            .map_err(|error| error.to_string())?;
        }
        self.alignment_version(tx, &version_id)
    }

    pub fn state(&self) -> Result<ReviewState, String> {
        let connection = self.connection()?;
        let devices = self.devices(&connection)?;
        let records = self.records(&connection)?;
        let events = self.events(&connection)?;
        let disturbances = self.disturbances(&connection)?;
        let memberships = self.memberships(&connection)?;
        let versions = self.versions(&connection)?;
        Ok(ReviewState {
            devices,
            records,
            events,
            disturbances,
            memberships,
            versions,
            deterministic_order: Vec::new(),
        })
    }

    pub fn state_with_order(&self) -> Result<ReviewState, String> {
        let mut state = self.state()?;
        if let Some(latest) = state.versions.last() {
            state.deterministic_order = self.deterministic_order(latest);
        }
        Ok(state)
    }

    fn devices(&self, connection: &Connection) -> Result<Vec<Device>, String> {
        let mut statement = connection
            .prepare("SELECT id, code, name, sample_rate_hz FROM devices ORDER BY code")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(Device {
                    id: row.get(0)?,
                    code: row.get(1)?,
                    name: row.get(2)?,
                    sample_rate_hz: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }

    fn records(&self, connection: &Connection) -> Result<Vec<EventRecord>, String> {
        let mut statement = connection
            .prepare(
                "SELECT id, batch_id, device_id, source_seq, source_name, sample_rate_hz,
                        start_local_ns, fingerprint
                 FROM records ORDER BY device_id, import_seq, source_seq",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(EventRecord {
                    id: row.get(0)?,
                    batch_id: row.get(1)?,
                    device_id: row.get(2)?,
                    seq: row.get(3)?,
                    source_name: row.get(4)?,
                    sample_rate_hz: row.get(5)?,
                    start_local_ns: row.get(6)?,
                    fingerprint: row.get(7)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }

    fn records_by_ids(
        &self,
        connection: &Connection,
        ids: &[String],
    ) -> Result<Vec<EventRecord>, String> {
        let mut result = Vec::new();
        for id in ids {
            connection
                .query_row(
                    "SELECT id, batch_id, device_id, source_seq, source_name, sample_rate_hz,
                            start_local_ns, fingerprint
                     FROM records WHERE id = ?1",
                    params![id],
                    |row| {
                        Ok(EventRecord {
                            id: row.get(0)?,
                            batch_id: row.get(1)?,
                            device_id: row.get(2)?,
                            seq: row.get(3)?,
                            source_name: row.get(4)?,
                            sample_rate_hz: row.get(5)?,
                            start_local_ns: row.get(6)?,
                            fingerprint: row.get(7)?,
                        })
                    },
                )
                .optional()
                .map_err(|error| error.to_string())?
                .map(|record| result.push(record));
        }
        Ok(result)
    }

    fn events(&self, connection: &Connection) -> Result<Vec<Event>, String> {
        let mut statement = connection
            .prepare(
                "SELECT id, record_id, device_id, record_import_seq, seq, event_type, label,
                        local_ns, frequency_hz, phasor_magnitude, phasor_angle_deg
                 FROM events ORDER BY device_id, record_import_seq, seq",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], event_from_row)
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }

    fn events_for_records(
        &self,
        connection: &Connection,
        record_ids: &[String],
    ) -> Result<Vec<Event>, String> {
        let mut events = Vec::new();
        for record_id in record_ids {
            let mut statement = connection
                .prepare(
                    "SELECT id, record_id, device_id, record_import_seq, seq, event_type, label,
                            local_ns, frequency_hz, phasor_magnitude, phasor_angle_deg
                     FROM events WHERE record_id = ?1 ORDER BY seq",
                )
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map(params![record_id], event_from_row)
                .map_err(|error| error.to_string())?;
            for event in rows {
                events.push(event.map_err(|error| error.to_string())?);
            }
        }
        events.sort_by_key(|event| (event.device_id.clone(), event.record_seq, event.seq));
        Ok(events)
    }

    fn disturbances(&self, connection: &Connection) -> Result<Vec<Disturbance>, String> {
        let mut statement = connection
            .prepare("SELECT id, code, title, status FROM disturbances ORDER BY code")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(Disturbance {
                    id: row.get(0)?,
                    code: row.get(1)?,
                    title: row.get(2)?,
                    status: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }

    fn memberships(&self, connection: &Connection) -> Result<Vec<Membership>, String> {
        let mut statement = connection
            .prepare(
                "SELECT disturbance_id, record_id, status, published_version_id
                 FROM memberships ORDER BY disturbance_id, record_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(Membership {
                    disturbance_id: row.get(0)?,
                    record_id: row.get(1)?,
                    status: row.get(2)?,
                    published_version_id: row.get(3)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }

    fn versions(&self, connection: &Connection) -> Result<Vec<AlignmentVersion>, String> {
        let mut ids: Vec<String> = Vec::new();
        let mut statement = connection
            .prepare(
                "SELECT id FROM alignment_versions
                 ORDER BY disturbance_id, version_no",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        for id in rows {
            ids.push(id.map_err(|error| error.to_string())?);
        }
        ids.iter()
            .map(|id| self.alignment_version(connection, id))
            .collect()
    }

    fn alignment_version(
        &self,
        connection: &Connection,
        id: &str,
    ) -> Result<AlignmentVersion, String> {
        let (disturbance_id, version_no, note, published_ns) = connection
            .query_row(
                "SELECT disturbance_id, version_no, note, published_ns
                 FROM alignment_versions WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|error| error.to_string())?;
        let segments = self.segments(connection, id)?;
        let timings = self.timings(connection, id)?;
        let record_ids = self.version_records(connection, id)?;
        Ok(AlignmentVersion {
            id: id.to_string(),
            disturbance_id,
            version_no,
            note,
            published_ns,
            record_ids,
            segments,
            timings,
        })
    }

    fn segments(&self, connection: &Connection, version_id: &str) -> Result<Vec<Segment>, String> {
        let mut statement = connection
            .prepare(
                "SELECT id, device_id, ordinal, start_event_id, end_event_id,
                        start_record_seq, start_event_seq, end_record_seq, end_event_seq,
                        start_local_ns, end_local_ns, start_corrected_ns, end_corrected_ns,
                        start_radius_ns, end_radius_ns, trusted
                 FROM alignment_segments WHERE version_id = ?1 ORDER BY ordinal",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![version_id], |row| {
                Ok(Segment {
                    id: row.get(0)?,
                    device_id: row.get(1)?,
                    ordinal: row.get(2)?,
                    start_event_id: row.get(3)?,
                    end_event_id: row.get(4)?,
                    start_capture: CapturePoint {
                        record_seq: row.get(5)?,
                        event_seq: row.get(6)?,
                    },
                    end_capture: CapturePoint {
                        record_seq: row.get(7)?,
                        event_seq: row.get(8)?,
                    },
                    start_local_ns: row.get(9)?,
                    end_local_ns: row.get(10)?,
                    start_corrected_ns: row.get(11)?,
                    end_corrected_ns: row.get(12)?,
                    start_radius_ns: row.get(13)?,
                    end_radius_ns: row.get(14)?,
                    trusted: row.get(15)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }

    fn timings(
        &self,
        connection: &Connection,
        version_id: &str,
    ) -> Result<Vec<EventTiming>, String> {
        let mut statement = connection
            .prepare(
                "SELECT event_id,
                        (SELECT device_id FROM events WHERE events.id = event_timings.event_id),
                        segment_id, corrected_ns, min_ns, max_ns
                 FROM event_timings WHERE version_id = ?1
                 ORDER BY corrected_ns, min_ns, max_ns, event_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![version_id], |row| {
                Ok(EventTiming {
                    event_id: row.get(0)?,
                    device_id: row.get(1)?,
                    segment_id: row.get(2)?,
                    corrected_ns: row.get(3)?,
                    min_ns: row.get(4)?,
                    max_ns: row.get(5)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }

    fn deterministic_order(&self, version: &AlignmentVersion) -> Vec<OrderRelation> {
        let timings = &version.timings;
        let mut relations = Vec::new();
        for (index, earlier) in timings.iter().enumerate() {
            for later in &timings[index + 1..] {
                if earlier.max_ns < later.min_ns {
                    relations.push(OrderRelation {
                        earlier_event_id: earlier.event_id.clone(),
                        later_event_id: later.event_id.clone(),
                    });
                }
            }
        }
        relations
    }

    fn version_records(
        &self,
        connection: &Connection,
        version_id: &str,
    ) -> Result<Vec<String>, String> {
        let mut statement = connection
            .prepare(
                "SELECT record_id FROM alignment_records WHERE version_id = ?1 ORDER BY record_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![version_id], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<_, _>>()
            .map_err(|error| error.to_string())
    }

    fn get_disturbance(
        &self,
        connection: &Connection,
        id: &str,
    ) -> Result<Option<Disturbance>, String> {
        connection
            .query_row(
                "SELECT id, code, title, status FROM disturbances WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Disturbance {
                        id: row.get(0)?,
                        code: row.get(1)?,
                        title: row.get(2)?,
                        status: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn require_disturbance(&self, connection: &Connection, id: &str) -> Result<(), String> {
        if self.get_disturbance(connection, id)?.is_some() {
            Ok(())
        } else {
            Err(format!("disturbance {} does not exist", id))
        }
    }

    fn require_record(&self, connection: &Connection, id: &str) -> Result<(), String> {
        let exists: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM records WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if exists == 1 {
            Ok(())
        } else {
            Err(format!("record {} does not exist", id))
        }
    }

    fn get_membership(
        &self,
        connection: &Connection,
        disturbance_id: &str,
        record_id: &str,
    ) -> Result<Option<Membership>, String> {
        connection
            .query_row(
                "SELECT disturbance_id, record_id, status, published_version_id
                 FROM memberships WHERE disturbance_id = ?1 AND record_id = ?2",
                params![disturbance_id, record_id],
                |row| {
                    Ok(Membership {
                        disturbance_id: row.get(0)?,
                        record_id: row.get(1)?,
                        status: row.get(2)?,
                        published_version_id: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn event_anchor(
        &self,
        connection: &Connection,
        event_id: &str,
        record_ids: &[String],
    ) -> Result<Event, String> {
        connection
            .query_row(
                "SELECT id, record_id, device_id, record_import_seq, seq, event_type, label,
                        local_ns, frequency_hz, phasor_magnitude, phasor_angle_deg
                 FROM events WHERE id = ?1",
                params![event_id],
                event_from_row,
            )
            .optional()
            .map_err(|error| error.to_string())?
            .filter(|event| record_ids.contains(&event.record_id))
            .ok_or_else(|| format!("anchor event {} is not in the selected records", event_id))
    }

    fn insert_segment(
        &self,
        tx: &rusqlite::Transaction<'_>,
        version_id: &str,
        segment: &Segment,
    ) -> Result<(), String> {
        tx.execute(
            "INSERT INTO alignment_segments(
                id, version_id, device_id, ordinal, start_event_id, end_event_id,
                start_record_seq, start_event_seq, end_record_seq, end_event_seq,
                start_local_ns, end_local_ns, start_corrected_ns, end_corrected_ns,
                start_radius_ns, end_radius_ns, trusted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                segment.id,
                version_id,
                segment.device_id,
                segment.ordinal,
                segment.start_event_id,
                segment.end_event_id,
                segment.start_capture.record_seq,
                segment.start_capture.event_seq,
                segment.end_capture.record_seq,
                segment.end_capture.event_seq,
                segment.start_local_ns,
                segment.end_local_ns,
                segment.start_corrected_ns,
                segment.end_corrected_ns,
                segment.start_radius_ns,
                segment.end_radius_ns,
                segment.trusted
            ],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }
}

#[derive(Serialize)]
struct RecordFingerprint<'a> {
    source_name: &'a str,
    sample_rate_hz: i64,
    start_local_ns: Option<i64>,
    events: &'a [EventInput],
}

fn event_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Event> {
    Ok(Event {
        id: row.get(0)?,
        record_id: row.get(1)?,
        device_id: row.get(2)?,
        record_seq: row.get(3)?,
        seq: row.get(4)?,
        event_type: row.get(5)?,
        label: row.get(6)?,
        local_ns: row.get(7)?,
        frequency_hz: row.get(8)?,
        phasor_magnitude: row.get(9)?,
        phasor_angle_deg: row.get(10)?,
    })
}

fn event_capture(event: &Event) -> CapturePoint {
    CapturePoint {
        record_seq: event.record_seq,
        event_seq: event.seq,
    }
}

fn next_version_no(connection: &Connection, disturbance_id: &str) -> Result<i64, String> {
    connection
        .query_row(
            "SELECT COALESCE(MAX(version_no), 0) + 1 FROM alignment_versions
             WHERE disturbance_id = ?1",
            params![disturbance_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn current_ns() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

pub fn stable_id(prefix: &str, material: &str) -> String {
    format!(
        "{}_{}",
        prefix,
        hash_text(material).chars().take(24).collect::<String>()
    )
}

pub fn hash_text(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect()
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}
