use serde_json::{Value, json};
use sm::ecc::{build_output, render_agent, render_command, unpack_package};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};
use tempfile::tempdir;

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn selection(path: &Path, value: Value) {
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

#[test]
fn agent_conversion_preserves_instructions_and_escapes_native_toml() {
    let body = "# Instructions\n\nKeep `quoted \"values\"`, \\\\paths, and \"\"\" blocks.\n";
    let text = format!(
        "---\nname: reviewer\ndescription: 'Review a user''s change'\nmodel: opus\ntools: Read, Bash\n---\n\n{body}"
    );
    let native = render_agent(&text).unwrap();
    let parsed: Value = toml_edit::de::from_str(&native).unwrap();
    assert_eq!(parsed["name"], "reviewer");
    assert_eq!(parsed["description"], "Review a user's change");
    assert_eq!(parsed["developer_instructions"], body);
    assert!(parsed.get("model").is_none());
    assert!(parsed.get("tools").is_none());
}

#[test]
fn command_conversion_keeps_the_command_body_as_a_skill_template() {
    let body = "# Plan Command\n\nDo not execute this fixture.\n";
    let text = format!(
        "---\r\ndescription: >-\r\n  Plan a change with\r\n  clear checks.\r\nargument-hint: '[feature]'\r\n---\r\n\r\n{body}"
    );
    let skill = render_command("plan", &text).unwrap();
    assert!(skill.starts_with("---\nname: \"source-command-plan\"\n"));
    assert!(skill.contains("description: \"Plan a change with clear checks.\""));
    assert!(skill.ends_with(&format!("## Command Template\n\n{body}")));
    assert!(!skill.contains("argument-hint"));
    assert!(render_command("../plan", &text).is_err());
    assert!(render_command("plan", "# Missing frontmatter").is_err());
    assert!(render_agent("---\nname: reviewer\n---\nBody").is_err());
}

#[test]
fn local_output_includes_only_selected_native_skills_commands_agents_and_mirrors() {
    let temp = tempdir().unwrap();
    let upstream = temp.path().join("upstream");
    let installed = temp.path().join("installed");
    let output = temp.path().join("output");
    let selected = temp.path().join("selection.json");
    write(
        &upstream.join("skills/api/SKILL.md"),
        "---\nname: api\n---\nUpstream source",
    );
    write(
        &upstream.join("skills/unselected/SKILL.md"),
        "Unselected source",
    );
    write(
        &upstream.join("commands/plan.md"),
        "---\ndescription: Plan a change\n---\n# Plan\n",
    );
    write(
        &upstream.join("agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Review a change\n---\n# Review\n",
    );
    write(&upstream.join("hooks/run.sh"), "Never run");
    write(
        &installed.join(".agents/skills/api/SKILL.md"),
        "Native installed copy\n",
    );
    write(
        &installed.join(".agents/skills/api/scripts/run.sh"),
        "#!/bin/sh\nexit 0\n",
    );
    fs::set_permissions(
        installed.join(".agents/skills/api/scripts/run.sh"),
        fs::Permissions::from_mode(0o750),
    )
    .unwrap();
    write(
        &installed.join(".agents/skills/unselected/SKILL.md"),
        "Unselected installed copy",
    );
    write(&installed.join(".codex/config.toml"), "Never import config");
    selection(
        &selected,
        json!({"skills":["api"], "commands":["plan"], "agents":["reviewer"], "claude_skills":["api", "source-command-plan"]}),
    );
    build_output(&selected, &upstream, &installed, &output).unwrap();
    assert_eq!(
        fs::read(output.join(".agents/skills/api/SKILL.md")).unwrap(),
        b"Native installed copy\n"
    );
    assert!(!output.join(".agents/skills/unselected").exists());
    assert!(!output.join(".codex/config.toml").exists());
    assert!(!output.join("hooks/run.sh").exists());
    assert!(!output.join("bundle.json").exists());
    assert_eq!(sm::files::payload_files(&output).unwrap().len(), 7);
    let script = output.join(".claude/skills/api/scripts/run.sh");
    assert_ne!(
        fs::metadata(&script).unwrap().permissions().mode() & 0o111,
        0
    );
    assert_eq!(fs::read(script).unwrap(), b"#!/bin/sh\nexit 0\n");
    assert!(output.join(".codex/agents/reviewer.toml").is_file());
    assert!(
        output
            .join(".claude/skills/source-command-plan/SKILL.md")
            .is_file()
    );
}

#[test]
fn removed_members_are_absent_while_existing_skills_commands_agents_and_files_generate() {
    let temp = tempdir().unwrap();
    let upstream = temp.path().join("upstream");
    let installed = temp.path().join("installed");
    let output = temp.path().join("output");
    let selected = temp.path().join("selection.json");
    write(&upstream.join("skills/api/SKILL.md"), "Skill");
    write(
        &installed.join(".agents/skills/api/SKILL.md"),
        "Installed Skill",
    );
    write(
        &upstream.join("commands/plan.md"),
        "---\ndescription: Plan\n---\nBody\n",
    );
    write(
        &upstream.join("agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Review\n---\nBody\n",
    );
    write(&upstream.join("rules/style.md"), "Style");
    selection(
        &selected,
        json!({
            "skills":["api","removed"], "commands":["plan","removed"], "agents":["reviewer","removed"],
            "claude_skills":["api","removed","source-command-plan","source-command-removed"],
            "files":[{"source":"rules/style.md","target":".claude/rules/style.md"},{"source":"rules/removed.md","target":".claude/rules/removed.md"}]
        }),
    );
    build_output(&selected, &upstream, &installed, &output).unwrap();
    for target in [
        ".agents/skills/api/SKILL.md",
        ".agents/skills/source-command-plan/SKILL.md",
        ".codex/agents/reviewer.toml",
        ".claude/rules/style.md",
        ".claude/skills/api/SKILL.md",
    ] {
        assert!(output.join(target).is_file(), "{target}");
    }
    for target in [
        ".agents/skills/removed",
        ".agents/skills/source-command-removed",
        ".codex/agents/removed.toml",
        ".claude/rules/removed.md",
        ".claude/skills/removed",
        "bundle.json",
    ] {
        assert!(!output.join(target).exists(), "{target}");
    }
}

#[test]
fn invalid_selections_and_symlinks_are_rejected_before_output() {
    let temp = tempdir().unwrap();
    let upstream = temp.path().join("upstream");
    let installed = temp.path().join("installed");
    let output = temp.path().join("output");
    let selected = temp.path().join("selection.json");
    for value in [
        json!({"skills":["../escape"], "commands":[], "agents":[]}),
        json!({"skills":["api","api"], "commands":[], "agents":[]}),
        json!({"skills":[], "commands":[], "agents":[], "claude_skills":["unselected"]}),
        json!({"skills":["source-command-plan"], "commands":["plan"], "agents":[]}),
        json!({"skills":[], "commands":[], "agents":[], "unknown":true}),
    ] {
        selection(&selected, value);
        assert!(build_output(&selected, &upstream, &installed, &output).is_err());
        assert!(!output.exists());
    }
    write(&upstream.join("skills/api/SKILL.md"), "Skill");
    write(&installed.join(".agents/skills/api/SKILL.md"), "Installed");
    symlink("SKILL.md", installed.join(".agents/skills/api/alias")).unwrap();
    selection(
        &selected,
        json!({"skills":["api"], "commands":[], "agents":[]}),
    );
    assert!(build_output(&selected, &upstream, &installed, &output).is_err());
    assert!(!output.exists());
    selection(
        &selected,
        json!({"skills":[], "commands":["plan"], "agents":[]}),
    );
    fs::create_dir_all(upstream.join("commands")).unwrap();
    symlink(
        installed.join(".agents/skills/api/SKILL.md"),
        upstream.join("commands/plan.md"),
    )
    .unwrap();
    assert!(build_output(&selected, &upstream, &installed, &output).is_err());
    assert!(!output.exists());
}

#[test]
fn selected_raw_files_keep_exact_reviewed_targets_bytes_and_executable_bits() {
    let temp = tempdir().unwrap();
    let upstream = temp.path().join("upstream");
    let output = temp.path().join("output");
    let selected = temp.path().join("selection.json");
    write(
        &upstream.join("agents/reviewer.md"),
        "Raw agent definition\n",
    );
    write(&upstream.join("rules/common/style.md"), "Shared rule\n");
    let script = upstream.join("scripts/hooks/check.sh");
    write(&script, "#!/bin/sh\nexit 0\n");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o750)).unwrap();
    write(
        &upstream.join("plugins/unselected/data.json"),
        "Unselected plugin cache",
    );
    selection(
        &selected,
        json!({
            "skills":[], "commands":[], "agents":[],
            "files":[
                {"source":"agents/reviewer.md", "target":".claude/agents/reviewer.md"},
                {"source":"rules/common/style.md", "target":".claude/rules/common/style.md"},
                {"source":"scripts/hooks/check.sh", "target":".claude/hooks/check.sh"}
            ]
        }),
    );
    build_output(
        &selected,
        &upstream,
        &temp.path().join("installed"),
        &output,
    )
    .unwrap();
    assert!(!output.join("bundle.json").exists());
    assert_eq!(
        fs::read(output.join(".claude/agents/reviewer.md")).unwrap(),
        b"Raw agent definition\n"
    );
    assert_eq!(
        fs::read(output.join(".claude/rules/common/style.md")).unwrap(),
        b"Shared rule\n"
    );
    assert_eq!(
        fs::read(output.join(".claude/hooks/check.sh")).unwrap(),
        b"#!/bin/sh\nexit 0\n"
    );
    assert_ne!(
        fs::metadata(output.join(".claude/hooks/check.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert_eq!(
        fs::metadata(output.join(".claude/rules/common/style.md"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert!(!output.join("plugins").exists());
    assert!(!output.join("agents").exists());
}

#[test]
fn selected_raw_file_errors_preserve_the_previous_output() {
    let temp = tempdir().unwrap();
    let upstream = temp.path().join("upstream");
    let output = temp.path().join("output");
    let selected = temp.path().join("selection.json");
    write(&upstream.join("rules/valid.md"), "Rule");
    write(&output.join("bundle.json"), "Previous approved bundle");
    symlink("valid.md", upstream.join("rules/link.md")).unwrap();
    symlink("rules", upstream.join("rule-alias")).unwrap();
    nix::unistd::mkfifo(
        &upstream.join("rules/fifo"),
        nix::sys::stat::Mode::from_bits_truncate(0o600),
    )
    .unwrap();
    for files in [
        json!([{"source":"../outside", "target":".claude/rules/rule.md"}]),
        json!([{"source":"/etc/passwd", "target":".claude/rules/rule.md"}]),
        json!([{"source":"rules/valid.md", "target":"../outside"}]),
        json!([{"source":"rules/valid.md", "target":"/absolute"}]),
        json!([{"source":"rules/link.md", "target":".claude/rules/rule.md"}]),
        json!([{"source":"rule-alias/valid.md", "target":".claude/rules/rule.md"}]),
        json!([{"source":"rules/fifo", "target":".claude/rules/rule.md"}]),
        json!([{"source":"rules", "target":".claude/rules/rule.md"}]),
        json!([{"source":"rules/valid.md", "target":".claude/rules/rule.md", "unknown":true}]),
        json!([{"source":"rules/valid.md", "target":".claude/rules/rule.md"}, {"source":"rules/valid.md", "target":".claude/rules/rule.md"}]),
    ] {
        selection(
            &selected,
            json!({"skills":[], "commands":[], "agents":[], "files":files}),
        );
        assert!(
            build_output(
                &selected,
                &upstream,
                &temp.path().join("installed"),
                &output
            )
            .is_err()
        );
        assert_eq!(
            fs::read(output.join("bundle.json")).unwrap(),
            b"Previous approved bundle"
        );
        assert!(!output.join("extra").exists());
    }
}

#[test]
fn selected_raw_file_targets_cannot_replace_generated_skills_agents_or_mirrors() {
    let temp = tempdir().unwrap();
    let upstream = temp.path().join("upstream");
    let installed = temp.path().join("installed");
    let output = temp.path().join("output");
    let selected = temp.path().join("selection.json");
    write(&upstream.join("skills/api/SKILL.md"), "Skill");
    write(
        &installed.join(".agents/skills/api/SKILL.md"),
        "Installed skill",
    );
    write(
        &upstream.join("agents/reviewer.md"),
        "---\nname: reviewer\ndescription: Review a change\n---\nBody\n",
    );
    write(
        &upstream.join("commands/plan.md"),
        "---\ndescription: Plan a change\n---\nBody\n",
    );
    write(&upstream.join("rules/rule.md"), "Rule");
    for target in [
        ".agents/skills/api/SKILL.md",
        ".agents/skills/source-command-plan/SKILL.md",
        ".codex/agents/reviewer.toml",
        ".claude/skills/api/SKILL.md",
    ] {
        selection(
            &selected,
            json!({"skills":["api"], "commands":["plan"], "agents":["reviewer"], "claude_skills":["api"], "files":[{"source":"rules/rule.md", "target":target}]}),
        );
        let error = build_output(&selected, &upstream, &installed, &output).unwrap_err();
        assert!(error.to_string().contains("target overlaps"), "{error:#}");
        assert!(!output.exists());
    }
}

fn archive(path: &str, kind: tar::EntryType) -> Vec<u8> {
    let mut header = tar::Header::new_gnu();
    header.set_mode(0o750);
    header.set_entry_type(kind);
    let data = if kind.is_file() {
        b"data".as_slice()
    } else {
        b"".as_slice()
    };
    header.set_size(data.len() as u64);
    header.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
    if kind.is_symlink() || kind.is_hard_link() {
        header.set_link_name("../../outside").unwrap();
    }
    header.set_cksum();
    let mut builder = tar::Builder::new(Vec::new());
    builder.append(&header, data).unwrap();
    builder.into_inner().unwrap()
}

#[test]
fn published_package_extraction_rejects_escape_paths_links_and_special_files() {
    let temp = tempdir().unwrap();
    let output = temp.path().join("output");
    unpack_package(
        &archive("package/skills/api/run.sh", tar::EntryType::Regular),
        &output,
    )
    .unwrap();
    assert_eq!(fs::read(output.join("skills/api/run.sh")).unwrap(), b"data");
    assert_ne!(
        fs::metadata(output.join("skills/api/run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    for (path, kind) in [
        ("package/../outside", tar::EntryType::Regular),
        ("/outside", tar::EntryType::Regular),
        ("other/file", tar::EntryType::Regular),
        ("package/link", tar::EntryType::Symlink),
        ("package/link", tar::EntryType::Link),
        ("package/fifo", tar::EntryType::Fifo),
    ] {
        assert!(
            unpack_package(&archive(path, kind), &output).is_err(),
            "{path}/{kind:?}"
        );
    }
    assert!(!temp.path().join("outside").exists());
    symlink(&output, temp.path().join("alias")).unwrap();
    assert!(
        unpack_package(
            &archive("package/file", tar::EntryType::Regular),
            &temp.path().join("alias")
        )
        .is_err()
    );
}
