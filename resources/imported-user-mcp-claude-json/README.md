# Native MCP configuration fragment

This local configuration snapshot manages only the named native file's MCP section, at its existing scope and path. Credentials remain local through `${SM_LOCAL}` leaves. It is separate from rules, hooks, and other clients so missing credentials block only this MCP resource. No client configuration is rewritten during capture.
