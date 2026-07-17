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

The endpoint information and generated access token are saved to:

```text
~/.failure/mcp.json
```

The state file always contains a local URL:

```text
http://127.0.0.1:2420/mcp?token=<generated-token>
```

When `cloudflared` is installed, Failure also starts a Cloudflare Quick Tunnel
and records a temporary public URL.

### Stable Cloudflare Worker URL

Users can provide a Cloudflare API token and account ID once, then access
Failure through a stable `workers.dev` URL instead of copying a new Quick
Tunnel URL every launch.

Configure it interactively:

```bash
failure mcp-worker configure
```

Failure asks for:

- a Cloudflare API token with **Workers Scripts Write** permission
- the Cloudflare account ID
- a Worker name, defaulting to `failure-mcp`

The credentials are stored locally with owner-only permissions at:

```text
~/.failure/cloudflare-worker.json
```

After configuration, launch Failure normally:

```bash
failure
```

Failure will:

1. start the local MCP bridge
2. start a Cloudflare Quick Tunnel to the local bridge
3. create or update the configured Worker
4. point the Worker at the new tunnel origin
5. write a stable `workerUrl` into `~/.failure/mcp.json`

The resulting URL looks like:

```text
https://failure-mcp.<account-subdomain>.workers.dev/mcp?token=<generated-token>
```

Paste that `workerUrl` into Claude or another remote MCP client. The Worker URL
stays the same across launches while Failure updates its private upstream
origin automatically.

Other Worker commands:

```bash
# Show saved configuration with a masked API token
failure mcp-worker status

# Remove the local Worker configuration and token
failure mcp-worker disable
```

The bridge exposes these MCP tools:

- `failure_new_chat`
- `failure_continue_chat`
- `failure_send_message`
- `failure_list_sessions`
- `failure_status`
- `failure_rpc` for direct access to any supported ACP JSON-RPC method

The generated MCP token is required for local, Quick Tunnel, and Worker
requests. Treat the complete URL as a password: remote callers can direct
Failure to edit files and execute commands through the agent.

Configuration:

```bash
# Disable the bridge completely
FAILURE_MCP_ENABLED=0 failure

# Keep MCP local and disable all public access
FAILURE_MCP_TUNNEL=0 failure

# Change the local port
FAILURE_MCP_PORT=9000 failure

# Use a fixed MCP access token
FAILURE_MCP_TOKEN="your-secret" failure

# Supply Cloudflare configuration non-interactively
CLOUDFLARE_API_TOKEN="..." CLOUDFLARE_ACCOUNT_ID="..." failure mcp-worker configure

# Override the cloudflared executable
CLOUDFLARED_BIN="/path/to/cloudflared" failure
```

Utility commands such as `failure sessions`, `failure update`, and
`failure models` do not start the bridge. The bridge and Worker update daemon
exit automatically when the Failure process exits.

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
