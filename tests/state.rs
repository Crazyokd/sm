use rusqlite::{Connection, params};
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
        expected.push(serde_json::to_value(write).unwrap());
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
    let saved = State::open(path.clone()).unwrap().owned().unwrap();
    let saved: Vec<Value> = saved
        .into_iter()
        .map(|write| serde_json::to_value(write).unwrap())
        .collect();
    assert_eq!(saved[0]["body"]["kind"], "file_fingerprint");
    assert!(saved[0]["body"]["content"].get("bytes").is_none());
    assert_eq!(saved[0]["body"]["content"]["executable"], true);
    assert_eq!(
        saved[0]["body"]["content"]["sha256"]
            .as_array()
            .unwrap()
            .len(),
        32
    );
    assert_eq!(&saved[1..], &expected[1..]);
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

fn legacy_database(path: &std::path::Path, writes: &[OwnedWrite]) -> Connection {
    let db = Connection::open(path).unwrap();
    db.execute_batch(
        "CREATE TABLE ownership (resource TEXT,path TEXT,value TEXT,PRIMARY KEY(resource,path));",
    )
    .unwrap();
    for write in writes {
        db.execute(
            "INSERT INTO ownership VALUES (?1,?2,?3)",
            params![
                write.resource_id,
                write.path.to_string_lossy(),
                serde_json::to_string(write).unwrap()
            ],
        )
        .unwrap();
    }
    db
}

#[test]
fn legacy_ownership_migrates_once_and_compacts_without_touching_targets() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite3");
    let target = root.path().join("file-0");
    fs::write(&target, "local content must survive migration").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o751)).unwrap();
    let mut writes: Vec<_> = (0..8)
        .map(|index| {
            OwnedWrite::new(
                format!("resource-{index}"),
                root.path().join(format!("file-{index}")),
                WriteBody::File {
                    bytes: vec![index; 256 * 1024],
                    executable: index % 2 == 0,
                },
            )
        })
        .collect();
    writes.push(OwnedWrite::new(
        "rule",
        root.path().join("AGENTS.md"),
        WriteBody::Markdown(b"managed rules".to_vec()),
    ));
    writes.push(OwnedWrite::new(
        "json",
        root.path().join("config.json"),
        WriteBody::Json(json!({"hooks":{"Stop":[]}}).as_object().unwrap().clone()),
    ));
    writes.push(OwnedWrite::new(
        "toml",
        root.path().join("config.toml"),
        WriteBody::Toml(
            json!({"mcp_servers":{"docs":{"command":"docs"}}})
                .as_object()
                .unwrap()
                .clone(),
        ),
    ));
    drop(legacy_database(&path, &writes));
    let before = fs::metadata(&path).unwrap().len();

    let state = State::open(path.clone()).unwrap();
    let saved = state.owned().unwrap();
    assert_eq!(saved.len(), writes.len());
    for write in &saved {
        let encoded = serde_json::to_value(write).unwrap();
        if write.resource_id.starts_with("resource-") {
            assert_eq!(encoded["body"]["kind"], "file_fingerprint");
            assert!(encoded["body"]["content"].get("bytes").is_none());
        } else {
            let original = writes
                .iter()
                .find(|old| old.resource_id == write.resource_id)
                .unwrap();
            assert_eq!(encoded, serde_json::to_value(original).unwrap());
        }
    }
    assert!(fs::metadata(&path).unwrap().len() < before / 10);
    let db = state.connect().unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let encoded: Vec<String> = db
        .prepare("SELECT value FROM ownership ORDER BY resource,path")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    drop(db);
    drop(state);
    let state = State::open(path).unwrap();
    let reopened: Vec<String> = state
        .connect()
        .unwrap()
        .prepare("SELECT value FROM ownership ORDER BY resource,path")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(encoded, reopened);
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "local content must survive migration"
    );
    assert_eq!(
        fs::metadata(target).unwrap().permissions().mode() & 0o777,
        0o751
    );
}

#[test]
fn malformed_legacy_ownership_rolls_back_every_conversion() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("state.sqlite3");
    let write = OwnedWrite::new(
        "a-valid",
        root.path().join("file"),
        WriteBody::File {
            bytes: vec![7; 8192],
            executable: true,
        },
    );
    let original = serde_json::to_string(&write).unwrap();
    let db = legacy_database(&path, &[write]);
    db.execute(
        "INSERT INTO ownership VALUES ('z-invalid','invalid','{\"body\":\"malformed\"}')",
        [],
    )
    .unwrap();
    assert!(State::open(path).is_err());
    assert_eq!(
        db.query_row(
            "SELECT value FROM ownership WHERE resource='a-valid'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        original
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM ownership", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
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

#[test]
fn concurrent_run_appends_preserve_both_records_and_recent_limit() {
    struct ConcurrentResult<'a>(&'a std::sync::Barrier);
    impl serde::Serialize for ConcurrentResult<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            self.0.wait();
            serializer.serialize_bool(true)
        }
    }

    let root = tempfile::tempdir().unwrap();
    let state = State::open(root.path().join("state.sqlite3")).unwrap();
    let initial: Vec<_> = (0..99)
        .map(|index| sm::state::RunRecord {
            operation: "old".into(),
            time: "2026-10-22T00:00:00Z".into(),
            result: json!(index),
        })
        .collect();
    state.set("runs", &initial).unwrap();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let first = scope.spawn(|| state.add_run("first", &ConcurrentResult(&barrier)).unwrap());
        let second = scope.spawn(|| {
            state
                .add_run("second", &ConcurrentResult(&barrier))
                .unwrap()
        });
        first.join().unwrap();
        second.join().unwrap();
    });
    let records: Vec<sm::state::RunRecord> = state.get("runs").unwrap();
    assert_eq!(records.len(), 100);
    assert!(records.iter().any(|record| record.operation == "first"));
    assert!(records.iter().any(|record| record.operation == "second"));
    assert_eq!(records[0].result, 1);
}
