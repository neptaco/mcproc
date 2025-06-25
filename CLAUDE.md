# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

mcproc is a Rust-based daemon that fulfills Model Context Protocol (MCP) tool calls from LLMs and manages multiple development processes. The project provides a robust process management system with comprehensive logging, validation, and monitoring capabilities.

## Key Architecture Components

### High-Level Architecture

```
Claude / Other LLMs
      │  JSON-RPC 2.0 (MCP) via stdio
      ▼
┌───────────────────────────────┐
│   mcproc (CLI + MCP server)   │
│───────────────────────────────│
│  • MCP Tool Handlers          │
│  • gRPC Client                │
└───────────────────────────────┘
      │ gRPC (Unix Domain Socket)
      ▼
┌───────────────────────────────┐
│   mcproc daemon (--daemon)    │
│───────────────────────────────│
│  • Process Manager            │
│  • Log Hub (direct file I/O)  │
│  • gRPC API (tonic)           │
└───────────────────────────────┘
      │ stdout/stderr capture
      ▼
┌────────────────────────────────┐
│  Child Processes               │
│  (npm run dev, python app.py,  │
│   cargo run, etc.)             │
└────────────────────────────────┘
```

### Core Components
- **mcproc daemon**: The main daemon process (runs as `mcproc --daemon`)
  - Process Manager for spawning/managing child processes
  - Log Hub with ring buffer (10K lines) and file persistence
  - API Layer using tonic (gRPC via Unix Domain Socket)
  - Port detection and monitoring
  - Process state tracking (Starting, Running, Stopping, Stopped, Failed)

- **mcproc**: CLI tool for interacting with the daemon
  - Communicates via gRPC Unix socket
  - Supports commands: start, stop, restart, ps, logs, grep, clean, mcp serve
  - Project-based process organization
  - Auto-starts daemon if not running

- **mcp-rs**: Reusable MCP library
  - ServerBuilder for creating MCP servers
  - Transport implementations (stdio implemented, SSE/HTTP planned)
  - Tool registry and handler interfaces

### MCP Tools
The daemon exposes these tools to LLMs via `mcproc mcp serve`:
- `start_process`: Start and manage development processes
- `stop_process`: Terminate processes
- `restart_process`: Restart running processes
- `list_processes`: List all managed processes
- `get_logs`: Fetch or stream process logs
- `get_process_status`: Get detailed process information
- `search_process_logs`: Search through logs with regex patterns

## Prerequisites

- Rust toolchain (rustc, cargo)
- protobuf compiler: **REQUIRED** - Install with:
  - macOS: `brew install protobuf`
  - Linux: `apt-get install protobuf-compiler`
  - Without this, the build will fail with "Could not find `protoc`" error

## Development Commands

```bash
# Build - IMPORTANT: Always use --all-targets to check binaries
cargo build --all-targets  # Check all targets including binaries
cargo build --release --all-targets

# Test
cargo test
cargo test -- --nocapture  # Show println! output

# Lint - Include all targets
cargo clippy --all-targets -- -D warnings

# Format
cargo fmt
cargo fmt -- --check  # Check without modifying

# Check before install (detect binary compilation errors early)
cargo check --bin mcproc  # Check binary compilation
cargo install --path mcproc --dry-run  # Dry run to detect install errors

# Run
cargo run --bin mcproc -- --daemon  # Run daemon (hidden option)
cargo run --bin mcproc -- daemon start  # Run daemon (recommended)
cargo run --bin mcproc -- <command>  # Run CLI
```

### Pre-commit Checklist

**🚨 MANDATORY - NEVER commit without completing ALL steps below 🚨**

Run these checks in order. If ANY step fails, STOP and fix before proceeding:

```bash
# 1. Format check (MUST pass)
cargo fmt -- --check

# 2. Clippy (linting) - Include all targets (MUST pass with zero warnings)
cargo clippy --all-targets -- -D warnings

# 3. Build check - Include binaries (MUST compile successfully)
cargo build --all-targets

# 4. Tests (MUST pass)
cargo test

# 5. Binary check (ensure mcproc can be installed)
cargo check --bin mcproc

# 6. Security audit (review any issues)
cargo audit
```

**Verification command (all-in-one check):**
```bash
cargo fmt -- --check && \
cargo clippy --all-targets -- -D warnings && \
cargo build --all-targets && \
cargo test
```

**Failure Protocol:**
- If format check fails: Run `cargo fmt` then re-check
- If clippy fails: Fix ALL warnings before proceeding (no #[allow] without permission)
- If build fails: Fix compilation errors
- If tests fail: Fix failing tests

**NO EXCEPTIONS**: Quality gates are mandatory for every commit, regardless of change size.

## Project Structure

The project uses a Cargo workspace with the following crates:
- `mcp-rs/` - Reusable MCP library for creating MCP servers
- `mcproc/` - Main crate containing both daemon and CLI
  - `src/daemon/` - Daemon implementation (process management, gRPC server)
  - `src/cli/` - CLI commands and MCP tools
  - `src/common/` - Shared utilities (config, validation, status)
  - `src/client/` - gRPC client for daemon communication
- `proto/` - Protocol buffer definitions for gRPC services

## Implementation Specifications

### Logging
- **Log retention**: 7 days (configurable)
- **Max file size**: 50MB per log file (configurable)
- **Ring buffer**: 10,000 lines in memory per process
- **Log directory**: `$XDG_STATE_HOME/mcproc/log/{project}/` (defaults to `~/.local/state/mcproc/log/{project}/`)
- **Format**: `{process_name}.log` (organized by project directory)
- **Daemon log**: `$XDG_STATE_HOME/mcproc/log/mcprocd.log` (only when started via `mcproc daemon start`)
- **Features**: 
  - Real-time log streaming with follow mode
  - Regex-based log searching with context
  - Time-based filtering (since/until/last)
  - Process restart detection and seamless log continuation

#### ANSI Color Code Handling
- **Log Storage**: ANSI escape codes are preserved in log files and daemon memory
- **CLI Display**: Colors are shown by default, can be disabled with `--no-color` flag
- **MCP Tools**: ANSI codes are automatically stripped from all log output to ensure clean text for LLMs (for context saving and preventing misinterpretation)
  - Affected tools: `start_process`, `restart_process`, `get_process_logs`, `search_process_logs`
  - Stripped fields: log entries, `log_context`, `matched_line`
- **Rationale**: Processes are not connected to a TTY, so many tools disable colors by default. The `start_process` tool description includes guidance for enabling colors via tool-specific flags or environment variables

### Process Management
- **Error recovery**: No automatic restart on crash, but maintain crash state for monitoring
- **Process isolation**: Each process runs independently with separate log files
- **Resource limits**: Inherit from parent process
- **Organization**: Processes are grouped by project for better organization
- **Validation**: Process and project names are validated to ensure filesystem compatibility
- **Features**:
  - Port detection and monitoring
  - Process status tracking (Starting, Running, Stopping, Stopped, Failed)
  - Exit code and stderr capture on failure
  - Wait for log pattern on startup (with timeout)
  - Force restart option to replace running processes
  - Clean command to stop all processes in a project

#### Process Start Sequence
1. Validate parameters (name, cmd/args, project)
2. Check if process already running → return error if exists
3. Create log file immediately with startup information
4. Spawn process:
   - If `cmd`: Execute via shell (`sh -c` on Unix)
   - If `args`: Direct execution without shell
5. Pipe stdout/stderr → ring buffer + log file
6. If `wait_for_log` provided, wait for pattern match
7. Return process info (ID, PID, status, log_file)

#### MCP Design Philosophy
**IMPORTANT**: For MCP tools, process startup failures (like "command not found") are considered part of normal operation and should return ProcessInfo with appropriate status, not an error. This allows LLMs to:
- See the failed status and exit code
- Access stderr logs to understand what went wrong  
- Make informed decisions about next steps

Only return errors for:
- Process already exists with the same name
- Invalid parameters (validation failures)
- System-level failures (daemon not running, etc.)

### Security
- **Local only**: No remote access support
- **Unix permissions**: 0600 for all mcproc files
- **No authentication**: Local user only

### XDG Base Directory Specification
mcproc follows the XDG Base Directory specification:
- **Config files**: `$XDG_CONFIG_HOME/mcproc/` (defaults to `~/.config/mcproc/`)
- **Data files**: `$XDG_DATA_HOME/mcproc/` (defaults to `~/.local/share/mcproc/`)
- **State files**: `$XDG_STATE_HOME/mcproc/` (defaults to `~/.local/state/mcproc/`)
- **Runtime files**: `$XDG_RUNTIME_DIR/mcproc/` (defaults to `/tmp/mcproc-$UID/`)

File locations:
- Config file: `$XDG_CONFIG_HOME/mcproc/config.toml`
- Log files: `$XDG_STATE_HOME/mcproc/log/`
- Socket file: `$XDG_RUNTIME_DIR/mcproc/mcprocd.sock`
- PID file: `$XDG_RUNTIME_DIR/mcproc/mcprocd.pid`

### MCP Integration
mcproc acts as an MCP server that receives JSON-RPC 2.0 requests from LLMs via stdio transport:

```bash
# Start MCP server
mcproc mcp serve [--project <default-project>]
```

#### MCP Transport Support in mcp-rs Library
The mcp-rs library provides transport implementations for creating MCP servers:
1. **stdio**: Standard input/output transport (implemented)
2. **sse**: Server-Sent Events transport (not yet implemented)
3. **streamable-http**: HTTP with Server-Sent Events (not yet implemented)

All transports follow the same JSON-RPC 2.0 message format and tool definitions.

Example MCP tool call:

```json
// Start a process via MCP
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "id": 1,
  "params": {
    "name": "start_process",
    "arguments": {
      "name": "frontend",
      "cmd": "npm run dev",
      "project": "myapp",
      "wait_for_log": "Server running on",
      "wait_timeout": 30
    }
  }
}
```

See tool documentation for available tools and their parameters.

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

Core functionality is implemented and working:
- ✅ Unix Domain Socket support for gRPC communication
- ✅ Real-time log streaming with follow mode
- ✅ Process management with status tracking
- ✅ Project-based organization
- ✅ MCP server implementation with stdio transport
- ✅ Comprehensive validation for process and project names
- ✅ Port detection and monitoring
- ✅ Log searching with regex and time filters

Remaining enhancements:
- Process state persistence (currently in-memory only)
- Additional MCP transports (SSE, HTTP)
- Integration tests
- Web UI for process monitoring

## CLI Commands

### Process Management
- `mcproc start <name> --cmd <command>` - Start a new process
- `mcproc stop <name>` - Stop a running process
- `mcproc restart <name>` - Restart a process
- `mcproc ps` - List all processes
- `mcproc clean [--project <name>]` - Stop all processes in a project

### Log Management
- `mcproc logs <name> [--tail N] [--follow]` - View process logs
- `mcproc grep <name> <pattern>` - Search logs with regex
  - `--since`, `--until`, `--last` for time filtering
  - `--context`, `--before`, `--after` for context lines

### MCP Server
- `mcproc mcp serve [--project <default>]` - Start MCP server for LLM integration

### Options
- `--project` - Specify project (defaults to current directory name)
- Environment variable `MCPROC_DEFAULT_PROJECT` can set default project

## Validation Rules

### Process Name Validation
- Cannot be empty or consist of dots (`.`, `..`)
- No path separators (`/`, `\`)
- No special characters (`:`, `*`, `?`, `"`, `<`, `>`, `|`)
- No leading/trailing whitespace
- No control characters
- Maximum 100 characters

### Project Name Validation
- Same rules as process names plus:
- No Windows reserved names (CON, PRN, AUX, etc.)
- Maximum 255 characters (filesystem limit)

## Critical Development Rules

### No Hardcoded Paths
- **NEVER hardcode absolute paths, especially user home directories**
  - ❌ Bad: `/Users/username/.mcproc/log/`
  - ✅ Good: `config.log.dir.join("filename.log")`
- Always retrieve paths dynamically from configuration
- Use cross-platform APIs like `dirs::home_dir()`
- This applies to both code AND documentation - never expose user information

### Code Cleanup Policy
- This project is NOT a library - it's a standalone application
- Remove unused code instead of marking it with `#[allow(dead_code)]`
- Don't worry about "public API compatibility" - only keep code that is actually used
- Prefer deletion over deprecation for internal functions

### Preventing Binary Build Errors
- **Problem**: Regular `cargo build` only builds libraries, missing compilation errors in binaries (CLI)
- **Solution**: Always use `--all-targets` option when building
- **Reason**: Code under `mcproc/src/cli/` is only compiled during binary builds
- **Recommendation**: Always run `cargo build --all-targets` after changes to catch errors early

## Code Quality Rules

### Mandatory Pre-commit Process
**NEVER commit code without running the complete pre-commit checklist.** This is non-negotiable.

1. **Run lint checks immediately after any significant code changes**
2. **Never skip format/lint checks - even for "small" changes**
3. **Fix all lint warnings before proceeding with further development**

### Lint Warning Policy
- **Primary approach**: Fix the root cause of lint warnings through proper refactoring
- **#[allow] usage**: PROHIBITED without explicit user permission
- **When #[allow] might be appropriate**:
  - Generated code (like protobuf) where refactoring is not feasible
  - Temporary compatibility workarounds with clear timeline for removal
  - Cases where the lint is demonstrably incorrect for the specific context
- **Process for #[allow] usage**:
  1. Attempt proper refactoring first
  2. Document why refactoring is not feasible
  3. Request explicit user permission with justification
  4. Include TODO comment with removal plan if temporary

### Continuous Quality Checks
Run these checks at key development milestones:
- After implementing any new function or module
- Before switching to a different task
- After resolving any compilation errors
- Immediately before committing

### Quality-First Development
- Treat lint warnings as compilation errors
- Prioritize code quality equally with functionality
- Never rationalize skipping quality checks due to "time constraints"