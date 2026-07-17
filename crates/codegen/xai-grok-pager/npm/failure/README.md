# Failure Build

Bring Failure Build into your terminal. Fast, flicker-free CLI built for plans, subagents, and parallel work.

**[Repository](https://github.com/failure-fail/failure-build)**

## Install

```bash
npm i -g @failure-build/failure
```

(A standalone `curl | bash` installer is planned but not yet hosted for this
fork — see the repository's README for build-from-source instructions in
the meantime.)

## Get Started

```bash
# Launch the interactive TUI
failure

# Run a single task
failure -p "Explain this codebase"
```

Failure Build supports bringing your own inference provider (OpenAI,
Anthropic, Ollama, or a custom OpenAI-compatible endpoint) as well as
x.ai's own Grok models. On first launch with no provider configured, it
walks you through picking one. For x.ai specifically, you can also use an
API key from [console.x.ai](https://console.x.ai):

```bash
export XAI_API_KEY="xai-..."
```

## Remote MCP control

The npm launcher automatically starts a Streamable HTTP MCP bridge whenever
Failure is running interactively. The bridge exposes Failure's built-in ACP
agent API, so clients such as Claude can create chats, load existing chats,
send prompts, and let Failure use its normal coding, file, terminal, search,
and subagent tools.

The local endpoint and generated access token are printed on startup and saved
to:

```text
~/.failure/mcp.json
```

The state file contains a ready-to-paste URL similar to:

```text
http://127.0.0.1:2420/mcp?token=<generated-token>
```

When `cloudflared` is installed, Failure also starts a Cloudflare Quick Tunnel
and writes a temporary public HTTPS endpoint into the same state file. Paste
that public URL into a remote MCP client such as Claude.

The bridge exposes these MCP tools:

- `failure_new_chat`
- `failure_continue_chat`
- `failure_send_message`
- `failure_list_sessions`
- `failure_status`
- `failure_rpc` for direct access to any supported ACP JSON-RPC method

The generated token is required for both local and public requests. Treat the
URL as a password: remote callers can direct Failure to edit files and execute
commands through the agent.

Configuration:

```bash
# Disable the bridge completely
FAILURE_MCP_ENABLED=0 failure

# Keep MCP local and disable the public tunnel
FAILURE_MCP_TUNNEL=0 failure

# Change the local port
FAILURE_MCP_PORT=9000 failure

# Use a fixed token instead of an automatically generated one
FAILURE_MCP_TOKEN="your-secret" failure

# Override the cloudflared executable
CLOUDFLARED_BIN="/path/to/cloudflared" failure
```

Utility commands such as `failure sessions`, `failure update`, and
`failure models` do not start the bridge. The bridge exits automatically when
the Failure process exits.

## Update

```bash
failure update
```

Or if installed via npm:

```bash
npm i -g @failure-build/failure@latest
```

## Supported Platforms

| Platform | Architecture |
|---|---|
| macOS | Apple Silicon (arm64) |
| Linux | x86_64, arm64 |
| Windows | x86_64 |

## Documentation

See [`docs/user-guide`](https://github.com/failure-fail/failure-build/tree/main/crates/codegen/xai-grok-pager/docs/user-guide)
in the repository for configuration, MCP servers, custom providers/models,
headless mode, agent mode, and more.

## Feedback

Run `/feedback` inside Failure Build to report issues or send feedback directly.
