# mcproc

A Model Context Protocol (MCP) server for comfortable background process management on AI agents.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Homebrew](https://img.shields.io/badge/Homebrew-tap%2Fneptaco-orange)](https://github.com/neptaco/homebrew-tap)

[English](README.md) | [日本語](README.ja.md)

## Overview

mcproc bridges the gap between AI agent development and traditional command-line workflows. It enables AI agents to manage long-running development processes (like dev servers, build watchers, etc.) while providing developers with full CLI access to monitor and control these same processes.

## Why mcproc?

Simple AI agent-launched processes are stateless and can't manage long-running processes effectively. mcproc solves this by:

- **Unified Control**: No more confusion about which agent or terminal is running what - all processes are centrally managed
- **Context Preservation**: Logs are captured and stored, allowing AI agents to debug issues while reviewing logs from earlier
- **Developer-Friendly**: Full CLI access means you're never locked out of your own development environment

## Key Features

- 🔄 **Unified Process Management**: Start and manage background processes from AI agents via MCP, then monitor them from your terminal
- 👁️ **Cross-Environment Visibility**: Processes started by AI agents are fully accessible via CLI and other agents, and vice versa
- 📝 **Intelligent Log Management**: Capture, persist, and search process logs with powerful regex patterns
- 📁 **Project-Aware**: Automatically groups processes by project context
- 📊 **Real-time Monitoring**: Follow logs in real-time from CLI while AI agents manage the processes
- 🛡️ **XDG Compliant**: Follows XDG Base Directory specification for proper file organization
- ⚡ **Wait-for-Log**: Start processes and wait for specific log patterns to ensure readiness
- 🔍 **Advanced Search**: Time-based filtering, context lines, and regex support for log analysis
- 🧰 **Toolchain Support**: Execute commands through version managers (mise, asdf, nvm, rbenv, etc.)
- 🧹 **Clean Command**: Stop all processes in a project with a single command
- 🌲 **Process Groups**: Automatic cleanup of child processes when stopping parent processes

## Installation

### Using Homebrew (macOS and Linux)

```bash
# Add the tap
brew tap neptaco/tap

# Install mcproc
brew install mcproc
```

### Build from source

#### Prerequisites

- Rust toolchain (rustc, cargo)
- protobuf compiler:
  - macOS: `brew install protobuf`
  - Linux: `apt-get install protobuf-compiler`

```bash
git clone https://github.com/neptaco/mcproc.git
cd mcproc
cargo build --release

# Install to PATH (optional)
cargo install --path mcproc
```

## Usage

### Setup as MCP Server

After installing mcproc, you need to register it as an MCP server with your AI assistant.

#### For Claude Code

```bash
# Register mcproc as an MCP server
claude mcp add mcproc mcproc mcp serve
```

#### For Other MCP Clients

Configure your MCP client by adding mcproc to your configuration:

```json
{
  "mcpServers": {
    "mcproc": {
      "command": "mcproc",
      "args": ["mcp", "serve"]
    }
  }
}
```

### Available MCP Tools

Once registered, AI agents can use these tools:

- `start_process`: Start a development server or background process
- `stop_process`: Stop a running process
- `restart_process`: Restart a process
- `list_processes`: List all running processes
- `get_process_logs`: Retrieve process logs
- `search_process_logs`: Search through process logs with pattern matching
- `get_process_status`: Get detailed process information

### For Developers (CLI)

While AI agents manage processes in the background, you can monitor and control them:

Recommended command: `mcproc logs -f`

#### CLI Commands

| Command | Description | Flags | Example |
|---------|-------------|-------|---------|
| 🗒️ `ps` | List all running processes | `-s, --status <STATUS>` Filter by status | `mcproc ps --status running` |
| 🚀 `start **<NAME>**` | Start a new process | `-c, --cmd <CMD>` Command to run<br>`-d, --cwd <DIR>` Working directory<br>`-e, --env <KEY=VAL>` Environment variables<br>`-p, --project <NAME>` Project name<br>`--wait-for-log <PATTERN>` Wait for log pattern<br>`--wait-timeout <SECS>` Wait timeout<br>`--toolchain <TOOL>` Version manager to use | `mcproc start web -c "npm run dev" -d ./app` |
| 🛑 `stop **<NAME>**` | Stop a running process | `-p, --project <NAME>` Project name<br>`-f, --force` Force kill (SIGKILL) | `mcproc stop web -p myapp` |
| 🔄 `restart **<NAME>**` | Restart a process | `-p, --project <NAME>` Project name | `mcproc restart web` |
| 📜 `logs **<NAME>**` | View process logs | `-p, --project <NAME>` Project name<br>`-f, --follow` Follow log output<br>`-t, --tail <NUM>` Number of lines to show | `mcproc logs web -f -t 100` |
| 🔍 `grep **<NAME>** **<PATTERN>**` | Search logs with regex | `-p, --project <NAME>` Project name<br>`-C, --context <NUM>` Context lines<br>`-B, --before <NUM>` Lines before match<br>`-A, --after <NUM>` Lines after match<br>`--since <TIME>` Search since time<br>`--until <TIME>` Search until time<br>`--last <DURATION>` Search last duration | `mcproc grep web "error" -C 3` |
| 🧹 `clean` | Stop all processes in project | `-p, --project <NAME>` Project name<br>`-f, --force` Force kill | `mcproc clean -p myapp` |
| 🎛️ `daemon start` | Start mcproc daemon | None | `mcproc daemon start` |
| 🎛️ `daemon stop` | Stop mcproc daemon | None | `mcproc daemon stop` |
| 🎛️ `daemon status` | Check daemon status | None | `mcproc daemon status` |
| 🔌 `mcp serve` | Run as MCP server | None | `mcproc mcp serve` |
| ℹ️ `--version` | Show version info | None | `mcproc --version` |
| ❓ `--help` | Show help message | None | `mcproc --help` |


#### Examples

```bash
# Start the daemon (if not already running)
mcproc daemon start

# View all processes (including those started by AI agents)
mcproc ps

# Follow logs in real-time
mcproc logs frontend -f

# Multi-process log streaming per project
mcproc logs -f

# Search through logs
mcproc grep backend "error" -C 5

# Stop a process
mcproc stop frontend
```

### Example Workflow

1. **AI agent starts your development server:**
   ```
   Agent: "I'll start the frontend dev server for you"
   → Uses MCP tool: start_process(name: "frontend", cmd: "npm run dev", wait_for_log: "Server running")
   ```

2. **You monitor it from terminal:**
   ```bash
   mcproc logs -f
   # See real-time logs as the server runs
   ```

3. **AI agent detects an error and searches logs:**
   ```
   Agent: "Let me check what's causing that error"
   → Uses MCP tool: search_process_logs(name: "frontend", pattern: "ERROR|WARN", last: "5m")
   ```

4. **You can see the same information:**
   ```bash
   mcproc grep frontend "ERROR|WARN" -C 3 --last 5m
   ```

### Advanced Examples

```bash
# Start a process with environment variables
mcproc start api --cmd "python app.py" --env PORT=8000 --env DEBUG=true

# Wait for a specific log pattern before considering the process ready
mcproc start web --cmd "npm run dev" --wait-for-log "Server running on" --wait-timeout 60

# Search logs with time filters
mcproc grep api "database.*connection" --since "14:30" --until "15:00"

# View logs from multiple processes in the same project
mcproc ps
mcproc logs web --project myapp -t 100

# Use version managers for Node.js projects
mcproc start web --cmd "npm run dev" --toolchain nvm
mcproc start api --cmd "yarn start" --toolchain mise

# Clean up all processes in a project
mcproc clean --project myapp

# Force stop all processes in current project
mcproc clean --force
```

## Architecture

mcproc consists of three main components:

1. **mcproc daemon**: A lightweight daemon that manages processes and handles log persistence
2. **mcproc CLI**: Command-line interface for developers to interact with the daemon
3. **MCP Server**: Exposes process management capabilities to AI agents via the Model Context Protocol

### File Locations (XDG Compliant)

- **Config**: `$XDG_CONFIG_HOME/mcproc/config.toml` (defaults to `~/.config/mcproc/`)
- **Logs**: `$XDG_STATE_HOME/mcproc/log/` (defaults to `~/.local/state/mcproc/log/`)
- **Runtime**: `$XDG_RUNTIME_DIR/mcproc/` (defaults to `/tmp/mcproc-$UID/`)

## Development

### Building from Source

```bash
# Clone the repository
git clone https://github.com/neptaco/mcproc.git
cd mcproc

# Build all components
cargo build --release

# Run tests
cargo test

# Run with verbose logging
RUST_LOG=mcproc=debug cargo run -- daemon start
```

### Project Structure

```
mcproc/
├── mcproc/         # CLI and daemon implementation
├── mcp-rs/         # Reusable MCP server library
├── proto/          # Protocol buffer definitions
└── docs/           # Architecture and design documentation
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

MIT License

Copyright (c) 2025 Atsuhito Machida (neptaco)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.