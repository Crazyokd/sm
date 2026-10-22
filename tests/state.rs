use serde_json::{Value, json};
use sm::{
    model::{OwnedWrite, WriteBody},
    state::State,
};
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn ownership_persists_typed_bodies_and_reopens_with_private_permissions() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite3");
    let state = State::open(path.clone()).unwrap();
    let bodies = [
        WriteBody::File {
            bytes: vec![0, 255, 128, 10],
            executable: true,
        },
        WriteBody::Markdown("工作约定\n".as_bytes().to_vec()),
        WriteBody::Json(json!({"hooks":{"Stop":[]}}).as_object().unwrap().clone()),
        WriteBody::Toml(
            json!({"mcp_servers":{"docs":{"command":"docs"}}})
                .as_object()
                .unwrap()
                .clone(),
        ),
    ];
    let mut expected = Vec::new();
    for (index, body) in bodies.into_iter().enumerate() {
        let target = root.path().join(index.to_string());
        let write = OwnedWrite::new("resource", &target, body);
        state
            .replace_owned(&target, std::slice::from_ref(&write))
            .unwrap();
        expected.push(write);
    }
    let saved: String = state
        .connect()
        .unwrap()
        .query_row("SELECT value FROM ownership LIMIT 1", [], |row| row.get(0))
        .unwrap();
    let saved: Value = serde_json::from_str(&saved).unwrap();
    assert!(
        saved.get("body").is_some(),
        "persist the domain body directly"
    );
    assert_eq!(saved.as_object().unwrap().len(), 3);
    drop(state);
    assert_eq!(
        State::open(path.clone()).unwrap().owned().unwrap(),
        expected
    );
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn malformed_state_fails_visibly_without_clearing_saved_data() {
    let root = tempfile::tempdir().unwrap();
    let state = State::open(root.path().join("state.sqlite3")).unwrap();
    let target = root.path().join("file");
    let write = OwnedWrite::new(
        "resource",
        &target,
        WriteBody::File {
            bytes: b"managed".to_vec(),
            executable: false,
        },
    );
    state.replace_owned(&target, &[write]).unwrap();
    let db = state.connect().unwrap();
    db.execute("UPDATE ownership SET value='broken'", [])
        .unwrap();
    assert!(state.owned().is_err());
    let saved: String = db
        .query_row("SELECT value FROM ownership", [], |row| row.get(0))
        .unwrap();
    assert_eq!(saved, "broken");
    db.execute("INSERT INTO kv VALUES ('catalog','broken')", [])
        .unwrap();
    assert!(state.get::<sm::model::Catalog>("catalog").is_err());
}
