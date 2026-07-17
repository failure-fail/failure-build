# Installing Failure Build

Failure Build (`failure`) is a terminal-based AI coding agent, forked from
xAI's open-sourced Grok Build. It's bring-your-own-provider: x.ai's Grok
models, OpenAI, Anthropic, Ollama, or any custom OpenAI-compatible endpoint
(including a local server on your own network).

All platform binaries are published on the
[GitHub Releases page](https://github.com/failure-fail/failure-build/releases/latest).

---

## Linux (x86_64 / arm64)

```sh
curl -fsSL https://raw.githubusercontent.com/failure-fail/failure-build/main/crates/codegen/xai-grok-pager/scripts/install.sh | bash
```

Or manually, picking the right asset for your CPU:

```sh
# x86_64
curl -fL --progress-bar -o failure "https://github.com/failure-fail/failure-build/releases/latest/download/failure-<version>-linux-x86_64"
# arm64
curl -fL --progress-bar -o failure "https://github.com/failure-fail/failure-build/releases/latest/download/failure-<version>-linux-aarch64"

chmod +x failure
./failure
```

Or via npm (any platform with Node.js):

```sh
npm i -g @failure-build/failure
```

---

## macOS (Apple Silicon / arm64)

Same installer script as Linux:

```sh
curl -fsSL https://raw.githubusercontent.com/failure-fail/failure-build/main/crates/codegen/xai-grok-pager/scripts/install.sh | bash
```

Or manually:

```sh
curl -fL --progress-bar -o failure "https://github.com/failure-fail/failure-build/releases/latest/download/failure-<version>-macos-aarch64"
chmod +x failure
./failure
```

Or via npm:

```sh
npm i -g @failure-build/failure
```

**Intel Macs (x86_64):** not built by the release pipeline (GitHub no longer
reliably provisions hosted Intel Mac runners) — build from source instead
(see [Building from source](README.md#building-from-source) in the main
README).

---

## Windows (x86_64)

PowerShell:

```powershell
irm https://raw.githubusercontent.com/failure-fail/failure-build/main/crates/codegen/xai-grok-pager/scripts/install.ps1 | iex
```

Or manually:

```powershell
Invoke-WebRequest -Uri "https://github.com/failure-fail/failure-build/releases/latest/download/failure-<version>-windows-x86_64.exe" -OutFile failure.exe
.\failure.exe
```

Or via npm:

```powershell
npm i -g @failure-build/failure
```

---

## Android (via Termux)

Android has no general-purpose terminal, so there's no standalone `.apk` —
Failure Build runs inside [Termux](https://termux.dev/) instead, which
provides a real terminal plus a Linux userland.

### 1. Install Termux

Get it from **[F-Droid](https://f-droid.org/en/packages/com.termux/)** — not
the Play Store version, which is outdated and unmaintained.

### 2. One-time setup

```sh
pkg update && pkg install ripgrep git
mkdir -p ~/failure-app
```

### 3. Download and run

```sh
cd ~/failure-app
curl -fL --progress-bar --retry 10 --retry-delay 3 -o failure "https://github.com/failure-fail/failure-build/releases/latest/download/failure-<version>-android-aarch64"
chmod +x failure
./failure
```

(Replace `<version>` with the version from the
[latest release](https://github.com/failure-fail/failure-build/releases/latest),
e.g. `failure-0.1.220-alpha.4-android-aarch64`.)

After this first run, launch it again anytime with:

```sh
~/failure-app/failure
```

### Notes / known limitations on Android

- This is a native `aarch64-linux-android` build, distinct from the
  `linux-aarch64` build above (different C library — Bionic vs. glibc — not
  interchangeable).
- **No auto-updater support yet** for this target — `failure update` and the
  `curl | bash` installer don't know about it. To upgrade, just re-run the
  download command above with the new version number.
- **Clipboard and microphone dictation report "unavailable"** — they compile
  and run fine, they just don't do anything yet. Real Termux support for
  these (via `termux-clipboard-get/set` and `termux-microphone-record`, from
  the separate Termux:API app) isn't wired up.
- If a download drops mid-transfer over a flaky connection, delete the
  partial file and retry:
  ```sh
  rm failure
  # then re-run the curl command above
  ```

### Using a custom / self-hosted provider on Android

If you want to point Failure Build at a custom OpenAI-compatible endpoint
(your own API, a local llama.cpp-style server on your LAN, etc.) instead of
x.ai/OpenAI/Anthropic/Ollama:

```sh
mkdir -p ~/.failure
cat >> ~/.failure/config.toml <<'EOF'
[provider.custom]
base_url = "https://your-endpoint.example.com/v1"

[model.your-model-name]
provider = "custom"
EOF

~/failure-app/failure login --provider custom --api-key YOUR_API_KEY
~/failure-app/failure --model your-model-name
```

For a **local server on your own Wi-Fi network** (e.g. something serving an
OpenAI-compatible API at `http://192.168.1.50:8080`), same idea — use its LAN
address as the `base_url` (usually with a `/v1` suffix), and pass any
placeholder string as the API key if the local server doesn't check one:

```sh
mkdir -p ~/.failure
cat >> ~/.failure/config.toml <<'EOF'
[provider.local]
base_url = "http://192.168.1.50:8080/v1"

[model.local]
provider = "local"
EOF

~/failure-app/failure login --provider local --api-key none
~/failure-app/failure --model local
```

Your phone must be on the same network as whatever's hosting that server.

---

## First launch (all platforms)

On first launch with no provider configured, Failure Build walks you through
picking one interactively (x.ai, OpenAI, Anthropic, Ollama, or custom). For
x.ai specifically, you can skip the picker with an API key from
[console.x.ai](https://console.x.ai):

```sh
export XAI_API_KEY="xai-..."
```

To use a **named BYOP provider** directly (bypassing the picker) on any
platform, store a key once:

```sh
failure login --provider openai --api-key sk-...
```

then launch normally — it remembers the choice.
