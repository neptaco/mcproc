# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

We take security vulnerabilities seriously. If you discover a security vulnerability within mcproc, please follow these steps:

1. **DO NOT** open a public issue
2. Contact @neptaco_ on X (Twitter) with the details
3. Include the following information:
   - Type of vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)

### What to expect

- Acknowledgment of your report within 48 hours
- Regular updates on the progress
- Credit in the security advisory (unless you prefer to remain anonymous)

### Security considerations for mcproc

mcproc is designed for local development use only:
- It does not implement authentication or authorization
- It binds to localhost only by default
- All file operations use the permissions of the running user
- Process management is restricted to the current user's processes

**Important**: mcproc should NOT be exposed to untrusted networks or used in production environments.