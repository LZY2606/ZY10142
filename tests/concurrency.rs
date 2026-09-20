use pair_wise_gsb::db;
use pair_wise_gsb::service::App;
use pair_wise_gsb::types::*;
use rusqlite::Connection;
use serde_json::json;
use std::sync::Barrier;

fn from_json<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> T {
    serde_json::from_value(value).unwrap()
}

#[test]
fn concurrent_publish_only_one_disturbance_owns_record() {
    let path = unique_db_path();
    let mut setup = App::new(db::open(&path).unwrap());
    let imported = setup
        .import_records(from_json(json!({
            "records": [{"deviceId":"A","localTimeNs":0,"events":[
                {"key":"start","kind":"feature","localTimeNs":100,"uncertaintyNs":0},
                {"key":"trip","kind":"trip","localTimeNs":200,"uncertaintyNs":0}
            ]}]
        })))
        .unwrap();
    let record_id = imported.imported[0].record_id.clone();
    let case_one = setup
        .create_case(from_json(
            json!({"name":"one","description":"","recordIds":[record_id]}),
        ))
        .unwrap();
    let case_two = setup
        .create_case(from_json(
            json!({"name":"two","description":"","recordIds":[record_id]}),
        ))
        .unwrap();
    setup
        .save_anchor(SaveAnchorRequest {
            anchor_key: "start".into(),
            label: "start".into(),
            corrected_time_ns: Some(100),
            uncertainty_ns: 0,
            trusted: true,
            event_refs: vec![AnchorEventRef {
                record_id: record_id.clone(),
                event_key: "start".into(),
            }],
        })
        .unwrap();
    setup
        .save_anchor(SaveAnchorRequest {
            anchor_key: "trip".into(),
            label: "trip".into(),
            corrected_time_ns: Some(200),
            uncertainty_ns: 0,
            trusted: true,
            event_refs: vec![AnchorEventRef {
                record_id: record_id.clone(),
                event_key: "trip".into(),
            }],
        })
        .unwrap();
    let version_one = setup
        .create_version(case_one, from_json(json!({"note":"one"})))
        .unwrap();
    let version_two = setup
        .create_version(case_two, from_json(json!({"note":"two"})))
        .unwrap();

    drop(setup.into_connection());

    let barrier = std::sync::Arc::new(Barrier::new(2));
    let left_barrier = barrier.clone();
    let right_barrier = barrier.clone();
    let left_path = path.clone();
    let right_path = path.clone();
    let left = std::thread::spawn(move || publish_once(left_path, version_one, left_barrier));
    let right = std::thread::spawn(move || publish_once(right_path, version_two, right_barrier));
    let left_result = left.join().unwrap();
    let right_result = right.join().unwrap();

    let results = [left_result.is_ok(), right_result.is_ok()];
    assert_eq!(results.iter().filter(|ok| **ok).count(), 1);
    let check = Connection::open(&path).unwrap();
    let owner_count = check
        .query_row(
            "SELECT COUNT(*), COUNT(DISTINCT case_id) FROM published_membership WHERE record_id = ?1",
            [&record_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap();
    assert_eq!(owner_count, (1, 1));
}

fn unique_db_path() -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "pair-wise-gsb-concurrency-{}-{nanos}.sqlite",
        std::process::id()
    ))
}

fn publish_once(
    path: std::path::PathBuf,
    version_id: i64,
    barrier: std::sync::Arc<Barrier>,
) -> Result<(), String> {
    let mut app = App::new(db::open(&path).unwrap());
    barrier.wait();
    app.publish_version(version_id)
}
