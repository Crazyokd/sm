<!-- CODEGRAPH_START -->
## CodeGraph

This project has a CodeGraph MCP server (`codegraph_*` tools) configured. CodeGraph is a tree-sitter-parsed knowledge graph of every symbol, edge, and file. Reads are sub-millisecond and return structural information grep cannot.

### When to prefer codegraph over native search

Use codegraph for **structural** questions — what calls what, what would break, where is X defined, what is X's signature. Use native grep/read only for **literal text** queries (string contents, comments, log messages) or after you already have a specific file open.

| Question | Tool |
|---|---|
| "Where is X defined?" / "Find symbol named X" | `codegraph_search` |
| "What calls function Y?" | `codegraph_callers` |
| "What does Y call?" | `codegraph_callees` |
| "How does X reach/become Y? / trace the flow from X to Y" | `codegraph_trace` (one call = the whole path, incl. callback/React/JSX dynamic hops) |
| "What would break if I changed Z?" | `codegraph_impact` |
| "Show me Y's signature / source / docstring" | `codegraph_node` |
| "Give me focused context for a task/area" | `codegraph_context` |
| "See several related symbols' source at once" | `codegraph_explore` |
| "What files exist under path/" | `codegraph_files` |
| "Is the index healthy?" | `codegraph_status` |

### Rules of thumb

- **Answer directly — don't delegate exploration.** For "how does X work" / architecture questions, answer with 2-3 codegraph calls: `codegraph_context` first, then ONE `codegraph_explore` for the source of the symbols it surfaces. For a specific **flow** ("how does X reach Y") start with `codegraph_trace` from→to — one call returns the whole path with dynamic hops bridged — then ONE `codegraph_explore` for the bodies; don't rebuild the path with `codegraph_search` + `codegraph_callers`. Codegraph IS the pre-built index, so spawning a separate file-reading sub-task/agent — or running a grep + read loop — repeats work codegraph already did and costs more for the same answer.
- **Trust codegraph results.** They come from a full AST parse. Do NOT re-verify them with grep — that's slower, less accurate, and wastes context.
- **Don't grep first** when looking up a symbol by name. `codegraph_search` is faster and returns kind + location + signature in one call.
- **Don't chain `codegraph_search` + `codegraph_node`** when you just want context — `codegraph_context` is one call.
- **Don't loop `codegraph_node` over many symbols** — one `codegraph_explore` call returns several symbols' source grouped in a single capped call, while each separate node/Read call re-reads the whole context and costs far more.
- **Index lag — check the staleness banner, don't guess a wait.** When a codegraph response starts with "⚠️ Some files referenced below were edited since the last index sync…", the listed files are pending re-index — Read those specific files for accurate content. Files NOT in that banner are fresh and codegraph is authoritative for them. `codegraph_status` also lists pending files under "Pending sync".

### Worktrees

- When creating a worktree manually, prefer `cg-worktree <branch> [path]` (= `git worktree add` + automatic index bootstrap).
- After ANY worktree is created (native tool, superpowers, or manual), run `cg-worktree-index <path>`: it copies the main checkout's index as a base and `codegraph sync`s the branch delta, giving the worktree a branch-accurate index in seconds. Idempotent; safe to re-run.
- Until then, codegraph inside a worktree silently falls back to the main checkout's index ("different git working tree" warning) — its results describe `main`, not your branch. Use them for orientation only; trust actual file reads.

### If `.codegraph/` doesn't exist

The MCP server returns "not initialized." Ask the user: *"I notice this project doesn't have CodeGraph initialized. Want me to run `codegraph init -i` to build the index?"*
<!-- CODEGRAPH_END -->

## AI 资源管理

- 用户级和项目级 Skill、Hook、规则、Agent、提示词、执行策略、MCP 和插件声明统一通过 SM 管理。先在 `~/repo/sm` 运行 `./sm inventory`，确认来源、作用域和现有管理者。
- 新建或修改自维护资源应修改 SM 的源文件并提交 MR；不要直接修改已经生效的安装目录。审核合并后，再由下一次拉取和同步应用。
- 上游资源使用登记的原生安装器跟随上游；调整订阅或启停声明走 MR，不直接改造上游副本，也不把上游内容纳入自维护优化。
- 新发现的未纳管资源先建立接管提案，保留其现有作用域和禁用状态。凭据、原生信任状态、会话及运行数据留在本机。
