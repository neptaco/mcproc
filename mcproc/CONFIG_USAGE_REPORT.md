# Config Field Usage Report

This report analyzes the usage of fields in the Config struct and its nested structs in the mcproc codebase.

## Config Structure

```rust
pub struct Config {
    pub daemon: DaemonConfig,
    pub log: LogConfig,
    pub api: ApiConfig,
    pub process: ProcessConfig,
}
```

## Field Usage Analysis

### DaemonConfig Fields

| Field | Type | Used | Where Used |
|-------|------|------|------------|
| `data_dir` | PathBuf | ✅ YES | - `daemon/config/mod.rs`: Used in `ensure_directories()` to create data directory |
| `pid_file` | PathBuf | ✅ YES | - `daemon/mod.rs`: Used to check if daemon is running, write PID, and clean up on shutdown<br>- `client/mod.rs`: Used to check daemon status<br>- `cli/commands/daemon.rs`: Used for daemon management |
| `socket_path` | PathBuf | ✅ YES | - `daemon/api/grpc.rs`: Used to bind Unix socket for gRPC server<br>- `client/mod.rs`: Used to connect to daemon |

### LogConfig Fields

| Field | Type | Used | Where Used |
|-------|------|------|------------|
| `dir` | PathBuf | ✅ YES | - `daemon/config/mod.rs`: Used in `ensure_directories()` to create log directory<br>- `daemon/log/mod.rs`: Used to construct log file paths<br>- `daemon/process/manager.rs`: Used to create log file paths<br>- `daemon/api/grpc.rs`: Used to construct log file paths for reading |
| `max_size_mb` | u64 | ❌ NO | Not used anywhere in the codebase |
| `max_files` | u32 | ❌ NO | Not used anywhere in the codebase |
| `ring_buffer_size` | usize | ❌ NO | Not used anywhere in the codebase |

### ApiConfig Fields

| Field | Type | Used | Where Used |
|-------|------|------|------------|
| `grpc_port` | u16 | ❌ NO | Not used - using Unix sockets instead of TCP |
| `unix_socket_permissions` | u32 | ✅ YES | - `daemon/api/grpc.rs`: Used to set permissions on Unix socket file |

### ProcessConfig Fields

| Field | Type | Used | Where Used |
|-------|------|------|------------|
| `max_restart_attempts` | u32 | ❌ NO | Not used anywhere in the codebase |
| `restart_delay_ms` | u64 | ✅ YES | - `daemon/process/manager.rs`: Used in `restart_process()` to wait between stop and start |
| `shutdown_timeout_ms` | u64 | ❌ NO | Not used anywhere in the codebase |

## Summary

### Used Fields (7/11 = 64%)
- ✅ `daemon.data_dir`
- ✅ `daemon.pid_file`
- ✅ `daemon.socket_path`
- ✅ `log.dir`
- ✅ `api.unix_socket_permissions`
- ✅ `process.restart_delay_ms`

### Unused Fields (5/11 = 45%)
- ❌ `log.max_size_mb` - Log rotation not implemented
- ❌ `log.max_files` - Log rotation not implemented
- ❌ `log.ring_buffer_size` - Ring buffer size is hardcoded in ProxyInfo
- ❌ `api.grpc_port` - Using Unix sockets instead of TCP
- ❌ `process.max_restart_attempts` - Auto-restart not implemented
- ❌ `process.shutdown_timeout_ms` - Graceful shutdown timeout not implemented

## Recommendations

1. **Remove unused fields** or implement the missing features:
   - Log rotation (max_size_mb, max_files)
   - Configurable ring buffer size
   - Auto-restart functionality
   - Graceful shutdown timeout

2. **Consider removing `grpc_port`** since the system uses Unix sockets exclusively

3. **The ring buffer is currently hardcoded** to 10,000 in `ProxyInfo::new()` but the config has a `ring_buffer_size` field that's not being used