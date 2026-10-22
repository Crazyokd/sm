# Partial-Install Recovery

Read this when a dependency was updated but the following package installation failed. For `easygo-app`, installing the parameter interfaces first can leave the app unpacked but unconfigured if the app install fails. Inspect the actual failure and package audit before retrying; restore a consistent package state before reporting success.

Use a verified matching deb for a focused retry through the existing `deploy_deb` function. This preserves the script's deployment gates and avoids invoking the full dependency sequence again. For the app, check that its `Depends` requires the exact parameter-interface version already installed and that both versions carry the intended source hash. Keep the source checkout pinned to that SHA during recovery.

The CLI `--deploy-only` skips compilation but repackages ordinary deb targets from existing build output; it is not a selector for the deb previously inspected.

```bash
python3 - <<'PY'
import importlib.util
from pathlib import Path

host = "<target-or-wrapper>"
deb = Path("deploy-dev-output/<matching-easygo-app-deb-name>")
spec = importlib.util.spec_from_file_location("deploy_dev", "script/deploy-dev.py")
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
mod.deploy_deb(deb, "easygo-app", host, None, None, "<container>", "easygo-app", "<arch>")
PY
```

Replace the host, deb path, container, and architecture with the resolved deployment values. The two `None` arguments are port and password: supply the resolved values for direct SSH when needed. For a wrapper route, prefix the command with `PATH="$WRAPPER_DIR/bin:$PATH"` and let the wrapper handle target authentication.

After recovery, resume independent target verification and local-checkout restoration from the main workflow. If repair cannot complete, report the exact remaining package/runtime state and blocker.
