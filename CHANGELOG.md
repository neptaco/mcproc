# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[Unreleased]: https://github.com/neptaco/mcproc/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/neptaco/mcproc/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/neptaco/mcproc/releases/tag/v0.1.0