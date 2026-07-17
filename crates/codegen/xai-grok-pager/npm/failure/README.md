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
