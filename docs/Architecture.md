# mcproc Architecture

## 1. Purpose

A local daemon that fulfills the Model Context Protocol (MCP) tool calls from an LLM and manages multiple development processes.
Features:

* Spawn / restart processes on demand.
* Ring-buffer + file log persistence with immediate log file creation.
* Range fetch & real-time log streaming (gRPC-stream).
* CLI utilities and UNIX-friendly interoperability.
* Environment variable support for process configuration.
* Full XDG Base Directory Specification compliance.
* Advanced log search capabilities with regex patterns and time filtering.

## 2. High-Level Topology

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
      │ gRPC (dynamic port)
      ▼
┌───────────────────────────────┐
│      mcprocd  (Rust daemon)   │
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

## 3. Core Components & Crates

### Crates Structure
- `mcproc/` - CLI and MCP server implementation
- `mcprocd/` - Daemon process manager
- `proto/` - Protocol buffer definitions
- `mcp-rs/` - MCP protocol library

### Dependencies
| Concern             | Crate                            |
| ------------------- | -------------------------------- |
| Async runtime       | `tokio`                          |
| JSON-RPC (MCP)      | Custom implementation in mcp-rs  |
| gRPC / streaming    | `tonic`                          |
| Process spawning    | `tokio::process`                 |
| Concurrency map     | `dashmap`                        |
| Ring buffer         | `ringbuf`                        |
| File logging        | Direct async file I/O            |
| CLI                 | `clap`                           |
| Tracing             | `tracing` + `tracing-subscriber` |

## 4. Data Structures

```rust
/// Metadata kept per managed process
struct ProxyInfo {
    id: Uuid,
    name: String,
    cmd: String,
    cwd: PathBuf,
    start_time: DateTime<Utc>,
    status: Arc<AtomicU8>,  // ProcessStatus enum
    ring: Arc<Mutex<HeapRb<Vec<u8>>>>,
    log_file: PathBuf,
    pid: Option<u32>,
    child_handle: Option<tokio::process::Child>,
    project: Option<String>,
    wait_for_log: Option<String>,
    wait_timeout: Option<u32>,
}

/// Process states
enum ProcessStatus {
    Starting = 1,
    Running = 2,
    Stopping = 3,
    Stopped = 4,
    Failed = 5,
}
```

`DashMap<String, Arc<ProxyInfo>>` acts as the global registry keyed by `name`.

## 5. Sequence (start_process)

1. JSON-RPC request arrives → validate params (`name`, `cmd` or `args`, `cwd`, `env`, `project`, `wait_for_log`, `wait_timeout`).
2. mcproc forwards request to mcprocd via gRPC.
3. Registry lookup: if running → return error (AlreadyExists).
4. Create log file immediately with startup information in XDG state directory.
5. Spawn process:
   * If `cmd`: Execute via shell (`sh -c` on Unix, `cmd /C` on Windows)
   * If `args`: Direct execution without shell
6. Child stdout & stderr are piped → each line:
   * Appended to ring buffer
   * Written directly to log file with timestamp
7. If `wait_for_log` provided, wait for matching pattern in output (up to `wait_timeout` seconds).
8. Respond with process metadata (ID, PID, status, log_file).

## 6. MCP Tool Definitions

```json
{
  "tools": [
    {
      "name": "start_process",
      "description": "Start and manage a long-running development process",
      "inputSchema": {
        "type": "object",
        "properties": {
          "name": { 
            "type": "string", 
            "description": "Unique identifier for this process" 
          },
          "cmd": { 
            "type": "string", 
            "description": "Shell command to execute (e.g., 'npm run dev')" 
          },
          "args": { 
            "type": "array", 
            "items": { "type": "string" },
            "description": "Command and arguments as array for direct execution"
          },
          "cwd": { 
            "type": "string", 
            "description": "Working directory path" 
          },
          "env": { 
            "type": "object", 
            "description": "Environment variables",
            "additionalProperties": { "type": "string" }
          },
          "project": {
            "type": "string",
            "description": "Project name (defaults to directory name)"
          },
          "wait_for_log": {
            "type": "string",
            "description": "Regex pattern to wait for before considering process ready"
          },
          "wait_timeout": {
            "type": "integer",
            "description": "Timeout for log wait in seconds (default: 30)"
          }
        },
        "required": ["name"]
      }
    },
    {
      "name": "stop_process",
      "description": "Gracefully stop a running process",
      "inputSchema": {
        "type": "object",
        "properties": {
          "name": { "type": "string" },
          "project": { "type": "string" }
        },
        "required": ["name"]
      }
    },
    {
      "name": "restart_process",
      "description": "Restart a running process",
      "inputSchema": {
        "type": "object",
        "properties": {
          "name": { "type": "string" },
          "project": { "type": "string" }
        },
        "required": ["name"]
      }
    },
    {
      "name": "get_process_status",
      "description": "Get comprehensive status information for a process",
      "inputSchema": {
        "type": "object",
        "properties": {
          "name": { "type": "string" },
          "project": { "type": "string" }
        },
        "required": ["name"]
      }
    },
    {
      "name": "list_processes",
      "description": "List all managed processes",
      "inputSchema": {
        "type": "object",
        "properties": {}
      }
    },
    {
      "name": "get_process_logs",
      "description": "Retrieve console output from a process",
      "inputSchema": {
        "type": "object",
        "properties": {
          "name": { "type": "string" },
          "project": { "type": "string" },
          "tail": { "type": "integer" }
        },
        "required": ["name"]
      }
    },
    {
      "name": "search_process_logs",
      "description": "Search through process logs using regex",
      "inputSchema": {
        "type": "object",
        "properties": {
          "name": { "type": "string" },
          "pattern": { "type": "string" },
          "project": { "type": "string" },
          "context": { "type": "integer" },
          "before": { "type": "integer" },
          "after": { "type": "integer" },
          "since": { "type": "string" },
          "until": { "type": "string" },
          "last": { "type": "string" }
        },
        "required": ["name", "pattern"]
      }
    }
  ]
}
```

### Example MCP Tool Calls

```json
// Start a process with shell command
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "id": 1,
  "params": {
    "name": "start_process",
    "arguments": {
      "name": "frontend",
      "cmd": "npm run dev",
      "cwd": "./webapp",
      "env": {
        "PORT": "5173",
        "NODE_ENV": "development"
      },
      "wait_for_log": "Server running on",
      "wait_timeout": 60
    }
  }
}

// Search logs with regex
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "id": 2,
  "params": {
    "name": "search_process_logs",
    "arguments": {
      "name": "backend",
      "pattern": "ERROR|WARN",
      "context": 3,
      "last": "1h"
    }
  }
}
```

## 7. gRPC API (Internal Communication)

```protobuf
service ProcessManager {
  rpc StartProcess(StartProcessRequest) returns (StartProcessResponse);
  rpc StopProcess(StopProcessRequest) returns (StopProcessResponse);
  rpc RestartProcess(RestartProcessRequest) returns (RestartProcessResponse);
  rpc GetProcess(GetProcessRequest) returns (GetProcessResponse);
  rpc ListProcesses(ListProcessesRequest) returns (ListProcessesResponse);
  rpc GetLogs(GetLogsRequest) returns (stream GetLogsResponse);
}
```

The daemon uses dynamic port allocation and writes the port to `$XDG_RUNTIME_DIR/mcproc/mcprocd.port`.

## 8. CLI Usage

```bash
# Start a process with shell command
mcproc start frontend --cmd "npm run dev" --cwd ./webapp -e PORT=5173 -e NODE_ENV=development

# Start a process with args array (direct execution)
mcproc start backend --args python app.py --port 8000 --cwd ./backend

# List all processes
mcproc ps

# View logs (specific range)
mcproc logs frontend --from 100 --to 200

# Follow logs (real-time streaming)
mcproc logs frontend -f

# Stop a process
mcproc stop frontend

# Restart a process
mcproc restart frontend

# Daemon management
mcproc daemon start
mcproc daemon stop
mcproc daemon restart

# MCP server mode (for LLM integration)
mcproc mcp serve
```

## 9. Log Management

### Log File Structure
- Location: `$XDG_STATE_HOME/mcproc/log/{process_name}_{date}.log`
- Format: `HH:MM:SS.mmm message` (simplified for readability)
- Immediate creation on process start with startup information
- Exit information logged when process terminates
- Automatic log rotation at 10MB per file
- 1-day retention policy

### Direct File Access
```bash
# Get log file path
mcproc ps  # Shows log_file column

# Direct tail
tail -f ~/.local/state/mcproc/log/frontend_20250616.log

# Search logs
mcproc logs frontend --search "ERROR" --context 5
mcproc logs frontend --search "started.*port" --since "10:30" --until "11:00"
```

## 10. Configuration & Recovery

### Configuration (XDG Compliant)
- Config file: `$XDG_CONFIG_HOME/mcproc/config.toml`
- Log files: `$XDG_STATE_HOME/mcproc/log/`
- Socket file: `$XDG_RUNTIME_DIR/mcproc/mcprocd.sock`
- PID file: `$XDG_RUNTIME_DIR/mcproc/mcprocd.pid`
- Port file: `$XDG_RUNTIME_DIR/mcproc/mcprocd.port`

### Daemon Management
- Auto-start daemon if not running when CLI commands are used
- Graceful shutdown with SIGTERM to all child processes
- Process cleanup on daemon restart

### Process States
1. **Starting** - Process is being spawned
2. **Running** - Process is active and healthy
3. **Stopping** - SIGTERM sent, waiting for graceful shutdown
4. **Stopped** - Process has exited
5. **Failed** - Process exited with error

---

## 11. Implementation Status

**Completed Features**

* ✅ MCP protocol implementation with stdio transport
* ✅ Full tool set (start, stop, restart, status, logs, search) exposed to LLMs
* ✅ Rust project structure (mcprocd, mcproc, proto, mcp-rs)
* ✅ Process spawn with stdout/stderr capture
* ✅ Log persistence with immediate file creation
* ✅ gRPC streaming for log access
* ✅ Complete CLI with daemon management
* ✅ Environment variable support for processes
* ✅ Automatic daemon startup
* ✅ Process lifecycle management with proper signal handling
* ✅ XDG Base Directory Specification compliance
* ✅ Advanced log search with regex patterns and time filtering
* ✅ Wait-for-log pattern matching for process readiness
* ✅ Project-based process organization
* ✅ Log rotation at 10MB with 1-day retention

**Future Enhancements**

* [ ] Web UI for process monitoring
* [ ] Process resource usage tracking
* [ ] Unix Domain Socket support (currently TCP only)
* [ ] Process state persistence with SQLite
* [ ] Process group management
* [ ] Remote daemon connection support
* [ ] SSE and Streamable HTTP transports for MCP