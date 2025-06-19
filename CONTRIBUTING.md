# Contributing to mcproc

Thank you for your interest in contributing to mcproc! This document provides guidelines and instructions for contributing.

## Code of Conduct

Please note that this project is released with a [Contributor Code of Conduct](CODE_OF_CONDUCT.md). By participating in this project you agree to abide by its terms.

## How to Contribute

### Reporting Issues

- Check if the issue has already been reported
- Create a clear and descriptive title
- Provide detailed steps to reproduce the issue
- Include relevant system information (OS, Rust version, etc.)
- Add logs or error messages if applicable

### Suggesting Enhancements

- Check if the enhancement has already been suggested
- Provide a clear use case for the enhancement
- Explain why existing features don't solve the problem
- If possible, suggest an implementation approach

### Pull Requests

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Run tests (`cargo test`)
5. Run linter (`cargo clippy -- -D warnings`)
6. Format code (`cargo fmt`)
7. Commit your changes with a descriptive message
8. Push to your fork
9. Create a Pull Request

## Development Setup

### Prerequisites

- Rust toolchain (latest stable)
- protobuf compiler
  - macOS: `brew install protobuf`
  - Linux: `apt-get install protobuf-compiler`

### Building

```bash
cargo build
```

### Testing

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture
```

### Code Style

- Follow Rust standard style guidelines
- Use `cargo fmt` before committing
- Ensure `cargo clippy` passes without warnings
- Write clear, self-documenting code
- Add comments for complex logic
- Update documentation for public APIs

## Commit Messages

- Use present tense ("Add feature" not "Added feature")
- Use imperative mood ("Move cursor to..." not "Moves cursor to...")
- Keep first line under 50 characters
- Reference issues and pull requests when relevant

Example:
```
feat: add process restart capability

- Add restart command to CLI
- Implement graceful shutdown before restart
- Update documentation

Fixes #123
```

## Release Process

Releases are managed by maintainers. The process is:

1. Update version in Cargo.toml files
2. Update CHANGELOG.md
3. Create and push a version tag
4. GitHub Actions will automatically build and release

## Questions?

Feel free to open an issue for any questions about contributing!