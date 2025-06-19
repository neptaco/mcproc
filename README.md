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

### Prerequisites

- Rust toolchain (rustc, cargo)
- protobuf compiler:
  - macOS: `brew install protobuf`
  - Linux: `apt-get install protobuf-compiler`

### Build from source

```bash
git clone https://github.com/neptaco/mcproc.git
cd mcproc
cargo build --release
```

## Usage

### For MCP Clients (AI Agents)

Configure your MCP client to use mcproc:

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

Once configured, AI agents can use these tools:

- `start_process`: Start a development server or background process
- `stop_process`: Stop a running process
- `restart_process`: Restart a process
- `list_processes`: List all running processes
- `get_process_logs`: Retrieve process logs
- `search_process_logs`: Search through process logs with pattern matching
- `get_process_status`: Get detailed process information

### For Developers (CLI)

While AI agents manage processes in the background, you can monitor and control them:

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