use serde_json::json;
use sm::{aliases, model::ApplyStatus, state::State};
use std::{fs, os::unix::fs::symlink, path::PathBuf};

struct Fixture {
    _temp: tempfile::TempDir,
    source: PathBuf,
    home: PathBuf,
    state: State,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let home = temp.path().join("home");
        fs::create_dir_all(source.join("resources/native-installations")).unwrap();
        fs::create_dir_all(home.join(".claude/skills")).unwrap();
        let state = State::open(temp.path().join("state/state.db")).unwrap();
        Self {
            _temp: temp,
            source,
            home,
            state,
        }
    }

    fn manifest(&self, path: &str, previous: &str) {
        fs::write(
            self.source
                .join("resources/native-installations/retired-aliases.json"),
            serde_json::to_vec(&json!([{"path":path,"previous":previous}])).unwrap(),
        )
        .unwrap();
    }

    fn status(&self) -> ApplyStatus {
        aliases::synchronize(&self.source, &self.home, &self.state).unwrap()["@alias/retired"]
            .status
    }

    fn alias(&self) -> PathBuf {
        self.home.join(".claude/skills/retired")
    }
}

#[test]
fn only_exact_reviewed_dangling_alias_is_retired_and_repeated_runs_are_safe() {
    let fixture = Fixture::new();
    fixture.manifest(".claude/skills/retired", "../../missing");
    symlink("../../missing", fixture.alias()).unwrap();
    assert_eq!(fixture.status(), ApplyStatus::Applied);
    assert!(fs::symlink_metadata(fixture.alias()).is_err());
    assert_eq!(fixture.status(), ApplyStatus::Applied);
    assert!(fixture.state.owned().unwrap().is_empty());
}

#[test]
fn changed_link_regular_file_and_now_valid_old_target_are_preserved() {
    let fixture = Fixture::new();
    fixture.manifest(".claude/skills/retired", "../../missing");
    symlink("../../different", fixture.alias()).unwrap();
    assert_eq!(fixture.status(), ApplyStatus::Conflict);
    assert_eq!(
        fs::read_link(fixture.alias()).unwrap(),
        PathBuf::from("../../different")
    );
    fs::remove_file(fixture.alias()).unwrap();
    fs::write(fixture.alias(), "local file").unwrap();
    assert_eq!(fixture.status(), ApplyStatus::Conflict);
    assert_eq!(fs::read_to_string(fixture.alias()).unwrap(), "local file");
    fs::remove_file(fixture.alias()).unwrap();
    fs::create_dir(fixture.home.join("missing")).unwrap();
    symlink("../../missing", fixture.alias()).unwrap();
    assert_eq!(fixture.status(), ApplyStatus::Conflict);
    assert!(fixture.alias().is_symlink() && fixture.alias().exists());
}

#[test]
fn symlinked_alias_parent_is_never_followed() {
    let fixture = Fixture::new();
    fixture.manifest(".claude/skills/retired", "missing");
    let outside = fixture.home.join("outside");
    fs::create_dir(&outside).unwrap();
    symlink("missing", outside.join("retired")).unwrap();
    fs::remove_dir(fixture.home.join(".claude/skills")).unwrap();
    symlink(&outside, fixture.home.join(".claude/skills")).unwrap();
    assert_eq!(fixture.status(), ApplyStatus::Conflict);
    assert!(outside.join("retired").is_symlink());
}

#[test]
fn invalid_paths_fail_before_any_alias_is_removed() {
    let fixture = Fixture::new();
    symlink("missing", fixture.alias()).unwrap();
    for path in [
        "../retired",
        "/.claude/skills/retired",
        ".claude/skills/../retired",
        ".claude/skills/nested/retired",
        ".agents/skills/retired",
        ".claude/skills/INVALID",
    ] {
        fs::write(
            fixture
                .source
                .join("resources/native-installations/retired-aliases.json"),
            serde_json::to_vec(&json!([
                {"path":".claude/skills/retired","previous":"missing"},
                {"path":path,"previous":"missing"}
            ]))
            .unwrap(),
        )
        .unwrap();
        assert!(
            aliases::synchronize(&fixture.source, &fixture.home, &fixture.state).is_err(),
            "{path}"
        );
        assert!(fixture.alias().is_symlink());
    }
}

#[test]
fn absent_manifest_is_an_empty_success_and_parse_errors_do_not_expose_values() {
    let fixture = Fixture::new();
    assert!(
        aliases::synchronize(&fixture.source, &fixture.home, &fixture.state)
            .unwrap()
            .is_empty()
    );
    fs::write(
        fixture
            .source
            .join("resources/native-installations/retired-aliases.json"),
        r#"[{"path":"private-token-value","previous":"missing","extra":true}]"#,
    )
    .unwrap();
    let error = aliases::synchronize(&fixture.source, &fixture.home, &fixture.state)
        .unwrap_err()
        .to_string();
    assert_eq!(error, "invalid retired alias manifest");
}
