use crate::service::App;
use crate::types::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn seed_if_empty(app: &mut App) -> Result<(), String> {
    if app.record_count()? != 0 {
        return Ok(());
    }
    let import_request: ImportRequest = from_json(json!({
        "records": [
            {
                "deviceId": "REL-A",
                "localTime": "2026-09-21T09:00:00Z",
                "sampleRateHz": 4000,
                "digitalChanges": [{"channel": "DI1", "value": 1, "localTimeNs": 995_000_000}],
                "phasorFeatures": [{"feature": "frequency_dip", "localTimeNs": 1_000_000_000}],
                "events": [
                    {"key": "feature", "kind": "feature", "localTimeNs": 1_000_000_000, "uncertaintyNs": 2, "label": "频率下陷"},
                    {"key": "trip", "kind": "trip", "localTimeNs": 2_000_000_000, "uncertaintyNs": 2, "label": "跳闸出口"}
                ]
            },
            {
                "deviceId": "REL-B",
                "localTime": "2026-09-21T09:00:00Z",
                "sampleRateHz": 5000,
                "digitalChanges": [{"channel": "DI7", "value": 1, "localTimeNs": 1_400_000_000}],
                "phasorFeatures": [{"feature": "frequency_dip", "localTimeNs": 995_000_000}],
                "events": [
                    {"key": "feature", "kind": "feature", "localTimeNs": 995_000_000, "uncertaintyNs": 3, "label": "频率下陷"},
                    {"key": "prejump_edge", "kind": "digital", "localTimeNs": 1_400_000_000, "uncertaintyNs": 1, "channel": "DI7", "label": "跳变前数字边沿"},
                    {"key": "trip", "kind": "trip", "localTimeNs": 1_500_000_000, "uncertaintyNs": 3, "label": "跳闸出口"},
                    {"key": "reclose", "kind": "reclose", "localTimeNs": 2_000_000_000, "uncertaintyNs": 3, "label": "重合闸"}
                ]
            },
            {
                "deviceId": "REL-C",
                "localTime": "2026-09-21T09:00:00Z",
                "sampleRateHz": 4000,
                "digitalChanges": [],
                "phasorFeatures": [{"feature": "frequency_dip", "localTimeNs": 990_000_000}],
                "events": [
                    {"key": "feature", "kind": "feature", "localTimeNs": 990_000_000, "uncertaintyNs": 2, "label": "频率下陷"},
                    {"key": "trip", "kind": "trip", "localTimeNs": 2_000_000_000, "uncertaintyNs": 2, "label": "跳闸出口"}
                ]
            }
        ]
    }))?;
    let imported = app.import_records(import_request)?;
    let record_ids = imported
        .imported
        .iter()
        .map(|record| record.record_id.clone())
        .collect::<Vec<_>>();

    let case_id = app.create_case(from_json(json!({
        "name": "线路瞬时故障复盘",
        "description": "固定 fixture：REL-B 在本地 1.5s 处有校时跳变；启动特征区间重叠，不强行排序。",
        "recordIds": record_ids
    }))?)?;
    let competitor_id = app.create_case(from_json(json!({
        "name": "竞争归属候选（未发布）",
        "description": "同一保护记录可在未决候选中并存；发布新版本时应被唯一归属规则拒绝。",
        "recordIds": [record_ids[0]]
    }))?)?;

    let by_device = imported
        .imported
        .iter()
        .map(|record| (record.device_id.clone(), record.record_id.clone()))
        .collect::<BTreeMap<_, _>>();
    save(
        app,
        "feature",
        "共同频率下陷",
        1_000_000_000,
        4,
        &[
            (&by_device["REL-A"], "feature"),
            (&by_device["REL-B"], "feature"),
            (&by_device["REL-C"], "feature"),
        ],
    )?;
    save(
        app,
        "edge-b",
        "REL-B 跳变前数字边沿",
        1_404_938_271,
        1,
        &[(&by_device["REL-B"], "prejump_edge")],
    )?;
    save(
        app,
        "trip",
        "各装置跳闸出口",
        2_000_000_000,
        4,
        &[
            (&by_device["REL-A"], "trip"),
            (&by_device["REL-B"], "trip"),
            (&by_device["REL-C"], "trip"),
        ],
    )?;
    save(
        app,
        "reclose-b",
        "REL-B 重合闸",
        2_500_000_000,
        3,
        &[(&by_device["REL-B"], "reclose")],
    )?;

    let version_request: CreateVersionRequest = from_json(json!({
        "note": "v1：信任共同频率、跳闸、重合闸和跳变前边沿；REL-B 分段点 1.5s。",
        "jumpsByDevice": {"REL-B": [1_500_000_000]}
    }))?;
    let version_id = app.create_version(case_id, version_request)?;
    app.add_review(version_id, from_json(json!({
        "conclusion": "启动特征在不确定区间内重叠，保留 incomparable；跳闸与重合闸偏序可用于复盘。"
    }))?)?;
    app.publish_version(version_id)?;

    let competitor_version = app.create_version(
        competitor_id,
        from_json(json!({"note": "竞争案例草稿；尝试发布会触发唯一归属约束。"}))?,
    )?;
    app.add_review(
        competitor_version,
        from_json(json!({
            "conclusion": "该版本仍为未决候选，不改变已发布归属。"
        }))?,
    )?;
    Ok(())
}

fn save(
    app: &mut App,
    key: &str,
    label: &str,
    corrected_time_ns: i64,
    uncertainty_ns: i64,
    refs: &[(&str, &str)],
) -> Result<(), String> {
    let event_refs = refs
        .iter()
        .map(|(record_id, event_key)| AnchorEventRef {
            record_id: (*record_id).to_string(),
            event_key: (*event_key).to_string(),
        })
        .collect();
    app.save_anchor(SaveAnchorRequest {
        anchor_key: key.into(),
        label: label.into(),
        corrected_time_ns: Some(corrected_time_ns),
        uncertainty_ns,
        trusted: true,
        event_refs,
    })
}

fn from_json<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("invalid built-in fixture: {error}"))
}
