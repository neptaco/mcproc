# mcproc Architecture

## 1. Purpose

A local daemon that fulfills the Model Context Protocol (MCP) tool calls from an LLM and manages multiple development processes.
Features:

* Spawn / restart processes on demand.
* Ring-buffer + file log persistence with immediate log file creation.
* Range fetch & real-time log streaming (gRPC-stream).
* CLI utilities and UNIX-friendly interoperability.
* Environment variable support for process configuration.

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

## 5. Sequence (mcproc_start)

1. JSON-RPC request arrives → validate params (`name`, `cmd` or `args`, `cwd`, `env`).
2. mcproc forwards request to mcprocd via gRPC.
3. Registry lookup: if running → return error (AlreadyExists).
4. Create log file immediately with startup information.
5. Spawn process:
   * If `cmd`: Execute via shell (`sh -c` on Unix, `cmd /C` on Windows)
   * If `args`: Direct execution without shell
6. Child stdout & stderr are piped → each line:
   * Appended to ring buffer
   * Written directly to log file with timestamp
7. Respond with process metadata (ID, PID, status, log_file).

## 6. MCP Tool Definitions

```json
{
  "tools": [
    {
      "name": "mcproc_start",
      "description": "Start a development server or process",
      "inputSchema": {
        "type": "object",
        "properties": {
          "name": { 
            "type": "string", 
            "description": "Unique name for this process" 
          },
          "cmd": { 
            "type": "string", 
            "description": "Command to execute with shell (e.g., 'npm run dev'). Use for commands with pipes, redirects, or shell features." 
          },
          "args": { 
            "type": "array", 
            "items": { "type": "string" },
            "description": "Command and arguments as array (e.g., ['npm', 'run', 'dev']). Use for direct execution without shell."
          },
          "cwd": { 
            "type": "string", 
            "description": "Working directory path" 
          },
          "env": { 
            "type": "object", 
            "description": "Environment variables",
            "additionalProperties": { "type": "string" }
          }
        },
        "required": ["name"],
        "oneOf": [
          { "required": ["cmd"] },
          { "required": ["args"] }
        ]
      }
    },
    {
      "name": "mcproc_stop",
      "description": "Stop a running process",
      "inputSchema": {
        "type": "object",
        "properties": {
          "name": { "type": "string" }
        },
        "required": ["name"]
      }
    },
    {
      "name": "mcproc_ps",
      "description": "List all managed processes",
      "inputSchema": {
        "type": "object",
        "properties": {}
      }
    },
    {
      "name": "mcproc_logs",
      "description": "View logs from a process",
      "inputSchema": {
        "type": "object",
        "properties": {
          "name": { "type": "string" },
          "from": { "type": "integer" },
          "to": { "type": "integer" },
          "follow": { "type": "boolean" }
        },
        "required": ["name"]
      }
    }
  ]
}
```

### Example MCP Tool Call

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "id": 1,
  "params": {
    "name": "mcproc_start",
    "arguments": {
      "name": "frontend",
      "cmd": "npm run dev",  // シェル実行の例
      "cwd": "./webapp",
      "env": {
        "PORT": "5173",
        "NODE_ENV": "development"
      }
    }
  }
}

// args配列を使用する例
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "id": 2,
  "params": {
    "name": "mcproc_start",
    "arguments": {
      "name": "backend",
      "args": ["python", "app.py", "--port", "8000"],
      "cwd": "./backend",
      "env": {
        "PYTHONPATH": "./src"
      }
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

The daemon uses dynamic port allocation and writes the port to `~/.mcproc/mcprocd.port`.

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
- Location: `~/.mcproc/log/{process_name}_{date}.log`
- Format: `YYYY-MM-DD HH:MM:SS.mmm [LEVEL] message`
- Immediate creation on process start with startup information
- Exit information logged when process terminates

### Direct File Access
```bash
# Get log file path
mcproc ps  # Shows log_file column

# Direct tail
tail -f ~/.mcproc/log/frontend_20250616.log
```

## 10. Configuration & Recovery

### Configuration
- Config file: `~/.mcproc/config.toml`
- Daemon PID file: `~/.mcproc/mcprocd.pid`
- Port file: `~/.mcproc/mcprocd.port`

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
* ✅ Full tool set (start, stop, ps, logs) exposed to LLMs
* ✅ Rust project structure (mcprocd, mcproc, proto, mcp-rs)
* ✅ Process spawn with stdout/stderr capture
* ✅ Log persistence with immediate file creation
* ✅ gRPC streaming for log access
* ✅ Complete CLI with daemon management
* ✅ Environment variable support for processes
* ✅ Automatic daemon startup
* ✅ Process lifecycle management with proper signal handling

**Future Enhancements**

* [ ] Web UI for process monitoring
* [ ] Process resource usage tracking
* [ ] Log rotation and cleanup policies
* [ ] Process group management
* [ ] Remote daemon connection support