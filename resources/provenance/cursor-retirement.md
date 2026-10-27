# Cursor resource retirement

The supported clients are Codex and Kimi. Retire 19 Cursor-specific Skill copies, seven project `.cursor/rules` resources, and the Cursor section of the otherwise native Codex create-subagent guide. Several copies were incorrectly named Codex while requiring Cursor MCP, SDK, rules, hook events or hosting. Preserve client-independent workflows and the native Codex agent instructions.

`resources/retirements/cursor-native.json` is a review proposal for previously unmanaged Cursor-native Skill, plugin-cache and MCP entries. It records their original user scope, native manager, unknown enablement and exact file fingerprints without importing credentials or trust. It authorizes no installation-directory mutation until review and merge. Cursor chats, project runtime state, credentials, trust, editor preferences and extensions are outside this resource retirement.

Do not rewrite shared upstream packages: the ECC install executor and target registry import Cursor-named support modules even for Codex consumers, and Codex/Kimi plugin packages include unused adapters. They remain unchanged and do not establish Cursor entry points. The corresponding entry-point subscriptions are retired. Tracked project-rule sources also require retirement in their owning repositories so fresh checkouts do not restore them.

The EasyGo workspace-rule template and installer are also retired as one legacy project integration. Its AGENTS instruction is removed in both the owning repository and the SM source; the existing simulation safety requirements in AGENTS remain.
