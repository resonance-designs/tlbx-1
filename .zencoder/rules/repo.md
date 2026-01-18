---
description: Repository Information Overview
alwaysApply: true
---

# Repository Information Overview

## Repository Summary
GrainRust is a 4-track granular sampler built in Rust, inspired by the Torso S-4 workflow. It operates as both a standalone application and an audio plugin (via `nih-plug`), featuring a UI developed with Slint. The project includes comprehensive documentation tooling using Docusaurus and Storybook.

## Repository Structure
- **src/**: Core Rust source code containing DSP logic, audio processing, and application state.
- **src/ui/**: Slint UI definitions (`.slint` files) and related visual resources.
- **docs/**: Developer-focused Markdown documentation.
- **docs-site/**: Docusaurus and Storybook project for generating the documentation website and UI component prototyping.
- **scripts/**: Utility scripts for packaging (Windows, macOS, Linux), version synchronization, and documentation deployment.
- **dist/**: Target directory for build artifacts and packaging output.
- **ref/**: Reference materials and 3rd-party documentation.

### Main Repository Components
- **GrainRust Core**: The primary Rust application and audio plugin.
- **Documentation Site**: A React-based project for developer and end-user documentation.

## Projects

### GrainRust (Core)
**Configuration File**: `Cargo.toml`

#### Language & Runtime
**Language**: Rust  
**Version**: Edition 2024  
**Build System**: Cargo + Slint-build  
**Package Manager**: cargo (Rust) / npm (Scripts)

#### Dependencies
**Main Dependencies**:
- `nih_plug`: Audio plugin framework (VST3/CLAP support).
- `slint`: UI framework.
- `fundsp`: Audio DSP library.
- `symphonia`: Audio decoding library (wav/flac/mp3/ogg).
- `cpal`: Cross-platform audio I/O.
- `baseview`: Windowing library for audio plugins.
- `serde`: Serialization/deserialization for project save/load.

#### Build & Installation
```bash
# Run standalone app in development mode
npm run grainrust:dev

# Run with backtraces
npm run grainrust:dev-bt

# Build installers/packages (requires GRAINRUST_VST3_PATH)
npm run grainrust:build
```

#### Main Files
- `src/main.rs`: Entry point for the standalone application.
- `src/lib.rs`: Entry point for the audio plugin and shared library logic.
- `src/ui/grainrust.slint`: Main UI definition file.

---

### Documentation Site
**Configuration File**: `docs-site/package.json`

#### Language & Runtime
**Language**: JavaScript / React  
**Build System**: Docusaurus & Storybook  
**Package Manager**: npm

#### Dependencies
**Main Dependencies**:
- `@docusaurus/core`: Documentation framework.
- `react`: UI library.
- `storybook`: UI component development environment.

#### Build & Installation
```bash
# Install documentation dependencies
npm run docs:install

# Start Docusaurus development server
npm run docs:dev

# Start Storybook for UI documentation
npm run storybook

# Build production documentation site
npm run docs:build
```

#### Key Resources
- `docs/`: Source Markdown files for developer documentation.
- `docs-site/src/`: Custom React components and styles for the site.
- `docs-site/stories/`: Storybook stories for UI components.

#### Validation
- **Version Sync**: `npm run version:sync` ensures versions are consistent across `Cargo.toml`, `README.md`, and `package.json`.
- **Slint Tooling**: Slint-build performs compile-time validation of UI definitions.
