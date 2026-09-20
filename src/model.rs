use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CapturePoint {
    pub record_seq: i64,
    pub event_seq: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventInput {
    pub event_type: String,
    pub label: String,
    pub local_ns: i64,
    #[serde(default)]
    pub frequency_hz: Option<f64>,
    #[serde(default)]
    pub phasor_magnitude: Option<f64>,
    #[serde(default)]
    pub phasor_angle_deg: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RecordInput {
    pub seq: i64,
    pub source_name: String,
    pub sample_rate_hz: i64,
    #[serde(default)]
    pub start_local_ns: Option<i64>,
    pub events: Vec<EventInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BatchInput {
    pub device_code: String,
    pub device_name: String,
    pub sample_rate_hz: i64,
    pub records: Vec<RecordInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DisturbanceInput {
    pub code: String,
    pub title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CandidateInput {
    pub disturbance_id: String,
    pub record_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SegmentInput {
    pub device_id: String,
    pub start_event_id: String,
    pub end_event_id: String,
    pub start_corrected_ns: i64,
    pub end_corrected_ns: i64,
    #[serde(default = "default_radius")]
    pub start_radius_ns: i64,
    #[serde(default = "default_radius")]
    pub end_radius_ns: i64,
    #[serde(default = "default_trusted")]
    pub trusted: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublishAlignmentInput {
    pub disturbance_id: String,
    #[serde(default)]
    pub note: String,
    pub record_ids: Vec<String>,
    pub segments: Vec<SegmentInput>,
}

fn default_radius() -> i64 {
    0
}

fn default_trusted() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub id: String,
    pub code: String,
    pub name: String,
    pub sample_rate_hz: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventRecord {
    pub id: String,
    pub batch_id: String,
    pub device_id: String,
    pub seq: i64,
    pub source_name: String,
    pub sample_rate_hz: i64,
    pub start_local_ns: Option<i64>,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub id: String,
    pub record_id: String,
    pub device_id: String,
    pub record_seq: i64,
    pub seq: i64,
    pub event_type: String,
    pub label: String,
    pub local_ns: i64,
    pub frequency_hz: Option<f64>,
    pub phasor_magnitude: Option<f64>,
    pub phasor_angle_deg: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Disturbance {
    pub id: String,
    pub code: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Membership {
    pub disturbance_id: String,
    pub record_id: String,
    pub status: String,
    pub published_version_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub id: String,
    pub device_id: String,
    pub ordinal: i64,
    pub start_event_id: String,
    pub end_event_id: String,
    pub start_capture: CapturePoint,
    pub end_capture: CapturePoint,
    pub start_local_ns: i64,
    pub end_local_ns: i64,
    pub start_corrected_ns: i64,
    pub end_corrected_ns: i64,
    pub start_radius_ns: i64,
    pub end_radius_ns: i64,
    pub trusted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventTiming {
    pub event_id: String,
    pub device_id: String,
    pub segment_id: String,
    pub corrected_ns: i64,
    pub min_ns: i64,
    pub max_ns: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlignmentVersion {
    pub id: String,
    pub disturbance_id: String,
    pub version_no: i64,
    pub note: String,
    pub published_ns: i64,
    pub record_ids: Vec<String>,
    pub segments: Vec<Segment>,
    pub timings: Vec<EventTiming>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderRelation {
    pub earlier_event_id: String,
    pub later_event_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewState {
    pub devices: Vec<Device>,
    pub records: Vec<EventRecord>,
    pub events: Vec<Event>,
    pub disturbances: Vec<Disturbance>,
    pub memberships: Vec<Membership>,
    pub versions: Vec<AlignmentVersion>,
    pub deterministic_order: Vec<OrderRelation>,
}
