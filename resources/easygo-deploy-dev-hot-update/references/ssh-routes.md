# SSH Routes

Read this only when the requested target is behind one or more intermediate hosts. Resolve aliases/IPs and hop order from the current request; do not assume a particular robot, jump host, or network.

Use an existing working SSH route when available. If nested SSH is needed, create a temporary wrapper with the bundled [helper](../scripts/make_nested_ssh_wrapper.sh):

```bash
WRAPPER_DIR="$(.agents/skills/easygo-deploy-dev-hot-update/scripts/make_nested_ssh_wrapper.sh <wrapper-host> <hop1> [<hop2> ...] -- <target-alias-or-ip>)"
PATH="$WRAPPER_DIR/bin:$PATH" ssh <wrapper-host> 'hostname'
```

If the target behind the hop chain needs password authentication, supply the resolved robot credentials (`std` / `SRpasswd@2017` by default):

```bash
WRAPPER_DIR="$(.agents/skills/easygo-deploy-dev-hot-update/scripts/make_nested_ssh_wrapper.sh --target-user "$SSH_USER" --target-pass "$SSH_PASS" <wrapper-host> <hop1> [<hop2> ...] -- <target-ip-or-alias>)"
PATH="$WRAPPER_DIR/bin:$PATH" ssh <wrapper-host> 'hostname'
```

Set `DEPLOY_HOST` to `<wrapper-host>` and prefix deployment and verification commands with `PATH="$WRAPPER_DIR/bin:$PATH"` so they use the same route. The helper supports `ssh` and the local-file-to-target-path `scp` shape used by `deploy-dev.py`.

The legacy one-hop form is also supported:

```bash
WRAPPER_DIR="$(.agents/skills/easygo-deploy-dev-hot-update/scripts/make_nested_ssh_wrapper.sh <wrapper-host> <hop1> <target-alias-or-ip>)"
```

Remove the temporary wrapper directory when the deployment workflow finishes.
