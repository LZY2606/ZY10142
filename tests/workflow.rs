use pair_wise_gsb::db;
use pair_wise_gsb::service::App;
use pair_wise_gsb::types::*;
use serde_json::json;

fn app() -> App {
    App::new(db::open_in_memory().unwrap())
}

fn from_json<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> T {
    serde_json::from_value(value).unwrap()
}

fn import_two_agents(app: &mut App) -> Vec<String> {
    let response = app
        .import_records(from_json(json!({
            "records": [
                {"deviceId":"A","localTime":"1970-01-01T00:00:00Z","events":[
                    {"key":"start","kind":"feature","localTimeNs":100,"uncertaintyNs":2},
                    {"key":"trip","kind":"trip","localTimeNs":200,"uncertaintyNs":2}
                ]},
                {"deviceId":"B","localTime":"1970-01-01T00:00:00.000000000Z","events":[
                    {"key":"start","kind":"feature","localTimeNs":50,"uncertaintyNs":2},
                    {"key":"jump_trip","kind":"trip","localTimeNs":100,"uncertaintyNs":2},
                    {"key":"reclose","kind":"reclose","localTimeNs":200,"uncertaintyNs":2}
                ]}
            ]
        })))
        .unwrap();
    assert_eq!(response.imported.len(), 2);
    response
        .imported
        .into_iter()
        .map(|record| record.record_id)
        .collect()
}

#[test]
fn device_and_content_fingerprint_deduplicates_imports() {
    let mut app = app();
    let first = import_two_agents(&mut app);
    let response = app
        .import_records(from_json(json!({
            "records": [
                {"deviceId":"A","localTimeNs":0,"events":[
                    {"key":"start","kind":"feature","localTimeNs":100,"uncertaintyNs":2},
                    {"key":"trip","kind":"trip","localTimeNs":200,"uncertaintyNs":2}
                ]},
                {"deviceId":"B","localTimeNs":0,"events":[
                    {"key":"start","kind":"feature","localTimeNs":50,"uncertaintyNs":2},
                    {"key":"jump_trip","kind":"trip","localTimeNs":100,"uncertaintyNs":2},
                    {"key":"reclose","kind":"reclose","localTimeNs":200,"uncertaintyNs":2}
                ]}
            ]
        })))
        .unwrap();
    assert!(response.imported.is_empty());
    let second: Vec<String> = response
        .skipped_duplicates
        .into_iter()
        .map(|record| record.record_id)
        .collect();
    assert_eq!(first, second);
    let state = app.state().unwrap();
    assert_eq!(state.records.len(), 2);
    assert_eq!(state.events.len(), 5);
}

#[test]
fn publishes_alignment_and_membership_together_and_rejects_second_case() {
    let mut app = app();
    let records = import_two_agents(&mut app);
    let first_case = app
        .create_case(from_json(
            json!({"name":"first","description":"","recordIds":records}),
        ))
        .unwrap();
    let second_case = app
        .create_case(from_json(
            json!({"name":"second","description":"","recordIds":[records[0]]}),
        ))
        .unwrap();
    save_anchor(
        &mut app,
        "start",
        100,
        4,
        &[(&records[0], "start"), (&records[1], "start")],
    );
    save_anchor(
        &mut app,
        "trip",
        200,
        4,
        &[(&records[0], "trip"), (&records[1], "jump_trip")],
    );
    save_anchor(&mut app, "reclose", 300, 2, &[(&records[1], "reclose")]);

    let first_version = app
        .create_version(
            first_case,
            from_json(json!({"note":"v1","jumpsByDevice":{"B":[100]}})),
        )
        .unwrap();
    app.publish_version(first_version).unwrap();
    let second_version = app
        .create_version(second_case, from_json(json!({"note":"v1"})))
        .unwrap();
    let conflict = app.publish_version(second_version).unwrap_err();
    assert!(conflict.contains("another published disturbance"));

    let state = app.state().unwrap();
    let second = state
        .cases
        .iter()
        .find(|case| case.case_id == second_case)
        .unwrap();
    assert_eq!(second.published_record_ids.len(), 0);
    assert!(!second.versions[0].published);
}

#[test]
fn old_version_and_review_remain_replayable_after_new_anchor_model() {
    let mut app = app();
    let records = import_two_agents(&mut app);
    let case_id = app
        .create_case(from_json(
            json!({"name":"case","description":"","recordIds":records}),
        ))
        .unwrap();
    save_anchor(
        &mut app,
        "start",
        100,
        4,
        &[(&records[0], "start"), (&records[1], "start")],
    );
    save_anchor(
        &mut app,
        "trip",
        200,
        4,
        &[(&records[0], "trip"), (&records[1], "jump_trip")],
    );
    save_anchor(&mut app, "reclose", 300, 2, &[(&records[1], "reclose")]);

    let v1 = app
        .create_version(
            case_id,
            from_json(json!({"note":"v1","jumpsByDevice":{"B":[100]}})),
        )
        .unwrap();
    app.add_review(
        v1,
        from_json(json!({"conclusion":"initial replayable conclusion"})),
    )
    .unwrap();
    app.publish_version(v1).unwrap();

    app.save_anchor(from_json(json!({
        "anchorKey":"trip",
        "label":"trip updated",
        "correctedTimeNs": 201,
        "uncertaintyNs": 4,
        "trusted": true,
        "eventRefs": [
            {"recordId": records[0], "eventKey":"trip"},
            {"recordId": records[1], "eventKey":"jump_trip"}
        ]
    })))
    .unwrap();
    let v2 = app
        .create_version(
            case_id,
            from_json(json!({"note":"v2","jumpsByDevice":{"B":[100]}})),
        )
        .unwrap();
    assert_ne!(v1, v2);

    let state = app.state().unwrap();
    let case = state
        .cases
        .into_iter()
        .find(|case| case.case_id == case_id)
        .unwrap();
    assert_eq!(case.versions.len(), 2);
    assert_eq!(
        case.versions[0].reviews[0].conclusion,
        "initial replayable conclusion"
    );
    assert_eq!(
        case.versions[0].timeline[0].low_ns,
        case.versions[0].timeline[0].low_ns
    );
    assert!(case.versions[0].published);
    assert!(!case.versions[1].published);
}

#[test]
fn overlapping_uncertainty_is_incomparable_without_device_tie_break() {
    let mut app = app();
    let records = import_two_agents(&mut app);
    let case_id = app
        .create_case(from_json(
            json!({"name":"case","description":"","recordIds":records}),
        ))
        .unwrap();
    save_anchor(
        &mut app,
        "start",
        100,
        10,
        &[(&records[0], "start"), (&records[1], "start")],
    );
    save_anchor(
        &mut app,
        "trip",
        200,
        0,
        &[(&records[0], "trip"), (&records[1], "jump_trip")],
    );
    save_anchor(&mut app, "reclose", 300, 0, &[(&records[1], "reclose")]);
    let version_id = app
        .create_version(
            case_id,
            from_json(json!({"note":"wide uncertainty","jumpsByDevice":{"B":[100]}})),
        )
        .unwrap();
    let state = app.state().unwrap();
    let case = state
        .cases
        .iter()
        .find(|case| case.case_id == case_id)
        .unwrap();
    let version = case
        .versions
        .iter()
        .find(|version| version.version_id == version_id)
        .unwrap();
    let relations = &version.pair_relations;
    assert!(relations.iter().any(|pair| pair.relation == "incomparable"));
    assert!(relations.iter().any(|pair| pair.relation == "before"));
}

fn save_anchor(
    app: &mut App,
    key: &str,
    corrected_time_ns: i64,
    uncertainty_ns: i64,
    refs: &[(&str, &str)],
) {
    app.save_anchor(SaveAnchorRequest {
        anchor_key: key.into(),
        label: key.into(),
        corrected_time_ns: Some(corrected_time_ns),
        uncertainty_ns,
        trusted: true,
        event_refs: refs
            .iter()
            .map(|(record_id, event_key)| AnchorEventRef {
                record_id: (*record_id).into(),
                event_key: (*event_key).into(),
            })
            .collect(),
    })
    .unwrap();
}
