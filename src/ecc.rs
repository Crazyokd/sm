use crate::{
    files::{atomic_write, contained, payload_files, reject_symlink_path, relative_path},
    model::valid_id,
    process::run_command,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    skills: Vec<String>,
    commands: Vec<String>,
    agents: Vec<String>,
    #[serde(default)]
    claude_skills: Vec<String>,
    #[serde(default)]
    files: Vec<SelectedFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectedFile {
    source: PathBuf,
    target: PathBuf,
}

fn selection(path: &Path) -> Result<Selection> {
    let selection: Selection = serde_json::from_slice(&fs::read(path)?)?;
    for names in [
        &selection.skills,
        &selection.commands,
        &selection.agents,
        &selection.claude_skills,
    ] {
        let mut unique = BTreeSet::new();
        for name in names {
            ensure!(
                valid_id(name) && unique.insert(name),
                "invalid or duplicate ECC selection name: {name}"
            );
        }
    }
    let skills: BTreeSet<_> = selection
        .skills
        .iter()
        .cloned()
        .chain(
            selection
                .commands
                .iter()
                .map(|name| format!("source-command-{name}")),
        )
        .collect();
    ensure!(
        skills.len() == selection.skills.len() + selection.commands.len(),
        "ECC command wrapper overlaps a selected skill"
    );
    ensure!(
        selection
            .claude_skills
            .iter()
            .all(|name| skills.contains(name)),
        "Claude mirror requires a selected ECC skill or command wrapper"
    );
    let mut targets = BTreeSet::new();
    for file in &selection.files {
        relative_path(&file.source)?;
        relative_path(&file.target)?;
        ensure!(
            targets.insert(&file.target),
            "duplicate ECC file target: {}",
            file.target.display()
        );
    }
    Ok(selection)
}

// shortcut: ECC name and description use scalar YAML; add a YAML parser if upstream changes that format.
fn frontmatter(text: &str) -> Result<(BTreeMap<String, String>, &str)> {
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().context("ECC source is empty")?;
    ensure!(
        first.trim_end() == "---",
        "ECC source requires YAML frontmatter"
    );
    let mut end = first.len();
    let mut header_end = None;
    for line in lines {
        if line.trim_end() == "---" {
            header_end = Some(end);
            end += line.len();
            break;
        }
        end += line.len();
    }
    let header = &text[first.len()..header_end.context("ECC frontmatter is not closed")?];
    let lines: Vec<_> = header.lines().collect();
    let mut fields = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        let Some((key @ ("name" | "description"), raw)) = line.split_once(':') else {
            continue;
        };
        let raw = raw.trim();
        let value = if matches!(raw, ">" | ">-" | ">+" | "|" | "|-" | "|+") {
            let parts: Vec<_> = lines[index + 1..]
                .iter()
                .take_while(|line| line.is_empty() || line.starts_with(char::is_whitespace))
                .map(|line| line.trim())
                .collect();
            parts
                .join(if raw.starts_with('>') { " " } else { "\n" })
                .trim()
                .to_owned()
        } else if raw.starts_with('"') {
            serde_json::from_str::<String>(raw)
                .context("unsupported ECC quoted frontmatter value")?
        } else if let Some(raw) = raw.strip_prefix('\'') {
            raw.strip_suffix('\'')
                .context("ECC single-quoted frontmatter is not closed")?
                .replace("''", "'")
        } else {
            raw.to_owned()
        };
        ensure!(
            !value.is_empty() && fields.insert(key.into(), value).is_none(),
            "missing or duplicate ECC {key}"
        );
    }
    Ok((fields, text[end..].trim_start_matches(['\r', '\n'])))
}

pub fn render_command(name: &str, text: &str) -> Result<String> {
    ensure!(valid_id(name), "invalid ECC command name");
    let (fields, body) = frontmatter(text)?;
    let description = fields
        .get("description")
        .context("ECC command description missing")?;
    let name = format!("source-command-{name}");
    Ok(format!(
        "---\nname: {}\ndescription: {}\n---\n\n# {name}\n\nUse this skill when the user asks to run the migrated source command `{}`.\n\n## Command Template\n\n{body}",
        serde_json::to_string(&name)?,
        serde_json::to_string(description)?,
        name.trim_start_matches("source-command-")
    ))
}

pub fn render_agent(text: &str) -> Result<String> {
    let (fields, body) = frontmatter(text)?;
    let name = fields.get("name").context("ECC agent name missing")?;
    ensure!(valid_id(name), "invalid ECC agent name");
    #[derive(Serialize)]
    struct Agent<'a> {
        name: &'a str,
        description: &'a str,
        developer_instructions: &'a str,
    }
    Ok(toml_edit::ser::to_string_pretty(&Agent {
        name,
        description: fields
            .get("description")
            .context("ECC agent description missing")?,
        developer_instructions: body,
    })?)
}

fn skill_source(upstream: &Path, name: &str) -> Result<PathBuf> {
    ["skills", ".agents/skills"]
        .into_iter()
        .map(|base| upstream.join(base).join(name))
        .find(|path| path.join("SKILL.md").is_file())
        .with_context(|| format!("selected ECC skill is missing upstream: {name}"))
}

fn validate_sources(selection: &Selection, upstream: &Path) -> Result<()> {
    for name in &selection.skills {
        let source = skill_source(upstream, name)?;
        reject_symlink_path(&source)?;
        payload_files(&source)?;
    }
    for (kind, names) in [
        ("commands", &selection.commands),
        ("agents", &selection.agents),
    ] {
        for name in names {
            let path = upstream.join(kind).join(format!("{name}.md"));
            reject_symlink_path(&path)?;
            ensure!(
                path.is_file(),
                "selected ECC {kind} source is missing: {name}"
            );
        }
    }
    for file in &selection.files {
        let path = contained(upstream, &file.source)?;
        reject_symlink_path(&path)?;
        ensure!(
            path.is_file(),
            "selected ECC file source is missing or not a regular file: {}",
            file.source.display()
        );
    }
    Ok(())
}

pub fn build_bundle(
    selection_path: &Path,
    upstream: &Path,
    installed: &Path,
    output: &Path,
) -> Result<()> {
    let selection = selection(selection_path)?;
    validate_sources(&selection, upstream)?;
    let mut files = Vec::new();
    for name in &selection.skills {
        let root = installed.join(".agents/skills").join(name);
        reject_symlink_path(&root)?;
        ensure!(
            root.join("SKILL.md").is_file(),
            "native ECC skill installation missing: {name}"
        );
        for (path, relative) in payload_files(&root)? {
            files.push((
                PathBuf::from(".agents/skills").join(name).join(relative),
                fs::read(&path)?,
                fs::metadata(&path)?.permissions().mode() & 0o111 != 0,
            ));
        }
    }
    for name in &selection.commands {
        files.push((
            PathBuf::from(format!(".agents/skills/source-command-{name}/SKILL.md")),
            render_command(
                name,
                &fs::read_to_string(upstream.join("commands").join(format!("{name}.md")))?,
            )?
            .into_bytes(),
            false,
        ));
    }
    for name in &selection.agents {
        let text = render_agent(&fs::read_to_string(
            upstream.join("agents").join(format!("{name}.md")),
        )?)?;
        let agent: serde_json::Value = toml_edit::de::from_str(&text)?;
        ensure!(
            agent["name"] == *name,
            "ECC agent frontmatter name differs from selected source: {name}"
        );
        files.push((
            PathBuf::from(format!(".codex/agents/{name}.toml")),
            text.into_bytes(),
            false,
        ));
    }
    let mut entries = Vec::new();
    let mut targets = BTreeSet::new();
    for (relative, _, _) in &files {
        targets.insert(relative.clone());
        entries.push(json!({"target":relative,"source":relative}));
        if let Ok(skill) = relative.strip_prefix(".agents/skills")
            && skill
                .components()
                .next()
                .and_then(|name| name.as_os_str().to_str())
                .is_some_and(|name| {
                    selection
                        .claude_skills
                        .iter()
                        .any(|selected| selected == name)
                })
        {
            let target = PathBuf::from(".claude/skills").join(skill);
            targets.insert(target.clone());
            entries.push(json!({"target":target,"source":relative}));
        }
    }
    for (index, file) in selection.files.iter().enumerate() {
        ensure!(
            targets.insert(file.target.clone()),
            "ECC file target overlaps another selected resource: {}",
            file.target.display()
        );
        let path = contained(upstream, &file.source)?;
        let relative = PathBuf::from("extra").join(index.to_string());
        files.push((
            relative.clone(),
            fs::read(&path)?,
            fs::metadata(&path)?.permissions().mode() & 0o111 != 0,
        ));
        entries.push(json!({"target":file.target,"source":relative}));
    }
    reject_symlink_path(output)?;
    ensure!(
        !output.exists() || fs::read_dir(output)?.next().is_none(),
        "ECC generator output must be empty"
    );
    fs::create_dir_all(output)?;
    for (relative, bytes, executable) in files {
        atomic_write(&output.join(relative), &bytes, Some(executable))?;
    }
    atomic_write(
        &output.join("bundle.json"),
        &serde_json::to_vec_pretty(&json!({"entries":entries}))?,
        Some(false),
    )?;
    Ok(())
}

pub fn unpack_package(bytes: &[u8], output: &Path) -> Result<()> {
    reject_symlink_path(output)?;
    fs::create_dir_all(output)?;
    for entry in tar::Archive::new(bytes).entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        relative_path(&path)?;
        let relative = path
            .strip_prefix("package")
            .context("ECC package entry is outside package/")?;
        let kind = entry.header().entry_type();
        ensure!(
            kind.is_file() || kind.is_dir(),
            "ECC package links and special files are not accepted"
        );
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = contained(output, relative)?;
        reject_symlink_path(&target)?;
        if kind.is_dir() {
            fs::create_dir_all(target)?;
        } else {
            let executable = entry.header().mode()? & 0o111 != 0;
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            atomic_write(&target, &bytes, Some(executable))?;
        }
    }
    Ok(())
}

pub fn generate(selection_path: &Path, output: &Path) -> Result<()> {
    let selection = selection(selection_path)?;
    let temporary = tempfile::tempdir()?;
    let mut npm = Command::new("npm");
    npm.args([
        "pack",
        "ecc-universal@latest",
        "--json",
        "--ignore-scripts",
        "--pack-destination",
    ])
    .arg(temporary.path())
    .current_dir(temporary.path());
    let packed = run_command(npm, None, Duration::from_secs(240))?;
    #[derive(Deserialize)]
    struct Package {
        filename: PathBuf,
    }
    let packages: Vec<Package> = serde_json::from_slice(&packed.stdout)?;
    ensure!(
        packages.len() == 1,
        "npm did not return exactly one ECC package"
    );
    let archive = contained(temporary.path(), &packages[0].filename)?;
    reject_symlink_path(&archive)?;
    let mut gzip = Command::new("gzip");
    gzip.arg("-dc").arg(&archive);
    let unpacked = run_command(gzip, None, Duration::from_secs(120))?;
    let upstream = temporary.path().join("upstream");
    unpack_package(&unpacked.stdout, &upstream)?;
    validate_sources(&selection, &upstream)?;
    let installed = temporary.path().join("installed");
    fs::create_dir(&installed)?;
    if !selection.skills.is_empty() {
        let mut native = Command::new("npx");
        native
            .args(["--yes", "skills@1.7.2", "add"])
            .arg(&upstream)
            .arg("--skill")
            .args(&selection.skills)
            .args(["--agent", "codex", "--yes", "--copy"])
            .current_dir(&installed)
            .env("DISABLE_TELEMETRY", "1");
        run_command(native, None, Duration::from_secs(240))?;
    }
    build_bundle(selection_path, &upstream, &installed, output)
}
