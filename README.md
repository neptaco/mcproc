# mcproc

A Model Context Protocol (MCP) server for seamless background process management across AI agents and CLI environments.

## Overview

mcproc bridges the gap between AI-assisted development and traditional command-line workflows. It enables AI agents to manage long-running development processes (like dev servers, build watchers, etc.) while providing developers with full CLI access to monitor and control these same processes.

## Why mcproc?

Traditional MCP tools are stateless and can't manage long-running processes effectively. mcproc solves this by:

- **Persistent Process Management**: AI agents can start processes that continue running even after the conversation ends
- **Unified Control**: No more confusion about which terminal is running what - all processes are centrally managed
- **Context Preservation**: Logs are captured and stored, allowing AI agents to debug issues that happened earlier
- **Developer-Friendly**: Full CLI access means you're never locked out of your own development environment

## Key Features

- **Unified Process Management**: Start and manage background processes from AI agents via MCP, then monitor them from your terminal
- **Cross-Environment Visibility**: Processes started by AI agents are fully accessible via CLI, and vice versa
- **Intelligent Log Management**: Capture, persist, and search process logs with powerful grep functionality
- **Project-Aware**: Automatically shares process information within the same project context
- **Real-time Monitoring**: Follow logs in real-time from CLI while AI agents manage the processes
- **MCP-First Design**: Built specifically for the MCP ecosystem, making it easy for AI agents to handle complex development workflows

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

#### CLI Commands

| Command | Description | Flags | Example |
|---------|-------------|-------|---------|
| 🗒️ `ps` | List all running processes | `-j, --json` Output in JSON format<br>`-p, --project <NAME>` Filter by project | `mcproc ps -p myapp` |
| 🚀 `start **<NAME>**` | Start a new process | `-c, --cmd <CMD>` Command to run<br>`-d, --cwd <DIR>` Working directory<br>`-e, --env <KEY=VAL>` Environment variables<br>`-p, --project <NAME>` Project name<br>`-w, --wait-for <PATTERN>` Wait for log pattern<br>`-t, --timeout <SECS>` Wait timeout | `mcproc start web -c "npm run dev" -d ./app` |
| 🛑 `stop **<NAME>**` | Stop a running process | `-p, --project <NAME>` Project name<br>`-f, --force` Force kill (SIGKILL) | `mcproc stop web -p myapp` |
| 🔄 `restart **<NAME>**` | Restart a process | `-p, --project <NAME>` Project name | `mcproc restart web` |
| 📜 `logs **<NAME>**` | View process logs | `-p, --project <NAME>` Project name<br>`-f, --follow` Follow log output<br>`-n, --lines <NUM>` Number of lines to show<br>`--since <TIME>` Show logs since time<br>`--until <TIME>` Show logs until time<br>`--last <DURATION>` Show logs from last duration | `mcproc logs web -f --last 5m` |
| 🔍 `grep **<NAME>** **<PATTERN>**` | Search logs with regex | `-p, --project <NAME>` Project name<br>`-C, --context <NUM>` Context lines<br>`-i, --ignore-case` Case insensitive<br>`--since <TIME>` Search since time<br>`--until <TIME>` Search until time<br>`--last <DURATION>` Search last duration | `mcproc grep web "error" -C 3 -i` |
| 🎛️ `daemon start` | Start mcproc daemon | None | `mcproc daemon start` |
| 🎛️ `daemon stop` | Stop mcproc daemon | None | `mcproc daemon stop` |
| 🎛️ `daemon status` | Check daemon status | None | `mcproc daemon status` |
| 🔌 `mcp serve` | Run as MCP server | `--stdio` Use stdio transport (default)<br>`--project <NAME>` Set project context | `mcproc mcp serve --project myapp` |
| ℹ️ `--version` | Show version info | None | `mcproc --version` |
| ❓ `--help` | Show help message | None | `mcproc --help` |

#### Global Flags

| Flag | Description |
|------|-------------|
| `-r, --remote <ADDR>` | Remote mcprocd address (default: http://127.0.0.1:50051) |

#### Examples

```bash
# Start the daemon (if not already running)
mcproc daemon start

# View all processes (including those started by AI agents)
mcproc ps

# Follow logs in real-time
mcproc logs frontend -f

# Search through logs
mcproc grep backend "error" -C 5

# Stop a process
mcproc stop frontend
```

### Example Workflow

1. AI agent starts your development server:
   ```
   Agent: "I'll start the frontend dev server for you"
   → Uses MCP tool: start("frontend", "npm run dev")
   ```

2. You monitor it from terminal:
   ```bash
   mcproc logs frontend -f
   # See real-time logs as the server runs
   ```

3. AI agent detects an error and searches logs:
   ```
   Agent: "Let me check what's causing that error"
   → Uses MCP tool: grep("frontend", "Error.*failed")
   ```

4. You can see the same information:
   ```bash
   mcproc grep frontend "Error.*failed" -B 5 -A 5
   ```

## Project Structure

- `mcproc/`: Main CLI and daemon implementation
- `mcp-rs/`: Reusable MCP server library
- `proto/`: Protocol buffer definitions

## License

MIT

Copyright (c) 2025 Atsuhito Machida (neptaco)