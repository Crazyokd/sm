use crate::{
    files::{atomic_write, contained, reject_symlink_path},
    model::{
        ApplyPlan, ApplyReport, ApplyStatus, Catalog, OwnedWrite, ResourceResult, Scope, WriteBody,
    },
    providers::{self, UnsupportedResource},
    state::State,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use toml_edit::{ArrayOfTables, DocumentMut, Item, TableLike};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Conflict(pub String);
fn conflict(message: impl Into<String>) -> anyhow::Error {
    Conflict(message.into()).into()
}

pub fn plan_apply(
    catalog: &Catalog,
    sources: &BTreeMap<String, PathBuf>,
    home: &Path,
) -> ApplyPlan {
    let mut plan = ApplyPlan::default();
    for resource in &catalog.resources {
        let build = || -> Result<Vec<OwnedWrite>> {
            let scope = catalog.scope_root(resource, home)?;
            ensure!(
                resource.scope != Scope::Project || scope.is_dir(),
                "project directory missing: {}",
                scope.display()
            );
            let source = sources
                .get(&resource.id)
                .context("resource source unavailable")?;
            let mut writes = Vec::new();
            for (client, path) in resource.targets() {
                for write in
                    providers::render(resource, client, &contained(source, &path)?, &scope)?
                {
                    if !writes.contains(&write) {
                        writes.push(write);
                    }
                }
            }
            Ok(writes)
        };
        match build() {
            Ok(writes) => plan.writes.extend(writes),
            Err(error) => {
                plan.results.insert(
                    resource.id.clone(),
                    ResourceResult::new(
                        if error.is::<UnsupportedResource>() {
                            ApplyStatus::Unsupported
                        } else {
                            ApplyStatus::Failed
                        },
                        format!("{error:#}"),
                    ),
                );
            }
        }
    }
    plan
}
fn block(write: &OwnedWrite, bytes: &[u8]) -> Result<String> {
    Ok(format!(
        "<!-- SM:{} -->\n{}\n<!-- /SM:{} -->\n",
        write.resource_id,
        std::str::from_utf8(bytes)?.trim_end(),
        write.resource_id
    ))
}

pub fn merge_entries(
    document: &mut Value,
    fragment: &Map<String, Value>,
    remove: bool,
) -> Result<()> {
    let document = document
        .as_object_mut()
        .ok_or_else(|| conflict("configuration must be an object"))?;
    for (section, values) in fragment {
        if let Some(values) = values.as_array() {
            let target = document
                .entry(section)
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(|| {
                    conflict(format!("configuration section must be an array: {section}"))
                })?;
            merge_array(target, values, remove)?;
        } else if section == "hooks" {
            let values = values
                .as_object()
                .ok_or_else(|| conflict("hooks configuration must be an object"))?;
            let target = document
                .entry(section)
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| conflict("hooks configuration must be an object"))?;
            for (event, groups) in values {
                let groups = groups
                    .as_array()
                    .ok_or_else(|| conflict("hook event entries must be an array"))?;
                let entries = target
                    .entry(event)
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .ok_or_else(|| conflict("hook event entries must be an array"))?;
                merge_array(entries, groups, remove)?;
                if entries.is_empty() {
                    target.remove(event);
                }
            }
        } else {
            let values = values.as_object().ok_or_else(|| {
                conflict(format!(
                    "configuration section must be an object: {section}"
                ))
            })?;
            let target = document
                .entry(section)
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| {
                    conflict(format!(
                        "configuration section must be an object: {section}"
                    ))
                })?;
            for (key, value) in values {
                if remove {
                    if target.get(key) != Some(value) {
                        return Err(conflict(format!("managed entry was edited: {key}")));
                    }
                    target.remove(key);
                } else {
                    if target.contains_key(key) {
                        return Err(conflict(format!("unmanaged or duplicate entry: {key}")));
                    }
                    target.insert(key.clone(), value.clone());
                }
            }
        }
    }
    Ok(())
}
fn merge_array(target: &mut Vec<Value>, values: &[Value], remove: bool) -> Result<()> {
    for value in values {
        if remove {
            let index = target
                .iter()
                .position(|entry| entry == value)
                .ok_or_else(|| conflict("managed hook was edited or removed"))?;
            target.remove(index);
        } else {
            if target.contains(value) {
                return Err(conflict("hook already exists outside this resource"));
            }
            target.push(value.clone());
        }
    }
    Ok(())
}
fn value_item(value: &Value) -> Result<Item> {
    let document = toml_edit::ser::to_document(&BTreeMap::from([("value", value)]))?;
    document
        .get("value")
        .cloned()
        .context("cannot encode TOML item")
}
fn patch_table(
    table: &mut dyn TableLike,
    before: &Map<String, Value>,
    after: &Map<String, Value>,
) -> Result<()> {
    for key in before.keys().filter(|key| !after.contains_key(*key)) {
        table.remove(key);
    }
    for (key, value) in after {
        if before.get(key) == Some(value) {
            continue;
        }
        match (table.get_mut(key), before.get(key)) {
            (Some(item), Some(previous)) => patch_item(item, previous, value)?,
            _ => {
                table.insert(key, value_item(value)?);
            }
        }
    }
    Ok(())
}
fn patch_item(item: &mut Item, before: &Value, after: &Value) -> Result<()> {
    if let (Some(before), Some(after)) = (before.as_object(), after.as_object()) {
        if let Some(table) = item.as_table_like_mut() {
            return patch_table(table, before, after);
        }
    }
    if let (Some(before), Some(after), Some(tables)) = (
        before.as_array(),
        after.as_array(),
        item.as_array_of_tables(),
    ) {
        if !after.is_empty() && after.iter().all(Value::is_object) {
            let mut result = ArrayOfTables::new();
            let mut used = BTreeSet::new();
            for value in after {
                if let Some((index, _)) = before
                    .iter()
                    .enumerate()
                    .find(|(index, old)| !used.contains(index) && *old == value)
                {
                    used.insert(index);
                    result.push(
                        tables
                            .get(index)
                            .context("TOML array shape changed")?
                            .clone(),
                    );
                } else {
                    let table = value_item(value)?;
                    let table = match table {
                        Item::Table(table) => table,
                        Item::Value(toml_edit::Value::InlineTable(table)) => table.into_table(),
                        _ => return Err(conflict("invalid TOML hook table")),
                    };
                    result.push(table);
                }
            }
            *item = Item::ArrayOfTables(result);
            return Ok(());
        }
    }
    *item = value_item(after)?;
    Ok(())
}
type ContentChange = (Option<Vec<u8>>, Option<Vec<u8>>);

fn prepare(path: &Path, new: &[OwnedWrite], old: &[OwnedWrite]) -> Result<ContentChange> {
    reject_symlink_path(path).map_err(|e| conflict(e.to_string()))?;
    let body = &new
        .first()
        .or_else(|| old.first())
        .context("target has no owner")?
        .body;
    if new
        .iter()
        .chain(old)
        .any(|w| std::mem::discriminant(&w.body) != std::mem::discriminant(body))
    {
        return Err(conflict("incompatible writes to the same path"));
    }
    let content = if path.exists() {
        Some(fs::read(path)?)
    } else {
        None
    };
    let result = match body {
        WriteBody::File { .. } => {
            if old.len() > 1 {
                return Err(conflict("multiple file owners"));
            }
            if let Some(previous) = old.first() {
                let WriteBody::File { bytes, .. } = &previous.body else {
                    return Err(conflict("invalid file ownership"));
                };
                if content.as_ref() != Some(bytes) {
                    return Err(conflict("managed file was changed locally"));
                }
            } else if content.is_some() {
                return Err(conflict("unmanaged file already exists"));
            }
            if new.len() > 1 {
                return Err(conflict("multiple resources target the same file"));
            }
            match new.first().map(|w| &w.body) {
                Some(WriteBody::File { bytes, .. }) => Some(bytes.clone()),
                None => None,
                _ => return Err(conflict("invalid file write")),
            }
        }
        WriteBody::Markdown(_) => {
            let mut text = String::from_utf8(content.clone().unwrap_or_default())?;
            for write in old {
                let WriteBody::Markdown(bytes) = &write.body else {
                    return Err(conflict("invalid rule ownership"));
                };
                let block = block(write, bytes)?;
                if text.matches(&block).count() != 1 {
                    return Err(conflict("managed rules were changed locally"));
                }
                text = text.replacen(&block, "", 1);
            }
            for write in new {
                let WriteBody::Markdown(bytes) = &write.body else {
                    return Err(conflict("invalid rule write"));
                };
                if text.contains(&format!("<!-- SM:{} -->", write.resource_id)) {
                    return Err(conflict("unmanaged resource marker already exists"));
                }
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&block(write, bytes)?);
            }
            if text.is_empty() {
                None
            } else {
                Some(text.into_bytes())
            }
        }
        WriteBody::Json(_) | WriteBody::Toml(_) => {
            let text = std::str::from_utf8(content.as_deref().unwrap_or_default())?;
            let original: Value = if matches!(body, WriteBody::Toml(_)) {
                toml_edit::de::from_str(text)?
            } else if text.is_empty() {
                Value::Object(Map::new())
            } else {
                serde_json::from_str(text)?
            };
            let mut document = original.clone();
            for (writes, remove) in [(old, true), (new, false)] {
                for write in writes {
                    let fragment = match &write.body {
                        WriteBody::Json(value) | WriteBody::Toml(value) => value,
                        _ => return Err(conflict("invalid config write")),
                    };
                    merge_entries(&mut document, fragment, remove)?;
                }
            }
            if matches!(body, WriteBody::Toml(_)) {
                let mut edited: DocumentMut = text.parse()?;
                patch_table(
                    edited.as_table_mut(),
                    original
                        .as_object()
                        .context("TOML root must be an object")?,
                    document
                        .as_object()
                        .context("TOML root must be an object")?,
                )?;
                Some(edited.to_string().into_bytes())
            } else {
                Some((serde_json::to_string_pretty(&document)? + "\n").into_bytes())
            }
        }
    };
    Ok((content, result))
}
struct Prepared {
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    writes: Vec<OwnedWrite>,
    owners: BTreeSet<String>,
}
pub fn apply_writes(plan: &ApplyPlan, state: &State) -> Result<ApplyReport> {
    let mut report = ApplyReport {
        results: plan.results.clone(),
    };
    let mut desired: BTreeMap<PathBuf, Vec<OwnedWrite>> = BTreeMap::new();
    let mut previous: BTreeMap<PathBuf, Vec<OwnedWrite>> = BTreeMap::new();
    for write in &plan.writes {
        let entries = desired.entry(write.path.clone()).or_default();
        if !entries.contains(write) {
            entries.push(write.clone());
        }
    }
    for write in state.owned()? {
        previous.entry(write.path.clone()).or_default().push(write);
    }
    let paths: BTreeSet<_> = desired.keys().chain(previous.keys()).cloned().collect();
    let mut prepared = BTreeMap::new();
    let mut blocked: BTreeSet<_> = plan.results.keys().cloned().collect();
    for path in paths {
        let new = desired.remove(&path).unwrap_or_default();
        let old = previous.remove(&path).unwrap_or_default();
        let owners: BTreeSet<_> = new
            .iter()
            .chain(&old)
            .map(|w| w.resource_id.clone())
            .collect();
        match prepare(&path, &new, &old) {
            Ok((before, after)) => {
                prepared.insert(
                    path,
                    Prepared {
                        before,
                        after,
                        writes: new,
                        owners,
                    },
                );
            }
            Err(error) => {
                for owner in owners {
                    blocked.insert(owner.clone());
                    report.results.insert(
                        owner,
                        ResourceResult::new(
                            if error.is::<Conflict>() {
                                ApplyStatus::Conflict
                            } else {
                                ApplyStatus::Failed
                            },
                            format!("{error:#}"),
                        ),
                    );
                }
            }
        }
    }
    let mut order: Vec<_> = prepared.into_iter().collect();
    order.sort_by_key(|(path, p)| {
        (
            p.after.is_none(),
            p.writes
                .iter()
                .any(|w| !matches!(w.body, WriteBody::File { .. })),
            path.clone(),
        )
    });
    let block_related = |blocked: &mut BTreeSet<String>, report: &mut ApplyReport| {
        loop {
            let mut expanded = blocked.clone();
            for (_, entry) in &order {
                if !entry.owners.is_disjoint(blocked) {
                    expanded.extend(entry.owners.iter().cloned());
                }
            }
            if expanded == *blocked {
                break;
            }
            *blocked = expanded;
        }
        for owner in blocked.iter() {
            if report
                .results
                .get(owner)
                .is_none_or(|result| result.status == ApplyStatus::Applied)
            {
                report.results.insert(
                    owner.clone(),
                    ResourceResult::new(ApplyStatus::Conflict, "共享目标的其他受管资源未通过检查"),
                );
            }
        }
    };
    block_related(&mut blocked, &mut report);
    for (path, entry) in &order {
        if !entry.owners.is_disjoint(&blocked) {
            continue;
        }
        let result = || -> Result<()> {
            match &entry.after {
                None => {
                    if path.exists() {
                        fs::remove_file(path)?;
                    }
                }
                Some(bytes) => {
                    let executable = match entry.writes.first().map(|w| &w.body) {
                        Some(WriteBody::File { executable, .. }) => Some(*executable),
                        _ => None,
                    };
                    let mode_changed = if let Some(executable) = executable {
                        path.exists()
                            && (fs::metadata(path)?.permissions().mode() & 0o111 != 0) != executable
                    } else {
                        false
                    };
                    if entry.before.as_ref() != Some(bytes) || mode_changed {
                        atomic_write(path, bytes, executable)?;
                    }
                }
            }
            state.replace_owned(path, &entry.writes)
        };
        match result() {
            Ok(()) => {
                for owner in &entry.owners {
                    report.results.insert(
                        owner.clone(),
                        ResourceResult::new(
                            ApplyStatus::Applied,
                            "受管文件已应用；Agent 加载状态尚未核验",
                        ),
                    );
                }
            }
            Err(error) => {
                for owner in &entry.owners {
                    blocked.insert(owner.clone());
                    report.results.insert(
                        owner.clone(),
                        ResourceResult::new(ApplyStatus::Failed, format!("{error:#}")),
                    );
                }
                block_related(&mut blocked, &mut report);
            }
        }
    }
    state.set("resource_results", &report.results)?;
    Ok(report)
}
