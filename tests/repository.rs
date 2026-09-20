use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{SystemTime, UNIX_EPOCH};

static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

use grid_timeline_review::db::Repository;
use grid_timeline_review::model::*;

fn unique_db() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join(format!(
            "grid-timeline-{}-{}-{}.sqlite3",
            std::process::id(),
            nanos,
            counter
        ))
        .to_string_lossy()
        .to_string()
}

fn batch() -> BatchInput {
    BatchInput {
        device_code: "TEST-RELAY".to_string(),
        device_name: "Test relay".to_string(),
        sample_rate_hz: 4000,
        records: vec![RecordInput {
            seq: 0,
            source_name: "summary.json".to_string(),
            sample_rate_hz: 4000,
            start_local_ns: Some(0),
            events: vec![
                EventInput {
                    event_type: "frequency".to_string(),
                    label: "start anchor".to_string(),
                    local_ns: 100,
                    frequency_hz: Some(49.98),
                    phasor_magnitude: None,
                    phasor_angle_deg: None,
                },
                EventInput {
                    event_type: "trip".to_string(),
                    label: "end anchor".to_string(),
                    local_ns: 200,
                    frequency_hz: None,
                    phasor_magnitude: None,
                    phasor_angle_deg: None,
                },
            ],
        }],
    }
}

fn two_segment_batch() -> BatchInput {
    let mut input = batch();
    input.records.push(RecordInput {
        seq: 1,
        source_name: "second-summary.json".to_string(),
        sample_rate_hz: 4000,
        start_local_ns: Some(300),
        events: vec![
            EventInput {
                event_type: "frequency".to_string(),
                label: "second start".to_string(),
                local_ns: 300,
                frequency_hz: None,
                phasor_magnitude: None,
                phasor_angle_deg: None,
            },
            EventInput {
                event_type: "trip".to_string(),
                label: "second end".to_string(),
                local_ns: 400,
                frequency_hz: None,
                phasor_magnitude: None,
                phasor_angle_deg: None,
            },
        ],
    });
    input
}

fn setup() -> (
    Repository,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
) {
    let path = unique_db();
    let repository = Repository::new(&path).unwrap();
    let summary = repository.import_batch(batch()).unwrap();
    assert_eq!(summary.inserted_records, 1);
    let state = repository.state().unwrap();
    let record_id = state.records[0].id.clone();
    let device_id = state.records[0].device_id.clone();
    let start_event = state.events[0].id.clone();
    let end_event = state.events[1].id.clone();
    let d1 = repository
        .create_disturbance(DisturbanceInput {
            code: "D1".to_string(),
            title: "First".to_string(),
        })
        .unwrap()
        .id;
    let d2 = repository
        .create_disturbance(DisturbanceInput {
            code: "D2".to_string(),
            title: "Second".to_string(),
        })
        .unwrap()
        .id;
    (
        repository,
        path,
        record_id,
        device_id,
        start_event,
        end_event,
        d1,
        d2,
    )
}

fn publish_input(
    disturbance_id: &str,
    record_id: &str,
    device_id: &str,
    start_event: &str,
    end_event: &str,
) -> PublishAlignmentInput {
    PublishAlignmentInput {
        disturbance_id: disturbance_id.to_string(),
        note: String::new(),
        record_ids: vec![record_id.to_string()],
        segments: vec![SegmentInput {
            device_id: device_id.to_string(),
            start_event_id: start_event.to_string(),
            end_event_id: end_event.to_string(),
            start_corrected_ns: 1000,
            end_corrected_ns: 1100,
            start_radius_ns: 2,
            end_radius_ns: 2,
            trusted: true,
        }],
    }
}

#[test]
fn import_uses_device_and_content_fingerprint_to_skip_duplicates() {
    let (repository, _path, _, _, _, _, _, _) = setup();
    let duplicate = repository.import_batch(batch()).unwrap();
    assert_eq!(duplicate.inserted_records, 0);
    assert_eq!(duplicate.skipped_records, 1);
    assert_eq!(repository.state().unwrap().records.len(), 1);
}

#[test]
fn candidate_can_coexist_but_published_record_has_one_owner() {
    let (repository, _path, record_id, device_id, start_event, end_event, d1, d2) = setup();
    repository
        .add_candidate(CandidateInput {
            disturbance_id: d1.clone(),
            record_id: record_id.clone(),
        })
        .unwrap();
    repository
        .add_candidate(CandidateInput {
            disturbance_id: d2.clone(),
            record_id: record_id.clone(),
        })
        .unwrap();
    assert_eq!(repository.state().unwrap().memberships.len(), 2);

    repository
        .publish_alignment(
            publish_input(&d1, &record_id, &device_id, &start_event, &end_event),
            Some(1),
        )
        .unwrap();
    let conflict = repository.publish_alignment(
        publish_input(&d2, &record_id, &device_id, &start_event, &end_event),
        Some(2),
    );
    assert!(conflict.is_err());

    let state = repository.state().unwrap();
    let published: Vec<_> = state
        .memberships
        .iter()
        .filter(|item| item.status == "published")
        .collect();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].disturbance_id, d1);
    assert_eq!(state.versions.len(), 1);
}

#[test]
fn failed_publish_rolls_back_and_keeps_previous_version_replayable() {
    let (repository, _path, record_id, _device_id, start_event, _end_event, d1, d2) = setup();
    repository
        .publish_alignment(
            publish_input(&d1, &record_id, &_device_id, &start_event, &_end_event),
            Some(1),
        )
        .unwrap();
    let mut invalid = publish_input(&d2, &record_id, &_device_id, &start_event, &_end_event);
    invalid.segments[0].end_corrected_ns = 900;
    assert!(repository.publish_alignment(invalid, Some(2)).is_err());
    let state = repository.state().unwrap();
    assert_eq!(state.versions.len(), 1);
    assert_eq!(state.versions[0].timings.len(), 2);
}

#[test]
fn concurrent_publish_of_same_record_exposes_only_one_owner() {
    let (repository, path, record_id, device_id, start_event, end_event, d1, d2) = setup();
    drop(repository);
    let barrier = Arc::new(Barrier::new(2));
    let parameters = (
        path.clone(),
        record_id.clone(),
        device_id.clone(),
        start_event.clone(),
        end_event.clone(),
        d1.clone(),
        d2.clone(),
        barrier.clone(),
    );
    let first = std::thread::spawn(move || {
        let (path, record, device, start, end, d1, _d2, barrier) = parameters;
        let repository = Repository::new(&path).unwrap();
        barrier.wait();
        repository.publish_alignment(publish_input(&d1, &record, &device, &start, &end), Some(10))
    });
    let parameters = (
        path.clone(),
        record_id,
        device_id,
        start_event,
        end_event,
        d1,
        d2,
        barrier,
    );
    let second = std::thread::spawn(move || {
        let (path, record, device, start, end, _d1, d2, barrier) = parameters;
        let repository = Repository::new(&path).unwrap();
        barrier.wait();
        repository.publish_alignment(publish_input(&d2, &record, &device, &start, &end), Some(11))
    });
    let results = (
        first.join().unwrap().is_ok(),
        second.join().unwrap().is_ok(),
    );
    assert_eq!((results.0 as u8) + (results.1 as u8), 1);

    let state = Repository::new(&path).unwrap().state().unwrap();
    assert_eq!(state.versions.len(), 1);
    assert_eq!(
        state
            .memberships
            .iter()
            .filter(|item| item.status == "published")
            .count(),
        1
    );
}

#[test]
fn untrusted_segment_is_versioned_without_timing_but_still_publishes_trusted_part() {
    let path = unique_db();
    let repository = Repository::new(&path).unwrap();
    repository.import_batch(two_segment_batch()).unwrap();
    let state = repository.state().unwrap();
    let disturbance = repository
        .create_disturbance(DisturbanceInput {
            code: "UNTRUSTED".to_string(),
            title: "Untrusted anchor".to_string(),
        })
        .unwrap();
    let records: Vec<_> = state
        .records
        .iter()
        .map(|record| record.id.clone())
        .collect();
    let event_id = |label: &str| {
        state
            .events
            .iter()
            .find(|event| event.label == label)
            .unwrap()
            .id
            .clone()
    };
    let device_id = state.devices[0].id.clone();
    let version = repository
        .publish_alignment(
            PublishAlignmentInput {
                disturbance_id: disturbance.id,
                note: "Second anchor pair is not trusted".to_string(),
                record_ids: records.clone(),
                segments: vec![
                    SegmentInput {
                        device_id: device_id.clone(),
                        start_event_id: event_id("start anchor"),
                        end_event_id: event_id("end anchor"),
                        start_corrected_ns: 1000,
                        end_corrected_ns: 1100,
                        start_radius_ns: 0,
                        end_radius_ns: 0,
                        trusted: true,
                    },
                    SegmentInput {
                        device_id,
                        start_event_id: event_id("second start"),
                        end_event_id: event_id("second end"),
                        start_corrected_ns: 1200,
                        end_corrected_ns: 1300,
                        start_radius_ns: 0,
                        end_radius_ns: 0,
                        trusted: false,
                    },
                ],
            },
            Some(3),
        )
        .unwrap();
    assert_eq!(version.record_ids.len(), 2);
    assert_eq!(version.segments.len(), 2);
    assert_eq!(version.timings.len(), 2);
    assert!(version.segments.iter().any(|segment| !segment.trusted));
}
