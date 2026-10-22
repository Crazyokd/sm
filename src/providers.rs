use crate::{
    files::{payload_files, shell_join},
    model::{Client, OwnedWrite, Resource, ResourceKind, Scope, WriteBody},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

#[derive(Debug, thiserror::Error)]
#[error("Kimi project hooks have no native project configuration")]
pub struct UnsupportedResource;

fn copy_payload(resource: &Resource, payload: &Path, target: &Path) -> Result<Vec<OwnedWrite>> {
    payload_files(payload)?
        .into_iter()
        .map(|(path, relative)| {
            Ok(OwnedWrite::new(
                &resource.id,
                target.join(relative),
                WriteBody::File {
                    bytes: fs::read(&path)?,
                    executable: fs::metadata(path)?.permissions().mode() & 0o111 != 0,
                },
            ))
        })
        .collect()
}
fn native_config(
    payload: &Path,
    filename: &str,
    toml: bool,
    section: &str,
) -> Result<Map<String, Value>> {
    let text = fs::read_to_string(payload.join(filename))?;
    let value: Value = if toml {
        toml_edit::de::from_str(&text)?
    } else {
        serde_json::from_str(&text)?
    };
    let content = value
        .as_object()
        .context("native configuration must be an object")?;
    ensure!(
        content.len() == 1 && content.contains_key(section),
        "native payload may only contain {section}"
    );
    Ok(content.clone())
}
fn hook_command(command: &str, directory: &Path) -> String {
    shell_join([
        "env".into(),
        format!("SM_RESOURCE_DIR={}", directory.display()),
        "sh".into(),
        "-c".into(),
        command.into(),
    ])
}
pub fn collector(client: Client, home: &Path, executable: &Path, state: &Path) -> OwnedWrite {
    let command = shell_join([
        executable.to_string_lossy().as_ref(),
        "record",
        "--agent",
        &client.to_string(),
        "--state-dir",
        state.to_string_lossy().as_ref(),
    ]);
    let events = ["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop"];
    match client {
        Client::Codex => {
            let hooks: Map<String, Value> = events
                .into_iter()
                .map(|event| {
                    (
                        event.into(),
                        json!([{"hooks":[{"type":"command","command":command,"timeout":2}]}]),
                    )
                })
                .collect();
            OwnedWrite::new(
                "@observer/codex",
                home.join(".codex/hooks.json"),
                WriteBody::Json(Map::from_iter([("hooks".into(), Value::Object(hooks))])),
            )
        }
        Client::Kimi => {
            let hooks: Vec<_> = events
                .into_iter()
                .map(|event| json!({"event":event,"command":command,"timeout":2}))
                .collect();
            OwnedWrite::new(
                "@observer/kimi",
                home.join(".kimi-code/config.toml"),
                WriteBody::Toml(Map::from_iter([("hooks".into(), json!(hooks))])),
            )
        }
    }
}
pub fn render(
    resource: &Resource,
    client: Client,
    payload: &Path,
    scope: &Path,
) -> Result<Vec<OwnedWrite>> {
    let kind = resource.content.kind();
    if kind == ResourceKind::Hook && client == Client::Kimi && resource.scope == Scope::Project {
        return Err(UnsupportedResource.into());
    }
    payload_files(payload)?;
    let base = scope.join(match client {
        Client::Codex => ".codex",
        Client::Kimi => ".kimi-code",
    });
    match kind {
        ResourceKind::Bundle => crate::bundles::render(resource, payload, scope),
        ResourceKind::Skill => {
            fs::read_to_string(payload.join("SKILL.md"))?;
            copy_payload(
                resource,
                payload,
                &scope.join(".agents/skills").join(&resource.id),
            )
        }
        ResourceKind::Rule => {
            let path = if resource.scope == Scope::User {
                base.join("AGENTS.md")
            } else {
                scope.join("AGENTS.md")
            };
            Ok(vec![OwnedWrite::new(
                &resource.id,
                path,
                WriteBody::Markdown(fs::read(payload.join("AGENTS.md"))?),
            )])
        }
        ResourceKind::Agent => {
            let (filename, extension) = match client {
                Client::Codex => ("agent.toml", "toml"),
                Client::Kimi => ("agent.md", "md"),
            };
            let content = fs::read(payload.join(filename))?;
            if client == Client::Codex {
                std::str::from_utf8(&content)?.parse::<toml_edit::DocumentMut>()?;
            }
            Ok(vec![OwnedWrite::new(
                &resource.id,
                base.join("agents")
                    .join(format!("{}.{extension}", resource.id)),
                WriteBody::File {
                    bytes: content,
                    executable: false,
                },
            )])
        }
        ResourceKind::Mcp => {
            let (filename, section, toml) = match client {
                Client::Codex => ("config.toml", "mcp_servers", true),
                Client::Kimi => ("mcp.json", "mcpServers", false),
            };
            let content = native_config(payload, filename, toml, section)?;
            ensure!(
                content[section].is_object(),
                "MCP services must be an object"
            );
            Ok(vec![OwnedWrite::new(
                &resource.id,
                base.join(filename),
                if toml {
                    WriteBody::Toml(content)
                } else {
                    WriteBody::Json(content)
                },
            )])
        }
        ResourceKind::Hook => {
            let directory = base.join("hooks/sm").join(&resource.id);
            let mut content = match client {
                Client::Codex => native_config(payload, "hooks.json", false, "hooks")?,
                Client::Kimi => native_config(payload, "config.toml", true, "hooks")?,
            };
            match client {
                Client::Codex => {
                    for groups in content
                        .get_mut("hooks")
                        .and_then(Value::as_object_mut)
                        .context("hooks must be an object")?
                        .values_mut()
                    {
                        for group in groups
                            .as_array_mut()
                            .context("hook event entries must be an array")?
                        {
                            for hook in group
                                .get_mut("hooks")
                                .and_then(Value::as_array_mut)
                                .context("hook handlers must be an array")?
                            {
                                ensure!(
                                    hook.get("type").and_then(Value::as_str) == Some("command"),
                                    "SM hook payload requires command handlers"
                                );
                                let command = hook
                                    .get("command")
                                    .and_then(Value::as_str)
                                    .context("hook command required")?;
                                hook["command"] = json!(hook_command(command, &directory));
                            }
                        }
                    }
                }
                Client::Kimi => {
                    for hook in content
                        .get_mut("hooks")
                        .and_then(Value::as_array_mut)
                        .context("Kimi hooks must be an array")?
                    {
                        let object = hook.as_object_mut().context("hook must be an object")?;
                        ensure!(
                            object
                                .keys()
                                .all(|key| ["event", "matcher", "command", "timeout"]
                                    .contains(&key.as_str())),
                            "unknown Kimi hook field"
                        );
                        let command = object
                            .get("command")
                            .and_then(Value::as_str)
                            .context("hook command required")?;
                        object.insert("command".into(), json!(hook_command(command, &directory)));
                    }
                }
            }
            let mut writes = copy_payload(resource, payload, &directory)?;
            writes.push(OwnedWrite::new(
                &resource.id,
                base.join(match client {
                    Client::Codex => "hooks.json",
                    Client::Kimi => "config.toml",
                }),
                match client {
                    Client::Codex => WriteBody::Json(content),
                    Client::Kimi => WriteBody::Toml(content),
                },
            ));
            Ok(writes)
        }
    }
}
