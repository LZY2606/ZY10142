use grid_timeline_review::clock::{
    ClockSegment, IntervalOrder, compare_closed_intervals, map_events,
};
use grid_timeline_review::model::{CapturePoint, Event};

#[test]
fn closed_intervals_are_incomparable_when_overlapping_or_touching() {
    assert_eq!(
        compare_closed_intervals(0, 10, 11, 20),
        IntervalOrder::Before
    );
    assert_eq!(
        compare_closed_intervals(11, 20, 0, 10),
        IntervalOrder::After
    );
    assert_eq!(
        compare_closed_intervals(0, 10, 10, 20),
        IntervalOrder::Incomparable
    );
    assert_eq!(
        compare_closed_intervals(0, 12, 10, 20),
        IntervalOrder::Incomparable
    );
}

fn event(id: &str, record_seq: i64, seq: i64, local_ns: i64) -> Event {
    Event {
        id: id.to_string(),
        record_id: format!("record-{record_seq}"),
        device_id: "device".to_string(),
        record_seq,
        seq,
        event_type: "test".to_string(),
        label: id.to_string(),
        local_ns,
        frequency_hz: None,
        phasor_magnitude: None,
        phasor_angle_deg: None,
    }
}

#[test]
fn piecewise_map_supports_discontinuous_clock_jump_with_one_endpoint_owner() {
    let events = vec![
        event("before", 0, 0, 100),
        event("jump", 0, 1, 200),
        event("after", 0, 2, 300),
    ];
    let segments = vec![
        ClockSegment {
            device_id: "device".to_string(),
            ordinal: 0,
            start: CapturePoint {
                record_seq: 0,
                event_seq: 0,
            },
            end: CapturePoint {
                record_seq: 0,
                event_seq: 1,
            },
            start_local_ns: 100,
            end_local_ns: 200,
            start_corrected_ns: 1000,
            end_corrected_ns: 1100,
            start_radius_ns: 0,
            end_radius_ns: 0,
            trusted: true,
        },
        ClockSegment {
            device_id: "device".to_string(),
            ordinal: 1,
            start: CapturePoint {
                record_seq: 0,
                event_seq: 1,
            },
            end: CapturePoint {
                record_seq: 0,
                event_seq: 2,
            },
            start_local_ns: 200,
            end_local_ns: 300,
            start_corrected_ns: 2000,
            end_corrected_ns: 2100,
            start_radius_ns: 0,
            end_radius_ns: 0,
            trusted: true,
        },
    ];
    let mapped = map_events(&events, &segments).unwrap();
    assert_eq!(mapped.len(), 3);
    assert_eq!(mapped[0].corrected_ns, 1000);
    assert_eq!(mapped[1].corrected_ns, 2000);
    assert_eq!(mapped[2].corrected_ns, 2100);
}

#[test]
fn clock_jump_cannot_move_corrected_time_backwards() {
    let segments = vec![
        ClockSegment {
            device_id: "device".to_string(),
            ordinal: 0,
            start: CapturePoint {
                record_seq: 0,
                event_seq: 0,
            },
            end: CapturePoint {
                record_seq: 0,
                event_seq: 1,
            },
            start_local_ns: 100,
            end_local_ns: 200,
            start_corrected_ns: 2000,
            end_corrected_ns: 2100,
            start_radius_ns: 0,
            end_radius_ns: 0,
            trusted: true,
        },
        ClockSegment {
            device_id: "device".to_string(),
            ordinal: 1,
            start: CapturePoint {
                record_seq: 0,
                event_seq: 1,
            },
            end: CapturePoint {
                record_seq: 0,
                event_seq: 2,
            },
            start_local_ns: 200,
            end_local_ns: 300,
            start_corrected_ns: 1900,
            end_corrected_ns: 2000,
            start_radius_ns: 0,
            end_radius_ns: 0,
            trusted: true,
        },
    ];
    assert!(map_events(&[], &segments).is_err());
}
