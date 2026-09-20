use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordEventInput {
    pub key: String,
    pub kind: String,
    #[serde(alias = "localTimeNs")]
    pub local_time: Value,
    #[serde(default)]
    pub uncertainty_ns: i64,
    pub channel: Option<String>,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordInput {
    pub device_id: String,
    #[serde(alias = "localTimeNs")]
    pub local_time: Value,
    pub sample_rate_hz: Option<f64>,
    #[serde(default)]
    pub digital_changes: Vec<Value>,
    #[serde(default)]
    pub phasor_features: Vec<Value>,
    #[serde(default)]
    pub events: Vec<RecordEventInput>,
    #[serde(default)]
    pub raw: Option<Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub records: Vec<RecordInput>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedRecord {
    pub record_id: String,
    pub device_id: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResponse {
    pub imported: Vec<ImportedRecord>,
    pub skipped_duplicates: Vec<ImportedRecord>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCaseRequest {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub record_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignRequest {
    pub record_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAnchorRequest {
    pub anchor_key: String,
    pub label: String,
    #[serde(default)]
    pub corrected_time_ns: Option<i64>,
    #[serde(default)]
    pub uncertainty_ns: i64,
    #[serde(default = "default_true")]
    pub trusted: bool,
    #[serde(default)]
    pub event_refs: Vec<AnchorEventRef>,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnchorEventRef {
    pub record_id: String,
    pub event_key: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVersionRequest {
    pub note: String,
    #[serde(default)]
    pub jumps_by_device: std::collections::BTreeMap<String, Vec<i64>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRequest {
    pub conclusion: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordEventDto {
    pub event_id: String,
    pub record_id: String,
    pub device_id: String,
    pub key: String,
    pub kind: String,
    pub local_time_ns: i64,
    pub uncertainty_ns: i64,
    pub channel: Option<String>,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordDto {
    pub record_id: String,
    pub device_id: String,
    pub local_time_ns: i64,
    pub sample_rate_hz: Option<f64>,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnchorDto {
    pub anchor_key: String,
    pub label: String,
    pub corrected_time_ns: Option<i64>,
    pub uncertainty_ns: i64,
    pub trusted: bool,
    pub event_refs: Vec<AnchorEventRef>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentDto {
    pub device_id: String,
    pub start_ns: Option<i64>,
    pub end_ns: Option<i64>,
    pub intercept_num: String,
    pub intercept_den: String,
    pub slope_num: String,
    pub slope_den: String,
    pub uncertainty_ns: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEventDto {
    pub event: RecordEventDto,
    pub segment_index: usize,
    pub corrected_time_ns: i64,
    pub low_ns: i64,
    pub high_ns: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelinePairDto {
    pub left_event_id: String,
    pub right_event_id: String,
    pub relation: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDto {
    pub review_id: i64,
    pub version_id: i64,
    pub conclusion: String,
    pub created_at_ns: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDto {
    pub version_id: i64,
    pub case_id: i64,
    pub version_no: i64,
    pub note: String,
    pub published: bool,
    pub created_at_ns: i64,
    pub published_at_ns: Option<i64>,
    pub segments: Vec<SegmentDto>,
    pub timeline: Vec<TimelineEventDto>,
    pub pair_relations: Vec<TimelinePairDto>,
    pub reviews: Vec<ReviewDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseDto {
    pub case_id: i64,
    pub name: String,
    pub description: String,
    pub created_at_ns: i64,
    pub candidate_record_ids: Vec<String>,
    pub published_record_ids: Vec<String>,
    pub versions: Vec<VersionDto>,
    pub anchors: Vec<AnchorDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StateResponse {
    pub records: Vec<RecordDto>,
    pub events: Vec<RecordEventDto>,
    pub cases: Vec<CaseDto>,
}
