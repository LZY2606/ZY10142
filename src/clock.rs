use crate::model::{CapturePoint, Event};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockSegment {
    pub device_id: String,
    pub ordinal: i64,
    pub start: CapturePoint,
    pub end: CapturePoint,
    pub start_local_ns: i64,
    pub end_local_ns: i64,
    pub start_corrected_ns: i64,
    pub end_corrected_ns: i64,
    pub start_radius_ns: i64,
    pub end_radius_ns: i64,
    pub trusted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedEvent {
    pub event_id: String,
    pub device_id: String,
    pub corrected_ns: i64,
    pub min_ns: i64,
    pub max_ns: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntervalOrder {
    Before,
    After,
    Incomparable,
}

pub fn validate_segments(segments: &[ClockSegment]) -> Result<(), String> {
    let mut by_device: BTreeMap<&str, Vec<&ClockSegment>> = BTreeMap::new();
    for segment in segments {
        if segment.start > segment.end {
            return Err("segment capture range is reversed".to_string());
        }
        if segment.trusted {
            if segment.start_radius_ns < 0 || segment.end_radius_ns < 0 {
                return Err("uncertainty radius cannot be negative".to_string());
            }
            if segment.start != segment.end && segment.end_local_ns < segment.start_local_ns {
                return Err("local time cannot reverse inside one clock segment".to_string());
            }
            if segment.end_corrected_ns < segment.start_corrected_ns {
                return Err("corrected time cannot reverse inside one clock segment".to_string());
            }
            by_device
                .entry(segment.device_id.as_str())
                .or_default()
                .push(segment);
        }
    }

    for device_segments in by_device.values_mut() {
        device_segments.sort_by_key(|segment| (segment.start, segment.ordinal));
        for pair in device_segments.windows(2) {
            let previous = pair[0];
            let next = pair[1];
            if previous.end > next.start {
                return Err("clock segments overlap in capture order".to_string());
            }
            if previous.end == next.start {
                if previous.end_local_ns != next.start_local_ns {
                    return Err(
                        "segments sharing an endpoint must use the same local time".to_string()
                    );
                }
            }
            if previous.end_corrected_ns > next.start_corrected_ns {
                return Err("corrected time cannot reverse across a clock jump".to_string());
            }
        }
    }
    Ok(())
}

pub fn map_events(events: &[Event], segments: &[ClockSegment]) -> Result<Vec<MappedEvent>, String> {
    validate_segments(segments)?;
    let mut by_device: BTreeMap<&str, Vec<&ClockSegment>> = BTreeMap::new();
    for segment in segments {
        by_device
            .entry(segment.device_id.as_str())
            .or_default()
            .push(segment);
    }
    for device_segments in by_device.values_mut() {
        device_segments.sort_by_key(|segment| (segment.start, segment.ordinal));
    }

    let capture = |event: &Event| CapturePoint {
        record_seq: event.record_seq,
        event_seq: event.seq,
    };

    let mut mapped = Vec::new();
    for event in events {
        let point = capture(event);
        let device_segments = by_device
            .get(event.device_id.as_str())
            .ok_or_else(|| format!("device {} has no clock segment", event.device_id))?;
        let covering: Vec<usize> = device_segments
            .iter()
            .enumerate()
            .filter_map(|(index, segment)| {
                (segment.start <= point && point <= segment.end).then_some(index)
            })
            .collect();
        if covering.is_empty() {
            return Err(format!("event {} is outside clock segments", event.id));
        }
        let index = covering[0];
        let index = covering
            .iter()
            .copied()
            .find(|candidate| device_segments[*candidate].start == point)
            .unwrap_or(index);
        let segment = device_segments[index];
        if !segment.trusted {
            continue;
        }

        let local_delta = event.local_ns - segment.start_local_ns;
        if local_delta < 0 {
            return Err(format!(
                "event {} precedes its segment local start",
                event.id
            ));
        }
        let corrected_delta = rounded_affine(segment, local_delta)?;
        let radius = interpolated_radius(segment, local_delta)?;
        let corrected = segment.start_corrected_ns + corrected_delta;
        mapped.push(MappedEvent {
            event_id: event.id.clone(),
            device_id: event.device_id.clone(),
            corrected_ns: corrected,
            min_ns: corrected - radius,
            max_ns: corrected + radius,
        });
    }
    Ok(mapped)
}

fn rounded_affine(segment: &ClockSegment, local_delta: i64) -> Result<i64, String> {
    let local_span = segment.end_local_ns - segment.start_local_ns;
    let corrected_span = segment.end_corrected_ns - segment.start_corrected_ns;
    if segment.start == segment.end || local_span == 0 {
        if segment.start == segment.end && local_delta == 0 {
            return Ok(0);
        }
        if segment.start == segment.end {
            return Err("singleton segment can map its anchor event only".to_string());
        }
        return Err("distinct capture points cannot share the same local time".to_string());
    }
    if local_delta > local_span {
        return Err("event follows its segment local end".to_string());
    }
    let product = i128::from(corrected_span) * i128::from(local_delta);
    let nearest = (product * 2 + i128::from(local_span)) / (2 * i128::from(local_span));
    i64::try_from(nearest).map_err(|_| "corrected time overflow".to_string())
}

fn interpolated_radius(segment: &ClockSegment, local_delta: i64) -> Result<i64, String> {
    if segment.start == segment.end {
        return Ok(segment.start_radius_ns);
    }
    let local_span = i128::from(segment.end_local_ns - segment.start_local_ns);
    let radius_span = i128::from(segment.end_radius_ns - segment.start_radius_ns);
    let delta = i128::from(local_delta);
    let scaled = (radius_span * delta * 2
        + i128::from(segment.end_radius_ns - segment.start_radius_ns).signum() * local_span)
        / (2 * local_span);
    let value = i128::from(segment.start_radius_ns) + scaled;
    i64::try_from(value.max(0)).map_err(|_| "uncertainty radius overflow".to_string())
}

pub fn compare_closed_intervals(
    left_min: i64,
    left_max: i64,
    right_min: i64,
    right_max: i64,
) -> IntervalOrder {
    if left_max < right_min {
        IntervalOrder::Before
    } else if right_max < left_min {
        IntervalOrder::After
    } else {
        IntervalOrder::Incomparable
    }
}
