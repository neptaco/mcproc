# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.4] - 2026-01-27

### Fixed
- **Grep pattern matching** - Fixed pattern matching to work correctly against original log lines including [ERROR]/[INFO] prefixes
- **gRPC status codes** - Use appropriate gRPC status codes for error responses
- **Multiple daemon instances** - Prevent multiple daemon instances from running simultaneously
- **MCP serve EOF handling** - MCP serve now properly exits when receiving EOF
- **Graceful shutdown logging** - Capture logs during graceful shutdown process

### Changed
- **MSRV updated to 1.80.0** - Minimum Supported Rust Version increased from 1.75.0 to 1.80.0
- **Simplified log system** - Removed ring buffer implementation for cleaner architecture

### Dependencies
- prost/tonic: 0.13 → 0.14
- axum-extra: 0.10.3 → 0.12.5
- sysinfo: 0.32.1 → 0.38.0
- colored: 2.2.0 → 3.1.1
- nix: 0.29.0 → 0.31.1
- tabled: 0.18.0 → 0.20.0
- tokio-tungstenite: 0.26.2 → 0.28.0
- toml: 0.8.23 → 0.9.11

### CI/CD
- cargo-deny for security and license checking
- cargo-machete for unused dependency detection
- MSRV verification job
- Code coverage with codecov
- Dependabot for automated dependency updates
- Updated GitHub Actions (checkout v6, cache v5, upload-artifact v6, download-artifact v7)

## [0.1.3] - 2025-07-30

### Added
- **Configurable gRPC timeouts** - Stop and restart command timeouts now properly account for graceful shutdown duration
- **Recursive child process termination** - All child processes are properly terminated when stopping a parent process (supports both macOS and Linux)
- **Improved process lifecycle management** - Enhanced signal handling for clean process termination

### Fixed
- **Graceful shutdown reliability** - Processes now properly complete graceful shutdown before termination
- **Port detection timing** - Improved timing for more reliable port detection
- **Process restart reliability** - Ensures processes are fully stopped before restarting
- **macOS compatibility** - Fixed shell trap handler for better cross-platform support

## [0.1.2] - 2025-07-08

### Added
- **Streaming support for restart command** - Real-time log output during process restart with progress visibility

### Improved
- **Enhanced error reporting for process failures** - Display exit code, failure reason, and stderr output when processes fail to start
- **Better MCP tool error handling** - Process startup failures now return ProcessInfo with failed status instead of errors, allowing LLMs to better understand and respond to failures

## [0.1.1] - 2025-07-03

### Added
- **Toolchain support** - Execute commands through version management tools (mise, asdf, nvm, etc.) via `--toolchain` parameter
- **Clean command** - Stop all processes in a project with `mcproc clean`
- **Process group management** - Proper cleanup of child processes when stopping parent
- **Enhanced process restart** - Improved restart capabilities for better automation
- **Log context display** - Show surrounding lines when wait_for_log pattern matches
- **Colored log output** - Lifecycle events now use colors (start=green, stop=yellow, exit=red)
- **Name validation** - Process and project names are validated for filesystem safety
- **High-performance logging** - New log streaming architecture for better performance
- **State synchronization** - Periodic sync between daemon and clients for accurate status

### Fixed
- Process name alignment in `ps` output (15-character padding)
- wait_for_log deadlock and hanging issues
- Ctrl+C responsiveness in `logs -f` command
- Daemon restart reliability with zombie process handling
- Command not found detection accuracy
- MCP timeout issues preventing operations from completing
- Daemon log file creation during auto-start
- ANSI code stripping in MCP responses

### Changed
- Logs are now organized by project: `~/.local/state/mcproc/log/{project}/{process}.log`
- Better error messages for process startup failures

## [0.1.0] - 2024-12-19

### Added
- Initial release
- Process management daemon with gRPC communication
- MCP (Model Context Protocol) server implementation
- CLI commands for process control (start, stop, restart, ps, logs, grep)
- Real-time log streaming with follow mode
- Project-based process organization
- Unix Domain Socket support
- Process status tracking and port detection
- Regex-based log searching with time filters

[Unreleased]: https://github.com/neptaco/mcproc/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/neptaco/mcproc/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/neptaco/mcproc/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/neptaco/mcproc/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/neptaco/mcproc/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/neptaco/mcproc/releases/tag/v0.1.0