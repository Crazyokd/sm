use crate::{
    analysis::{Candidate, editable_text},
    files::{contained, payload_files},
    model::{CheckKind, ValidationCheck, ValidationReport, ValidationStatus},
    process::run_command,
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{fs, path::Path, process::Command, time::Duration};
#[derive(Deserialize, Default)]
struct Cases {
    #[serde(default)]
    required: Vec<String>,
    #[serde(default)]
    forbidden: Vec<String>,
}
pub(crate) fn syntax(path: &Path) -> Result<()> {
    let text = fs::read_to_string(path)?;
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => {
            serde_json::from_str::<serde_json::Value>(&text)?;
        }
        Some("toml") => {
            text.parse::<toml_edit::DocumentMut>()?;
        }
        Some("py") => {
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&tree_sitter_python::LANGUAGE.into())?;
            let tree = parser
                .parse(&text, None)
                .context("Python parser cancelled")?;
            ensure!(!tree.root_node().has_error(), "invalid Python syntax");
        }
        Some("sh") => {
            let mut command = Command::new("bash");
            command.arg("-n").arg(path);
            run_command(command, None, Duration::from_secs(10))?;
        }
        _ => {}
    }
    if path.file_name().is_some_and(|n| n == "SKILL.md") {
        let header: Vec<_> = text.splitn(3, "---").collect();
        ensure!(
            header.len() == 3
                && text.starts_with("---\n")
                && ["name:", "description:"]
                    .iter()
                    .all(|field| header[1].lines().any(|line| line.starts_with(field))),
            "Skill requires name and description frontmatter"
        );
    }
    Ok(())
}
fn markdown(root: &Path) -> Result<String> {
    if !root.exists() {
        return Ok(String::new());
    }
    let mut content = Vec::new();
    for (path, _) in payload_files(root)? {
        if editable_text(&path) && path.extension().is_some_and(|e| e == "md") {
            content.push(fs::read_to_string(path)?);
        }
    }
    Ok(content.join("\n"))
}
pub fn evaluate_candidate(
    candidate: &Candidate,
    original: &Path,
    proposed: &Path,
) -> Result<ValidationReport> {
    let mut checks = Vec::new();
    for name in candidate.changes.keys() {
        let path = contained(proposed, Path::new(name))?;
        let result = syntax(&path);
        checks.push(ValidationCheck {
            name: format!("syntax:{name}"),
            kind: CheckKind::Static,
            before: None,
            after: result.is_ok(),
            error: result
                .err()
                .map(|error| crate::privacy::redact_text(&error.to_string(), Some(300))),
        });
    }
    let base = Path::new("resources").join(&candidate.resource_id);
    let casefile = original.join(&base).join("evals.json");
    if casefile.is_file() {
        let cases: Cases = serde_json::from_slice(&fs::read(casefile)?)?;
        let before = markdown(&original.join(&base))?;
        let after = markdown(&proposed.join(&base))?;
        for (values, required) in [(cases.required, true), (cases.forbidden, false)] {
            for word in values {
                checks.push(ValidationCheck {
                    name: format!("{}:{word}", if required { "required" } else { "forbidden" }),
                    kind: CheckKind::ApprovedFragmentCheck,
                    before: Some(before.contains(&word) == required),
                    after: after.contains(&word) == required,
                    error: None,
                });
            }
        }
    }
    Ok(ValidationReport {
        status: if checks.iter().any(|c| !c.after) {
            ValidationStatus::Failed
        } else {
            ValidationStatus::Partial
        },
        checks,
        unverified: vec![
            "尚未真实复跑业务行为；静态和文本案例检查不证明执行效果，需人工审阅".into(),
        ],
    })
}
