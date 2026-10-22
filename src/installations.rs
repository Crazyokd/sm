use crate::{
    files::{atomic_write, reject_symlink_path},
    model::{
        ApplyStatus, Catalog, Client, InstallationPolicy, ResourceContent, ResourceResult, valid_id,
    },
    process::run_command,
    state::State,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    time::Duration,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installation {
    pub id: String,
    #[serde(flatten)]
    pub policy: InstallationPolicy,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    id: String,
    paths: Vec<PathBuf>,
}

pub fn load(source: &Path) -> Result<Vec<Installation>> {
    let path = source.join("catalog.json");
    reject_symlink_path(&path)?;
    let catalog: Catalog = serde_json::from_slice(&fs::read(path)?)
        .map_err(|_| anyhow::anyhow!("invalid installation catalog"))?;
    let declarations: Vec<_> = catalog
        .resources
        .into_iter()
        .filter_map(|resource| match resource.content {
            ResourceContent::Installation { installation } => Some(Installation {
                id: resource.id,
                policy: installation,
            }),
            _ => None,
        })
        .collect();
    let mut ids = BTreeSet::new();
    let mut selectors = BTreeSet::new();
    for declaration in &declarations {
        ensure!(
            valid_id(&declaration.id) && ids.insert(&declaration.id),
            "invalid or duplicate installation id"
        );
        ensure!(
            !declaration.policy.source.trim().is_empty()
                && !declaration.policy.selector.trim().is_empty()
                && selectors.insert((declaration.policy.agent, &declaration.policy.selector)),
            "invalid or duplicate installation selector"
        );
        ensure!(
            !declaration.policy.paths.is_empty(),
            "installation paths required"
        );
        ensure!(
            !declaration.policy.update.is_empty()
                && declaration.policy.update.iter().all(|argv| {
                    !argv.is_empty()
                        && !argv[0].is_empty()
                        && argv.iter().all(|arg| !arg.contains('\0'))
                }),
            "installation update argv required"
        );
    }
    Ok(declarations)
}

fn target(path: &Path, home: &Path) -> Result<PathBuf> {
    let path = match path.strip_prefix("~") {
        Ok(relative) => home.join(relative),
        Err(_) => path.to_path_buf(),
    };
    ensure!(
        path.is_absolute()
            && path.starts_with(home)
            && path != home
            && !path.components().any(|c| matches!(c, Component::ParentDir)),
        "installation target must be inside home"
    );
    reject_symlink_path(&path)?;
    Ok(path)
}

fn restore_enabled(declaration: &Installation, home: &Path, update_failed: bool) -> Result<()> {
    match declaration.policy.agent {
        Client::Codex => {
            let path = target(&home.join(".codex/config.toml"), home)?;
            if !path.exists() {
                return Ok(());
            }
            let mut document = fs::read_to_string(&path)?
                .parse::<toml_edit::DocumentMut>()
                .map_err(|_| anyhow::anyhow!("invalid native plugin configuration"))?;
            let Some(plugin) = document
                .get_mut("plugins")
                .and_then(|plugins| plugins.get_mut(&declaration.policy.selector))
            else {
                return Ok(());
            };
            ensure!(
                plugin.is_table() || plugin.is_inline_table(),
                "invalid native plugin configuration"
            );
            if plugin.get("enabled").and_then(|item| item.as_bool())
                != Some(declaration.policy.enabled)
            {
                plugin["enabled"] = toml_edit::value(declaration.policy.enabled);
                atomic_write(&path, document.to_string().as_bytes(), None)?;
            }
        }
        Client::Kimi => {
            if !update_failed {
                return Ok(());
            }
            let path = target(&home.join(".kimi-code/plugins/installed.json"), home)?;
            if !path.exists() {
                return Ok(());
            }
            let mut registry: Value = serde_json::from_slice(&fs::read(&path)?)
                .map_err(|_| anyhow::anyhow!("invalid native plugin registry"))?;
            let plugins = registry["plugins"]
                .as_array_mut()
                .context("invalid native plugin registry")?;
            // A failed native API operation may have enabled the plugin before exiting.
            if let Some(plugin) = plugins
                .iter_mut()
                .find(|p| p["id"] == declaration.policy.selector)
            {
                if plugin["enabled"] != declaration.policy.enabled {
                    plugin["enabled"] = declaration.policy.enabled.into();
                    atomic_write(&path, &serde_json::to_vec_pretty(&registry)?, None)?;
                }
            }
        }
    }
    Ok(())
}

fn verify(declaration: &Installation, home: &Path, paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        target(path, home)?;
        ensure!(path.is_dir(), "native installation path missing");
    }
    match declaration.policy.agent {
        Client::Codex => {
            let path = target(&home.join(".codex/config.toml"), home)?;
            let document = fs::read_to_string(path)?
                .parse::<toml_edit::DocumentMut>()
                .map_err(|_| anyhow::anyhow!("invalid native plugin configuration"))?;
            ensure!(
                document
                    .get("plugins")
                    .and_then(|plugins| plugins.get(&declaration.policy.selector))
                    .and_then(|plugin| plugin.get("enabled"))
                    .and_then(|enabled| enabled.as_bool())
                    == Some(declaration.policy.enabled),
                "native plugin registration missing or enabled state differs"
            );
            let (_, market) = declaration
                .policy
                .selector
                .split_once('@')
                .context("invalid native plugin selector")?;
            if market != "openai-api-curated" {
                let marketplace = document
                    .get("marketplaces")
                    .and_then(|m| m.get(market))
                    .context("native marketplace registration missing")?;
                let source = marketplace
                    .get("source")
                    .and_then(|s| s.as_str())
                    .context("native marketplace source missing")?;
                let actual =
                    if marketplace.get("source_type").and_then(|s| s.as_str()) == Some("local") {
                        let root = target(Path::new(source), home)?;
                        crate::repository::git(&root, &["remote", "get-url", "origin"])?
                    } else {
                        source.to_owned()
                    };
                ensure!(
                    same_source(&actual, &declaration.policy.source),
                    "native marketplace source differs from declaration"
                );
            }
        }
        Client::Kimi => {
            let path = target(&home.join(".kimi-code/plugins/installed.json"), home)?;
            let registry: Value = serde_json::from_slice(&fs::read(path)?)
                .map_err(|_| anyhow::anyhow!("invalid native plugin registry"))?;
            let plugin = registry["plugins"]
                .as_array()
                .and_then(|plugins| {
                    plugins
                        .iter()
                        .find(|p| p["id"] == declaration.policy.selector)
                })
                .context("native plugin registration missing")?;
            ensure!(
                plugin["enabled"] == declaration.policy.enabled,
                "native plugin enabled state differs"
            );
            ensure!(
                same_kimi_source(plugin, &declaration.policy.source),
                "native plugin source differs from declaration"
            );
            let root = target(
                Path::new(
                    plugin["root"]
                        .as_str()
                        .context("native plugin root missing")?,
                ),
                home,
            )?;
            ensure!(
                root.is_dir() && paths.iter().any(|path| root.starts_with(path)),
                "native plugin root does not match declared paths"
            );
            // Kimi persists registrations; readiness is computed only when its runtime loads them.
            let root_manifest = root.join("kimi.plugin.json");
            let manifest_path = if root_manifest.is_file() {
                root_manifest
            } else {
                root.join(".kimi-plugin/plugin.json")
            };
            reject_symlink_path(&manifest_path)?;
            let manifest: Value = serde_json::from_slice(&fs::read(manifest_path)?)
                .map_err(|_| anyhow::anyhow!("invalid native plugin manifest"))?;
            ensure!(
                manifest["name"]
                    .as_str()
                    .is_some_and(|name| name.trim().to_lowercase() == declaration.policy.selector),
                "native plugin manifest identity differs from declaration"
            );
        }
    }
    Ok(())
}

fn same_source(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        value
            .trim_end_matches('/')
            .strip_suffix(".git")
            .unwrap_or(value.trim_end_matches('/'))
            .to_owned()
    };
    normalize(left) == normalize(right)
}

fn same_kimi_source(plugin: &Value, source: &str) -> bool {
    let Some(installed_source) = plugin["originalSource"].as_str() else {
        return false;
    };
    if same_source(installed_source, source) {
        return true;
    }
    let github = |value: &str| -> Option<Vec<String>> {
        let url = url::Url::parse(value).ok()?;
        (url.scheme() == "https"
            && matches!(url.host_str(), Some("github.com" | "www.github.com"))
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.query().is_none()
            && url.fragment().is_none())
        .then(|| {
            url.path_segments()
                .map(|p| p.filter(|v| !v.is_empty()).map(str::to_owned).collect())
        })
        .flatten()
    };
    let (Some(declared), Some(installed)) = (github(source), github(installed_source)) else {
        return false;
    };
    declared.len() == 2
        && installed.len() == 4
        && declared[0] == installed[0]
        && declared[1].trim_end_matches(".git") == installed[1]
        && installed[2] == "commit"
        && installed[3].len() == 40
        && installed[3].bytes().all(|b| b.is_ascii_hexdigit())
        && plugin["source"] == "github"
        && plugin["github"]["owner"] == declared[0]
        && plugin["github"]["repo"] == installed[1]
        && plugin["github"]["installedSha"] == installed[3]
        && plugin["github"]["ref"]["kind"] == "sha"
        && plugin["github"]["ref"]["value"] == installed[3]
}

pub fn synchronize(
    source: &Path,
    home: &Path,
    state: &State,
) -> Result<BTreeMap<String, ResourceResult>> {
    ensure!(home.is_absolute(), "home must be absolute");
    reject_symlink_path(home)?;
    let declarations = load(source)?;
    let previous: Vec<Receipt> = state.get("external_installations")?;
    // Validate every target before any approved installer can run.
    let targets: Vec<Vec<PathBuf>> = declarations
        .iter()
        .map(|declaration| {
            declaration
                .policy
                .paths
                .iter()
                .map(|p| target(p, home))
                .collect()
        })
        .collect::<Result<_>>()?;
    let mut results = BTreeMap::new();
    let mut receipts = Vec::new();
    for (declaration, paths) in declarations.iter().zip(&targets) {
        let updated = (|| -> Result<()> {
            for argv in &declaration.policy.update {
                if matches!(
                    Path::new(&argv[0]).file_name().and_then(|s| s.to_str()),
                    Some("codex" | "kimi")
                ) {
                    ensure!(
                        std::env::var_os("HOME").as_deref() == Some(home.as_os_str()),
                        "native installer home differs from runtime home"
                    );
                    let (key, expected) = match declaration.policy.agent {
                        Client::Codex => ("CODEX_HOME", home.join(".codex")),
                        Client::Kimi => ("KIMI_CODE_HOME", home.join(".kimi-code")),
                    };
                    ensure!(
                        std::env::var_os(key).is_none_or(|value| Path::new(&value) == expected),
                        "native installer configuration root differs from declared home"
                    );
                }
                let mut command = Command::new(&argv[0]);
                command
                    .args(argv[1..].iter().map(|arg| {
                        if arg == "${SM_ENABLED}" {
                            declaration.policy.enabled.to_string()
                        } else {
                            arg.clone()
                        }
                    }))
                    .current_dir(source)
                    .env("SM_INSTALL_HOME", home)
                    .env("KIMI_CODE_NO_AUTO_UPDATE", "1")
                    .env("SM_INTERNAL_RUN", "1");
                // Installer output can include local configuration or credentials.
                run_command(command, None, Duration::from_secs(120))
                    .map_err(|_| anyhow::anyhow!("native installer command failed"))?;
            }
            Ok(())
        })();
        // Re-read the installer's output and restore only this selector, including after failure.
        let restored = restore_enabled(declaration, home, updated.is_err());
        let verified = restored.and_then(|_| verify(declaration, home, paths));
        let keep_existing = updated.is_err()
            && verified.is_ok()
            && previous
                .iter()
                .any(|p| p.id == declaration.id && p.paths == *paths);
        let checked = updated.and(verified);
        if checked.is_ok() || keep_existing {
            receipts.push(Receipt {
                id: declaration.id.clone(),
                paths: paths.clone(),
            });
        }
        let result = if checked.is_ok() {
            ResourceResult::new(
                ApplyStatus::Applied,
                "native installer completed; registration and installed files verified",
            )
        } else {
            ResourceResult::new(
                ApplyStatus::Failed,
                "native installation update or verification failed",
            )
        };
        results.insert(declaration.id.clone(), result);
    }
    state.set("external_installations", &receipts)?;
    Ok(results)
}

/// Record verified existing registrations without running a network installer.
pub fn register_existing(source: &Path, home: &Path, state: &State) -> Result<()> {
    let declarations = load(source)?;
    let mut receipts = Vec::new();
    for declaration in &declarations {
        let paths = declaration
            .policy
            .paths
            .iter()
            .map(|p| target(p, home))
            .collect::<Result<Vec<_>>>()?;
        if verify(declaration, home, &paths).is_ok() {
            receipts.push(serde_json::json!({"id":declaration.id,"paths":paths}));
        }
    }
    state.set("external_installations", &receipts)
}
