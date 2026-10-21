from pathlib import Path

from .catalog import Catalog, Resource


def resolve_scope(resource: Resource, catalog: Catalog, home: Path) -> Path:
    if resource.scope == "user":
        return home
    value = catalog.projects[resource.project]
    return home / value[2:] if value.startswith("~/") else Path(value)


def state_dir(home: Path) -> Path:
    return home / ".local/state/sm"
