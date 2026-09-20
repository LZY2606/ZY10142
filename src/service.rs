use crate::db::now_ns;
use crate::time::{
    build_clock_model, compare_intervals, AnchorObservation, ClockModel, ClockSegment,
    IntervalOrder, Rational, TimeInterval,
};
use crate::types::*;
use crate::util::{canonical_json, fingerprint128, parse_time_ns};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value;

pub struct App {
    connection: Connection,
}

impl App {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }

    pub fn into_connection(self) -> Connection {
        self.connection
    }

    pub fn record_count(&mut self) -> Result<i64, String> {
        self.connection
            .query_row("SELECT COUNT(*) FROM records", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(sql_error)
    }

    pub fn import_records(&mut self, request: ImportRequest) -> Result<ImportResponse, String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let now = now_ns();
        transaction
            .execute(
                "INSERT INTO import_batches(imported_at_ns) VALUES (?1)",
                [now],
            )
            .map_err(sql_error)?;
        let batch_id = transaction.last_insert_rowid();
        let mut imported = Vec::new();
        let mut skipped = Vec::new();

        for input in request.records {
            validate_record_input(&input)?;
            let local_time_ns = parse_time_ns(&input.local_time)?;
            let digital = input.digital_changes.clone();
            let phasor = input.phasor_features.clone();
            let mut normalized_raw = serde_json::Map::new();
            normalized_raw.insert("deviceId".into(), Value::String(input.device_id.clone()));
            normalized_raw.insert("localTimeNs".into(), Value::from(local_time_ns));
            if let Some(sample_rate) = input.sample_rate_hz {
                normalized_raw.insert("sampleRateHz".into(), Value::from(sample_rate));
            }
            normalized_raw.insert(
                "digitalChanges".into(),
                Value::Array(input.digital_changes.clone()),
            );
            normalized_raw.insert(
                "phasorFeatures".into(),
                Value::Array(input.phasor_features.clone()),
            );
            normalized_raw.insert(
                "events".into(),
                Value::Array(normalized_events_for_fingerprint(&input.events)?),
            );
            let raw = input
                .raw
                .clone()
                .unwrap_or_else(|| Value::Object(normalized_raw.clone()));
            let mut fingerprint_map = serde_json::Map::new();
            fingerprint_map.insert("deviceId".into(), Value::String(input.device_id.clone()));
            fingerprint_map.insert("localTimeNs".into(), Value::from(local_time_ns));
            if let Some(sample_rate) = input.sample_rate_hz {
                fingerprint_map.insert("sampleRateHz".into(), Value::from(sample_rate));
            }
            fingerprint_map.insert("digitalChanges".into(), Value::Array(digital.clone()));
            fingerprint_map.insert("phasorFeatures".into(), Value::Array(phasor.clone()));
            fingerprint_map.insert(
                "events".into(),
                Value::Array(normalized_events_for_fingerprint(&input.events)?),
            );
            if input.raw.is_some() {
                fingerprint_map.insert("raw".into(), input.raw.clone().unwrap());
            }
            let fingerprint_source = Value::Object(fingerprint_map);
            let fingerprint = fingerprint128(&canonical_json(&fingerprint_source));
            if let Some(record_id) = transaction
                .query_row(
                    "SELECT record_id FROM records WHERE device_id = ?1 AND fingerprint = ?2",
                    [&input.device_id, &fingerprint],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(sql_error)?
            {
                skipped.push(ImportedRecord {
                    record_id,
                    device_id: input.device_id,
                    fingerprint,
                });
                continue;
            }

            let record_id = format!(
                "rec_{}",
                fingerprint128(format!("{}:{}", input.device_id, fingerprint).as_bytes())
            );
            transaction
                .execute(
                    "INSERT INTO records(record_id,batch_id,device_id,fingerprint,local_time_ns,sample_rate_hz,digital_changes_json,phasor_features_json,raw_json)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    rusqlite::params![
                        record_id,
                        batch_id,
                        input.device_id,
                        fingerprint,
                        local_time_ns,
                        input.sample_rate_hz,
                        canonical_json(&serde_json::to_value(&input.digital_changes).map_err(|error| error.to_string())?),
                        canonical_json(&serde_json::to_value(&input.phasor_features).map_err(|error| error.to_string())?),
                        canonical_json(&raw),
                    ],
                )
                .map_err(sql_error)?;
            for event in &input.events {
                let event_local = parse_time_ns(&event.local_time)?;
                let event_id = format!(
                    "evt_{}",
                    fingerprint128(format!("{record_id}:{}", event.key).as_bytes())
                );
                transaction
                    .execute(
                        "INSERT INTO record_events(event_id,record_id,key,kind,local_time_ns,uncertainty_ns,channel,label)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                        rusqlite::params![
                            event_id,
                            record_id,
                            event.key,
                            event.kind,
                            event_local,
                            event.uncertainty_ns.max(0),
                            event.channel,
                            event.label,
                        ],
                    )
                    .map_err(sql_error)?;
            }
            imported.push(ImportedRecord {
                record_id,
                device_id: input.device_id,
                fingerprint,
            });
        }
        transaction.commit().map_err(sql_error)?;
        Ok(ImportResponse {
            imported,
            skipped_duplicates: skipped,
        })
    }

    pub fn create_case(&mut self, request: CreateCaseRequest) -> Result<i64, String> {
        if request.name.trim().is_empty() {
            return Err("case name is required".into());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        transaction
            .execute(
                "INSERT INTO cases(name,description,created_at_ns) VALUES (?1,?2,?3)",
                rusqlite::params![request.name, request.description, now_ns()],
            )
            .map_err(sql_error)?;
        let case_id = transaction.last_insert_rowid();
        for record_id in unique_sorted(request.record_ids) {
            require_record_exists(&transaction, &record_id)?;
            transaction
                .execute(
                    "INSERT OR IGNORE INTO case_candidates(case_id,record_id,added_at_ns) VALUES (?1,?2,?3)",
                    rusqlite::params![case_id, record_id, now_ns()],
                )
                .map_err(sql_error)?;
        }
        transaction.commit().map_err(sql_error)?;
        Ok(case_id)
    }

    pub fn assign_candidates(
        &mut self,
        case_id: i64,
        request: AssignRequest,
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        require_case_exists(&transaction, case_id)?;
        for record_id in unique_sorted(request.record_ids) {
            require_record_exists(&transaction, &record_id)?;
            transaction
                .execute(
                    "INSERT INTO case_candidates(case_id,record_id,added_at_ns) VALUES (?1,?2,?3)",
                    rusqlite::params![case_id, record_id, now_ns()],
                )
                .map_err(sql_error)?;
        }
        transaction.commit().map_err(sql_error)?;
        Ok(())
    }

    pub fn save_anchor(&mut self, request: SaveAnchorRequest) -> Result<(), String> {
        if request.anchor_key.trim().is_empty() || request.label.trim().is_empty() {
            return Err("anchor key and label are required".into());
        }
        if request.uncertainty_ns < 0 {
            return Err("anchor uncertainty must be non-negative".into());
        }
        if request.trusted && request.corrected_time_ns.is_none() {
            return Err("a trusted anchor requires correctedTimeNs".into());
        }
        if request.event_refs.is_empty() {
            return Err("an anchor must reference at least one event".into());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        transaction
            .execute(
                "INSERT INTO anchors(anchor_key,label,corrected_time_ns,uncertainty_ns,trusted)
                 VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(anchor_key) DO UPDATE SET
                    label=excluded.label,
                    corrected_time_ns=excluded.corrected_time_ns,
                    uncertainty_ns=excluded.uncertainty_ns,
                    trusted=excluded.trusted",
                rusqlite::params![
                    request.anchor_key,
                    request.label,
                    request.corrected_time_ns,
                    request.uncertainty_ns,
                    request.trusted as i64
                ],
            )
            .map_err(sql_error)?;

        let mut event_ids = Vec::new();
        for reference in &request.event_refs {
            let event_id = event_id(&reference.record_id, &reference.event_key);
            require_event_exists(&transaction, &event_id)?;
            event_ids.push(event_id);
        }
        for event_id in &event_ids {
            transaction
                .execute(
                    "DELETE FROM anchor_events WHERE anchor_key = ?1 OR event_id = ?2",
                    rusqlite::params![request.anchor_key, event_id],
                )
                .map_err(sql_error)?;
        }
        for event_id in &event_ids {
            transaction
                .execute(
                    "INSERT INTO anchor_events(anchor_key,event_id) VALUES (?1,?2)",
                    rusqlite::params![request.anchor_key, event_id],
                )
                .map_err(sql_error)?;
        }
        transaction.commit().map_err(sql_error)?;
        Ok(())
    }

    pub fn create_version(
        &mut self,
        case_id: i64,
        request: CreateVersionRequest,
    ) -> Result<i64, String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        require_case_exists(&transaction, case_id)?;
        let record_ids = candidate_record_ids(&transaction, case_id)?;
        if record_ids.is_empty() {
            return Err("case has no candidate records".into());
        }

        #[derive(Clone)]
        struct SelectedEvent {
            event_id: String,
            device_id: String,
            local_time_ns: i64,
            event_uncertainty_ns: i64,
            anchor_key: String,
            corrected_time_ns: i64,
            anchor_uncertainty_ns: i64,
        }

        let mut selected = Vec::new();
        {
            let placeholders = record_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "SELECT re.event_id,re.record_id,r.device_id,re.local_time_ns,re.uncertainty_ns,
                        a.anchor_key,a.corrected_time_ns,a.uncertainty_ns
                 FROM anchor_events ae
                 JOIN anchors a ON a.anchor_key = ae.anchor_key
                 JOIN record_events re ON re.event_id = ae.event_id
                 JOIN records r ON r.record_id = re.record_id
                 WHERE a.trusted = 1 AND re.record_id IN ({placeholders})"
            );
            let mut params: Vec<&dyn rusqlite::ToSql> = Vec::new();
            for record_id in &record_ids {
                params.push(record_id);
            }
            let mut statement = transaction.prepare(&sql).map_err(sql_error)?;
            let rows = statement
                .query_map(params.as_slice(), |row| {
                    Ok(SelectedEvent {
                        event_id: row.get(0)?,
                        device_id: row.get(2)?,
                        local_time_ns: row.get(3)?,
                        event_uncertainty_ns: row.get(4)?,
                        anchor_key: row.get(5)?,
                        corrected_time_ns: row.get::<_, Option<i64>>(6)?.unwrap_or_default(),
                        anchor_uncertainty_ns: row.get(7)?,
                    })
                })
                .map_err(sql_error)?;
            for row in rows {
                selected.push(row.map_err(sql_error)?);
            }
        }

        let devices = record_devices(&transaction, &record_ids)?;
        let mut models: Vec<ClockModel> = Vec::new();
        for device_id in &devices {
            let observations = selected
                .iter()
                .filter(|event| &event.device_id == device_id)
                .map(|event| AnchorObservation {
                    local_ns: event.local_time_ns,
                    corrected_ns: event.corrected_time_ns,
                    uncertainty_ns: event.anchor_uncertainty_ns + event.event_uncertainty_ns,
                })
                .collect::<Vec<_>>();
            if observations.is_empty() {
                return Err(format!(
                    "device {device_id} has no trusted anchor for this case"
                ));
            }
            let jumps = request
                .jumps_by_device
                .get(device_id)
                .cloned()
                .unwrap_or_default();
            let mut model = build_clock_model(device_id.clone(), &observations, &jumps)?;
            include_device_event_uncertainty(&transaction, &record_ids, &mut model)?;
            models.push(model);
        }

        let version_no = transaction
            .query_row(
                "SELECT COALESCE(MAX(version_no),0)+1 FROM versions WHERE case_id = ?1",
                [case_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(sql_error)?;
        let created_at = now_ns();
        transaction
            .execute(
                "INSERT INTO versions(version_id,case_id,version_no,note,created_at_ns)
                 VALUES (NULL,?1,?2,?3,?4)",
                rusqlite::params![case_id, version_no, request.note, created_at],
            )
            .map_err(sql_error)?;
        let version_id = transaction.last_insert_rowid();
        for record_id in &record_ids {
            transaction
                .execute(
                    "INSERT INTO version_records(version_id,record_id) VALUES (?1,?2)",
                    rusqlite::params![version_id, record_id],
                )
                .map_err(sql_error)?;
        }
        for event in &selected {
            transaction
                .execute(
                    "INSERT INTO version_anchors(version_id,anchor_key,event_id,device_id,local_time_ns,corrected_time_ns,uncertainty_ns)
                     VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    rusqlite::params![
                        version_id,
                        event.anchor_key,
                        event.event_id,
                        event.device_id,
                        event.local_time_ns,
                        event.corrected_time_ns,
                        event.anchor_uncertainty_ns + event.event_uncertainty_ns
                    ],
                )
                .map_err(sql_error)?;
        }
        for model in &models {
            for (index, segment) in model.segments.iter().enumerate() {
                transaction
                    .execute(
                        "INSERT INTO clock_segments(version_id,device_id,segment_index,start_ns,end_ns,
                                              intercept_num,intercept_den,slope_num,slope_den,uncertainty_ns)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                        rusqlite::params![
                            version_id,
                            model.device_id,
                            index as i64,
                            segment.start_ns,
                            segment.end_ns,
                            segment.intercept.numer.to_string(),
                            segment.intercept.denom.to_string(),
                            segment.slope.numer.to_string(),
                            segment.slope.denom.to_string(),
                            segment.uncertainty_ns
                        ],
                    )
                    .map_err(sql_error)?;
            }
        }
        transaction.commit().map_err(sql_error)?;
        Ok(version_id)
    }

    pub fn publish_version(&mut self, version_id: i64) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let case_id = transaction
            .query_row(
                "SELECT case_id FROM versions WHERE version_id = ?1",
                [version_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(sql_error)?
            .ok_or_else(|| format!("version {version_id} does not exist"))?;
        let already_published = transaction
            .query_row(
                "SELECT published_at_ns FROM versions WHERE version_id = ?1",
                [version_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(sql_error)?
            .is_some();
        if already_published {
            return Ok(());
        }
        let record_ids = version_record_ids(&transaction, version_id)?;
        let placeholders = record_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("SELECT record_id, case_id FROM published_membership WHERE record_id IN ({placeholders}) AND case_id <> ?");
        let mut params: Vec<&dyn rusqlite::ToSql> = record_ids
            .iter()
            .map(|id| id as &dyn rusqlite::ToSql)
            .collect();
        params.push(&case_id);
        let mut conflict_text = Vec::new();
        {
            let mut statement = transaction.prepare(&sql).map_err(sql_error)?;
            let conflicts = statement
                .query_map(params.as_slice(), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(sql_error)?;
            for conflict in conflicts {
                let (record_id, other_case_id) = conflict.map_err(sql_error)?;
                conflict_text.push(format!("{record_id}->case {other_case_id}"));
            }
        }
        if !conflict_text.is_empty() {
            return Err(format!(
                "records already belong to another published disturbance: {}",
                conflict_text.join(", ")
            ));
        }
        let published_at = now_ns();
        for record_id in &record_ids {
            transaction
                .execute(
                    "INSERT INTO published_membership(record_id,case_id,version_id,published_at_ns)
                     VALUES (?1,?2,?3,?4)
                     ON CONFLICT(record_id) DO UPDATE SET
                        case_id=excluded.case_id,
                        version_id=excluded.version_id,
                        published_at_ns=excluded.published_at_ns",
                    rusqlite::params![record_id, case_id, version_id, published_at],
                )
                .map_err(sql_error)?;
        }
        transaction
            .execute(
                "UPDATE versions SET published_at_ns = ?1 WHERE version_id = ?2",
                rusqlite::params![published_at, version_id],
            )
            .map_err(sql_error)?;
        transaction.commit().map_err(sql_error)?;
        Ok(())
    }

    pub fn add_review(&mut self, version_id: i64, request: ReviewRequest) -> Result<i64, String> {
        if request.conclusion.trim().is_empty() {
            return Err("review conclusion is required".into());
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let exists = transaction
            .query_row(
                "SELECT 1 FROM versions WHERE version_id = ?1",
                [version_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(sql_error)?;
        if exists.is_none() {
            return Err(format!("version {version_id} does not exist"));
        }
        transaction
            .execute(
                "INSERT INTO reviews(version_id,conclusion,created_at_ns) VALUES (?1,?2,?3)",
                rusqlite::params![version_id, request.conclusion, now_ns()],
            )
            .map_err(sql_error)?;
        let review_id = transaction.last_insert_rowid();
        transaction.commit().map_err(sql_error)?;
        Ok(review_id)
    }

    pub fn state(&mut self) -> Result<StateResponse, String> {
        let records = self.load_records()?;
        let events = self.load_events()?;
        let cases = self.load_cases(&events)?;
        Ok(StateResponse {
            records,
            events,
            cases,
        })
    }

    fn load_records(&self) -> Result<Vec<RecordDto>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT record_id,device_id,local_time_ns,sample_rate_hz,fingerprint
                 FROM records ORDER BY record_id",
            )
            .map_err(sql_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok(RecordDto {
                    record_id: row.get(0)?,
                    device_id: row.get(1)?,
                    local_time_ns: row.get(2)?,
                    sample_rate_hz: row.get(3)?,
                    fingerprint: row.get(4)?,
                })
            })
            .map_err(sql_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sql_error)
    }

    fn load_events(&self) -> Result<Vec<RecordEventDto>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT re.event_id,re.record_id,r.device_id,re.key,re.kind,re.local_time_ns,
                        re.uncertainty_ns,re.channel,re.label
                 FROM record_events re JOIN records r ON r.record_id = re.record_id
                 ORDER BY re.record_id,re.local_time_ns,re.key",
            )
            .map_err(sql_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok(RecordEventDto {
                    event_id: row.get(0)?,
                    record_id: row.get(1)?,
                    device_id: row.get(2)?,
                    key: row.get(3)?,
                    kind: row.get(4)?,
                    local_time_ns: row.get(5)?,
                    uncertainty_ns: row.get(6)?,
                    channel: row.get(7)?,
                    label: row.get(8)?,
                })
            })
            .map_err(sql_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(sql_error)
    }

    fn load_cases(&self, all_events: &[RecordEventDto]) -> Result<Vec<CaseDto>, String> {
        let mut cases = Vec::new();
        let mut case_statement = self
            .connection
            .prepare("SELECT case_id,name,description,created_at_ns FROM cases ORDER BY case_id")
            .map_err(sql_error)?;
        let case_rows = case_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)?;

        for (case_id, name, description, created_at_ns) in case_rows {
            let candidate_record_ids = candidate_record_ids(&self.connection, case_id)?;
            let published_record_ids = published_record_ids(&self.connection, case_id)?;
            let anchors = load_anchors(&self.connection)?;
            let versions = load_versions(&self.connection, case_id, all_events)?;
            cases.push(CaseDto {
                case_id,
                name,
                description,
                created_at_ns,
                candidate_record_ids,
                published_record_ids,
                versions,
                anchors,
            });
        }
        Ok(cases)
    }
}

fn validate_record_input(input: &RecordInput) -> Result<(), String> {
    if input.device_id.trim().is_empty() {
        return Err("deviceId is required".into());
    }
    if input.events.is_empty() {
        return Err("at least one event is required".into());
    }
    let mut keys = std::collections::BTreeSet::new();
    for event in &input.events {
        if event.key.trim().is_empty() || event.kind.trim().is_empty() {
            return Err("event key and kind are required".into());
        }
        if event.uncertainty_ns < 0 || !keys.insert(event.key.clone()) {
            return Err("event keys must be unique with non-negative uncertainty".into());
        }
        parse_time_ns(&event.local_time)?;
    }
    Ok(())
}

fn event_id(record_id: &str, event_key: &str) -> String {
    format!(
        "evt_{}",
        fingerprint128(format!("{record_id}:{event_key}").as_bytes())
    )
}

fn require_event_exists(connection: &Connection, event_id: &str) -> Result<(), String> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM record_events WHERE event_id = ?1",
            [event_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(sql_error)?;
    if exists.is_none() {
        return Err(format!("event {event_id} does not exist"));
    }
    Ok(())
}

fn candidate_record_ids(connection: &Connection, case_id: i64) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare("SELECT record_id FROM case_candidates WHERE case_id = ?1 ORDER BY record_id")
        .map_err(sql_error)?;
    let rows = statement
        .query_map([case_id], |row| row.get::<_, String>(0))
        .map_err(sql_error)?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row.map_err(sql_error)?);
    }
    Ok(records)
}

fn version_record_ids(connection: &Connection, version_id: i64) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare("SELECT record_id FROM version_records WHERE version_id = ?1 ORDER BY record_id")
        .map_err(sql_error)?;
    let rows = statement
        .query_map([version_id], |row| row.get::<_, String>(0))
        .map_err(sql_error)?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row.map_err(sql_error)?);
    }
    Ok(records)
}

fn record_devices(connection: &Connection, record_ids: &[String]) -> Result<Vec<String>, String> {
    let placeholders = record_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT DISTINCT device_id FROM records WHERE record_id IN ({placeholders}) ORDER BY device_id"
    );
    let mut statement = connection.prepare(&sql).map_err(sql_error)?;
    let params: Vec<&dyn rusqlite::ToSql> = record_ids
        .iter()
        .map(|id| id as &dyn rusqlite::ToSql)
        .collect();
    let rows = statement
        .query_map(params.as_slice(), |row| row.get::<_, String>(0))
        .map_err(sql_error)?;
    let mut devices = Vec::new();
    for row in rows {
        devices.push(row.map_err(sql_error)?);
    }
    Ok(devices)
}

fn include_device_event_uncertainty(
    connection: &Connection,
    record_ids: &[String],
    model: &mut ClockModel,
) -> Result<(), String> {
    let placeholders = record_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT MAX(re.uncertainty_ns) FROM record_events re
         JOIN records r ON r.record_id = re.record_id
         WHERE r.device_id = ? AND re.record_id IN ({placeholders})"
    );
    let mut params: Vec<&dyn rusqlite::ToSql> = vec![&model.device_id];
    for record_id in record_ids {
        params.push(record_id);
    }
    let device_event_uncertainty = connection
        .query_row(&sql, params.as_slice(), |row| row.get::<_, Option<i64>>(0))
        .map_err(sql_error)?
        .unwrap_or(0);
    for segment in &mut model.segments {
        segment.uncertainty_ns = segment.uncertainty_ns.max(device_event_uncertainty);
    }
    model.validate()
}

fn published_record_ids(connection: &Connection, case_id: i64) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare("SELECT record_id FROM published_membership WHERE case_id = ?1 ORDER BY record_id")
        .map_err(sql_error)?;
    let rows = statement
        .query_map([case_id], |row| row.get::<_, String>(0))
        .map_err(sql_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sql_error)
}

fn load_anchors(connection: &Connection) -> Result<Vec<AnchorDto>, String> {
    let mut statement = connection
        .prepare(
            "SELECT anchor_key,label,corrected_time_ns,uncertainty_ns,trusted
             FROM anchors ORDER BY anchor_key",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    let mut anchors = Vec::new();
    for (anchor_key, label, corrected_time_ns, uncertainty_ns, trusted) in rows {
        let mut reference_statement = connection
            .prepare(
                "SELECT re.record_id,re.key FROM anchor_events ae
                 JOIN record_events re ON re.event_id = ae.event_id
                 WHERE ae.anchor_key = ?1 ORDER BY re.record_id,re.key",
            )
            .map_err(sql_error)?;
        let reference_rows = reference_statement
            .query_map([&anchor_key], |row| {
                Ok(AnchorEventRef {
                    record_id: row.get(0)?,
                    event_key: row.get(1)?,
                })
            })
            .map_err(sql_error)?;
        let event_refs = reference_rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)?;
        anchors.push(AnchorDto {
            anchor_key,
            label,
            corrected_time_ns,
            uncertainty_ns,
            trusted: trusted != 0,
            event_refs,
        });
    }
    Ok(anchors)
}

fn load_versions(
    connection: &Connection,
    case_id: i64,
    all_events: &[RecordEventDto],
) -> Result<Vec<VersionDto>, String> {
    let mut versions = Vec::new();
    let mut statement = connection
        .prepare(
            "SELECT version_id,version_no,note,created_at_ns,published_at_ns
             FROM versions WHERE case_id = ?1 ORDER BY version_no",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map([case_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)?;
    for (version_id, version_no, note, created_at_ns, published_at_ns) in rows {
        let record_ids = version_record_ids(connection, version_id)?;
        let segments = load_segments(connection, version_id)?;
        let models = build_models_from_segments(&segments)?;
        let timeline = build_timeline(all_events, &record_ids, &models)?;
        let pair_relations = build_pair_relations(&timeline);
        let reviews = load_reviews(connection, version_id)?;
        versions.push(VersionDto {
            version_id,
            case_id,
            version_no,
            note,
            published: published_at_ns.is_some(),
            created_at_ns,
            published_at_ns,
            segments,
            timeline,
            pair_relations,
            reviews,
        });
    }
    Ok(versions)
}

fn load_segments(connection: &Connection, version_id: i64) -> Result<Vec<SegmentDto>, String> {
    let mut statement = connection
        .prepare(
            "SELECT device_id,start_ns,end_ns,intercept_num,intercept_den,slope_num,slope_den,uncertainty_ns
             FROM clock_segments WHERE version_id = ?1 ORDER BY device_id,segment_index",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map([version_id], |row| {
            Ok(SegmentDto {
                device_id: row.get(0)?,
                start_ns: row.get(1)?,
                end_ns: row.get(2)?,
                intercept_num: row.get(3)?,
                intercept_den: row.get(4)?,
                slope_num: row.get(5)?,
                slope_den: row.get(6)?,
                uncertainty_ns: row.get(7)?,
            })
        })
        .map_err(sql_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sql_error)
}

fn build_models_from_segments(segments: &[SegmentDto]) -> Result<Vec<ClockModel>, String> {
    let device_ids = segments
        .iter()
        .map(|segment| segment.device_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut models = Vec::new();
    for device_id in device_ids {
        let specs = segments
            .iter()
            .filter(|segment| segment.device_id == device_id)
            .map(clock_segment_from_dto)
            .collect::<Result<Vec<_>, String>>()?;
        let model = ClockModel {
            device_id,
            segments: specs,
        };
        model.validate()?;
        models.push(model);
    }
    Ok(models)
}

fn clock_segment_from_dto(segment: &SegmentDto) -> Result<ClockSegment, String> {
    let intercept_den = parse_i128(&segment.intercept_den)?;
    let slope_den = parse_i128(&segment.slope_den)?;
    if intercept_den <= 0 || slope_den <= 0 {
        return Err("stored clock segment has a non-positive denominator".into());
    }
    Ok(ClockSegment {
        start_ns: segment.start_ns,
        end_ns: segment.end_ns,
        intercept: Rational::new(parse_i128(&segment.intercept_num)?, intercept_den),
        slope: Rational::new(parse_i128(&segment.slope_num)?, slope_den),
        uncertainty_ns: segment.uncertainty_ns,
    })
}

pub fn parse_i128(value: &str) -> Result<i128, String> {
    value
        .parse::<i128>()
        .map_err(|error| format!("invalid stored integer: {error}"))
}

fn build_timeline(
    all_events: &[RecordEventDto],
    record_ids: &[String],
    models: &[ClockModel],
) -> Result<Vec<TimelineEventDto>, String> {
    let mut timeline = Vec::new();
    for event in all_events
        .iter()
        .filter(|event| record_ids.contains(&event.record_id))
    {
        let model = models
            .iter()
            .find(|model| model.device_id == event.device_id)
            .ok_or_else(|| format!("missing clock model for device {}", event.device_id))?;
        let segment_index = model
            .segment_index_for_event(event.local_time_ns)
            .ok_or_else(|| format!("event {} is outside a clock segment", event.event_id))?;
        let model_interval = model.correct_interval(event.local_time_ns)?;
        let low = (model_interval.low as i128 - event.uncertainty_ns as i128)
            .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        let high = (model_interval.high as i128 + event.uncertainty_ns as i128)
            .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        let corrected_time_ns = midpoint(model_interval.low, model_interval.high);
        timeline.push(TimelineEventDto {
            event: event.clone(),
            segment_index,
            corrected_time_ns,
            low_ns: low,
            high_ns: high,
        });
    }
    timeline.sort_by(|left, right| {
        left.low_ns
            .cmp(&right.low_ns)
            .then(left.high_ns.cmp(&right.high_ns))
            .then(left.event.event_id.cmp(&right.event.event_id))
    });
    Ok(timeline)
}

fn midpoint(low: i64, high: i64) -> i64 {
    low / 2 + high / 2 + ((low & 1) + (high & 1)) / 2
}

fn build_pair_relations(timeline: &[TimelineEventDto]) -> Vec<TimelinePairDto> {
    let mut relations = Vec::new();
    for left_index in 0..timeline.len() {
        for right in timeline.iter().skip(left_index + 1) {
            let left = &timeline[left_index];
            let relation = match compare_intervals(
                TimeInterval {
                    low: left.low_ns,
                    high: left.high_ns,
                },
                TimeInterval {
                    low: right.low_ns,
                    high: right.high_ns,
                },
            ) {
                IntervalOrder::Before => "before",
                IntervalOrder::After => "after",
                IntervalOrder::Equal => "equal",
                IntervalOrder::Incomparable => "incomparable",
            };
            relations.push(TimelinePairDto {
                left_event_id: left.event.event_id.clone(),
                right_event_id: right.event.event_id.clone(),
                relation: relation.to_string(),
            });
        }
    }
    relations
}

fn load_reviews(connection: &Connection, version_id: i64) -> Result<Vec<ReviewDto>, String> {
    let mut statement = connection
        .prepare(
            "SELECT review_id,version_id,conclusion,created_at_ns
             FROM reviews WHERE version_id = ?1 ORDER BY review_id",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map([version_id], |row| {
            Ok(ReviewDto {
                review_id: row.get(0)?,
                version_id: row.get(1)?,
                conclusion: row.get(2)?,
                created_at_ns: row.get(3)?,
            })
        })
        .map_err(sql_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(sql_error)
}

fn normalized_events_for_fingerprint(events: &[RecordEventInput]) -> Result<Vec<Value>, String> {
    events
        .iter()
        .map(|event| {
            Ok(serde_json::json!({
                "key": event.key,
                "kind": event.kind,
                "localTimeNs": parse_time_ns(&event.local_time)?,
                "uncertaintyNs": event.uncertainty_ns.max(0),
                "channel": event.channel,
                "label": event.label,
            }))
        })
        .collect()
}

fn require_record_exists(connection: &Connection, record_id: &str) -> Result<(), String> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM records WHERE record_id = ?1",
            [record_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(sql_error)?;
    if exists.is_none() {
        return Err(format!("record {record_id} does not exist"));
    }
    Ok(())
}

fn require_case_exists(connection: &Connection, case_id: i64) -> Result<(), String> {
    let exists = connection
        .query_row("SELECT 1 FROM cases WHERE case_id = ?1", [case_id], |_| {
            Ok(())
        })
        .optional()
        .map_err(sql_error)?;
    if exists.is_none() {
        return Err(format!("case {case_id} does not exist"));
    }
    Ok(())
}

fn unique_sorted(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn sql_error(error: rusqlite::Error) -> String {
    format!("database error: {error}")
}
