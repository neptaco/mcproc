# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

mcproc is a Rust-based daemon that fulfills Model Context Protocol (MCP) tool calls from LLMs and manages multiple development processes. The project is currently in the architecture design phase.

## Key Architecture Components

### Core Components
- **mcprocd**: The main daemon process
  - Tool Registry for MCP tools
  - ProxyProc Manager for spawning/managing child processes
  - Log Hub with ring buffer and file persistence
  - API Layer using tonic (gRPC) and warp (HTTP/WebSocket)

- **mcproc**: CLI tool for interacting with the daemon
  - Communicates via gRPC-unix socket
  - Supports commands: start, stop, restart, ps, logs

### MCP Tools
The daemon exposes these tools to LLMs:
- `dev_proxy.start`: Spawn or attach to development processes
- `dev_proxy.stop`: Terminate proxy processes
- `dev_proxy.logs`: Fetch or stream process logs

## Prerequisites

- Rust toolchain (rustc, cargo)
- protobuf compiler: **REQUIRED** - Install with:
  - macOS: `brew install protobuf`
  - Linux: `apt-get install protobuf-compiler`
  - Without this, the build will fail with "Could not find `protoc`" error

## Development Commands

```bash
# Build
cargo build
cargo build --release

# Test
cargo test
cargo test -- --nocapture  # Show println! output

# Lint
cargo clippy -- -D warnings

# Format
cargo fmt
cargo fmt -- --check  # Check without modifying

# Run
cargo run --bin mcprocd  # Run daemon
cargo run --bin mcproc -- <command>  # Run CLI
```

## Project Structure

The intended structure follows a workspace layout:
- `mcprocd/` - Daemon crate with API servers and process management
- `mcproc/` - CLI crate for user interaction
- `proto/` - Protocol definitions for gRPC services

## Implementation Specifications

### Logging
- **Log retention**: 1 day
- **Max file size**: 10MB per log file
- **Ring buffer**: 10,000 lines in memory per process
- **Log directory**: `~/.mcp/log/`
- **Format**: `{process_name}-{date}.log`

### Process Management
- **Error recovery**: No automatic restart on crash, but maintain crash state for monitoring
- **Process isolation**: Each process runs independently
- **Resource limits**: Inherit from parent process

### Security
- **Local only**: No remote access support
- **Unix permissions**: 0600 for all mcproc files
- **No authentication**: Local user only

### MCP Integration
mcproc acts as an MCP server that receives JSON-RPC 2.0 requests from LLMs:

The mcprocd daemon communicates with MCP clients using its own HTTP API on port 3434.

#### MCP Transport Support in mcp-rs Library
The mcp-rs library provides transport implementations for creating MCP servers:
1. **stdio**: Standard input/output transport (implemented)
2. **sse**: Server-Sent Events transport (not yet implemented)
3. **streamable-http**: HTTP with Server-Sent Events (not yet implemented)

All transports follow the same JSON-RPC 2.0 message format and tool definitions.

Example MCP interactions:

```json
// Initialize session - POST /mcp
{
  "jsonrpc": "2.0",
  "method": "initialize",
  "id": 1
}

// List available tools - POST /mcp
{
  "jsonrpc": "2.0",
  "method": "tools/list",
  "id": 2
}

// Start a process - POST /mcp
{
  "jsonrpc": "2.0",
  "method": "dev_proxy.start",
  "id": 1,
  "params": {
    "name": "frontend",
    "cmd": "npm run dev",
    "cwd": "./webapp",
    "port": 5173
  }
}

// mcprocd responds:
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "id": "uuid-here",
    "name": "frontend",
    "pid": 12345,
    "status": "Running",
    "port": 5173
  }
}
```

### MCP Library (mcp-rs)

This project includes a reusable MCP library that can be used to create MCP servers easily:

```rust
use mcp_rs::{ServerBuilder, StdioTransport};

// Create server with stdio transport
let mut server = ServerBuilder::new("my-server", "1.0.0")
    .add_tool(Arc::new(MyTool))
    .build(Box::new(StdioTransport::new()))
    .await?;

// SSE and Streamable HTTP transports are not yet implemented
// When implemented, they will follow a similar pattern
```

## Current Status

Basic implementation complete. Remaining tasks:
- Unix Domain Socket support (currently TCP only)
- Log streaming functionality
- Process state persistence with SQLite
- Integration tests