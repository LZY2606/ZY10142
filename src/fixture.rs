use crate::db::Repository;
use crate::model::*;

pub const FIXTURE_DISTURBANCE_CODE: &str = "D-2026-001";
pub const ALT_DISTURBANCE_CODE: &str = "D-2026-002";

fn event(event_type: &str, label: &str, local_ns: i64, frequency_hz: Option<f64>) -> EventInput {
    EventInput {
        event_type: event_type.to_string(),
        label: label.to_string(),
        local_ns,
        frequency_hz,
        phasor_magnitude: Some(1.0),
        phasor_angle_deg: Some(0.0),
    }
}

pub fn seed(repository: &Repository) -> Result<(), String> {
    let batches = vec![
        BatchInput {
            device_code: "RLY-A".to_string(),
            device_name: "Feeder A protection relay".to_string(),
            sample_rate_hz: 4000,
            records: vec![RecordInput {
                seq: 0,
                source_name: "A-desensitized-summary.json".to_string(),
                sample_rate_hz: 4000,
                start_local_ns: Some(101_999_000_000),
                events: vec![
                    event(
                        "frequency",
                        "A frequency feature",
                        102_000_000_000,
                        Some(49.97),
                    ),
                    event("digital_edge", "A digital edge", 102_025_000_000, None),
                    event("trip", "A trip", 102_050_100_000, None),
                    event("reclose", "A reclose close", 102_130_005_000, None),
                ],
            }],
        },
        BatchInput {
            device_code: "RLY-B".to_string(),
            device_name: "Feeder B protection relay".to_string(),
            sample_rate_hz: 5000,
            records: vec![RecordInput {
                seq: 0,
                source_name: "B-desensitized-summary.json".to_string(),
                sample_rate_hz: 5000,
                start_local_ns: Some(98_499_000_000),
                events: vec![
                    event(
                        "frequency",
                        "B frequency feature",
                        98_500_000_000,
                        Some(49.97),
                    ),
                    event("digital_edge", "B digital edge", 98_526_000_000, None),
                    event("trip", "B trip", 98_551_100_000, None),
                    event("reclose", "B reclose close", 98_632_000_000, None),
                ],
            }],
        },
        BatchInput {
            device_code: "RLY-C".to_string(),
            device_name: "Bus C protection relay".to_string(),
            sample_rate_hz: 4000,
            records: vec![RecordInput {
                seq: 0,
                source_name: "C-desensitized-summary.json".to_string(),
                sample_rate_hz: 4000,
                start_local_ns: Some(99_499_000_000),
                events: vec![
                    event(
                        "frequency",
                        "C frequency feature",
                        99_500_000_000,
                        Some(49.97),
                    ),
                    event(
                        "digital_edge",
                        "C pre-jump digital edge",
                        99_501_000_000,
                        None,
                    ),
                    event(
                        "time_sync",
                        "C time synchronization jump",
                        100_501_000_000,
                        None,
                    ),
                    event("trip", "C trip", 100_550_100_000, None),
                    event("reclose", "C reclose close", 100_580_000_000, None),
                ],
            }],
        },
    ];
    for batch in batches {
        repository.import_batch(batch)?;
    }

    let primary = repository.create_disturbance(DisturbanceInput {
        code: FIXTURE_DISTURBANCE_CODE.to_string(),
        title: "Desensitized feeder disturbance".to_string(),
    })?;
    repository.create_disturbance(DisturbanceInput {
        code: ALT_DISTURBANCE_CODE.to_string(),
        title: "Pending alternate assignment".to_string(),
    })?;

    let state = repository.state()?;
    if state
        .versions
        .iter()
        .any(|version| version.disturbance_id == primary.id)
    {
        return Ok(());
    }
    let record_a = state
        .records
        .iter()
        .find(|record| record.source_name == "A-desensitized-summary.json")
        .ok_or_else(|| "fixture record A missing".to_string())?;
    let record_b = state
        .records
        .iter()
        .find(|record| record.source_name == "B-desensitized-summary.json")
        .ok_or_else(|| "fixture record B missing".to_string())?;
    let record_c = state
        .records
        .iter()
        .find(|record| record.source_name == "C-desensitized-summary.json")
        .ok_or_else(|| "fixture record C missing".to_string())?;

    repository.add_candidate(CandidateInput {
        disturbance_id: primary.id.clone(),
        record_id: record_a.id.clone(),
    })?;
    repository.add_candidate(CandidateInput {
        disturbance_id: primary.id.clone(),
        record_id: record_b.id.clone(),
    })?;
    repository.add_candidate(CandidateInput {
        disturbance_id: primary.id.clone(),
        record_id: record_c.id.clone(),
    })?;
    repository.add_candidate(CandidateInput {
        disturbance_id: repository
            .state()?
            .disturbances
            .iter()
            .find(|item| item.code == ALT_DISTURBANCE_CODE)
            .map(|item| item.id.clone())
            .ok_or_else(|| "alternate disturbance missing".to_string())?,
        record_id: record_c.id.clone(),
    })?;

    let event_id = |label: &str| {
        state
            .events
            .iter()
            .find(|item| item.label == label)
            .map(|item| item.id.clone())
            .ok_or_else(|| format!("fixture event missing: {label}"))
    };
    repository.publish_alignment(
        PublishAlignmentInput {
            disturbance_id: primary.id,
            note: "Fixed fixture: digital-edge and frequency anchors; C includes a clock jump."
                .to_string(),
            record_ids: vec![
                record_a.id.clone(),
                record_b.id.clone(),
                record_c.id.clone(),
            ],
            segments: vec![
                SegmentInput {
                    device_id: record_a.device_id.clone(),
                    start_event_id: event_id("A frequency feature")?,
                    end_event_id: event_id("A reclose close")?,
                    start_corrected_ns: 100_000_000_000,
                    end_corrected_ns: 100_130_005_000,
                    start_radius_ns: 5_000,
                    end_radius_ns: 8_000,
                    trusted: true,
                },
                SegmentInput {
                    device_id: record_b.device_id.clone(),
                    start_event_id: event_id("B frequency feature")?,
                    end_event_id: event_id("B reclose close")?,
                    start_corrected_ns: 100_000_000_000,
                    end_corrected_ns: 100_132_000_000,
                    start_radius_ns: 5_000,
                    end_radius_ns: 12_000,
                    trusted: true,
                },
                SegmentInput {
                    device_id: record_c.device_id.clone(),
                    start_event_id: event_id("C frequency feature")?,
                    end_event_id: event_id("C time synchronization jump")?,
                    start_corrected_ns: 100_000_000_000,
                    end_corrected_ns: 100_000_100_000,
                    start_radius_ns: 5_000,
                    end_radius_ns: 12_000,
                    trusted: true,
                },
                SegmentInput {
                    device_id: record_c.device_id.clone(),
                    start_event_id: event_id("C time synchronization jump")?,
                    end_event_id: event_id("C reclose close")?,
                    start_corrected_ns: 100_000_100_000,
                    end_corrected_ns: 100_080_000_000,
                    start_radius_ns: 12_000,
                    end_radius_ns: 8_000,
                    trusted: true,
                },
            ],
        },
        Some(100_000_000_000),
    )?;
    Ok(())
}
