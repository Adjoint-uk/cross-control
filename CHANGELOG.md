# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial workspace scaffold with 8 crates
- Shared types: input events, device descriptors, screen geometry, machine identity, protocol messages
- Trait definitions: `InputCapture`, `InputEmulation`, `ClipboardProvider`, `Discovery`
- Wire format: length-prefixed bincode v2 encoding/decoding
- TLS certificate generation with SHA-256 fingerprinting
- CLI skeleton with `start`, `stop`, `status`, `generate-cert`, `pair` subcommands
- TOML configuration with sensible defaults
- CI pipeline (GitHub Actions): fmt, clippy, build, test on Linux + Windows
