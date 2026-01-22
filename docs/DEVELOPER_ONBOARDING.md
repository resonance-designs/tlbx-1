---
title: Developer Onboarding
sidebar_position: 2
---

# Developer Onboarding

Welcome! This guide covers the minimum steps to build and run TLBX-1 locally.

## Prerequisites

- Rust toolchain (stable)
- A working audio backend on your platform (WASAPI on Windows)
- Node.js + npm (for documentation tooling)

## Build and Run (Standalone)

```bash
npm run tlbx:dev
```

## Build and Run (Docs)

```bash
npm run docs:install
npm run docs:dev
```

## End-User Documentation (Storybook)

```bash
npm run storybook
```

## Local Docs Deployment

```bash
npm run tlbx:dev-docs
```

This generates `documentation/index.html` in the repo root for the app to open.

## Packaging

```bash
npm run tlbx:build
```

## Logging

Use `RUST_LOG` to adjust log verbosity:

```bash
RUST_LOG=symphonia_core=warn
```

```powershell
$env:RUST_LOG="symphonia_core=warn"
```

## Version Sync

```bash
npm run version:sync
```

The version is sourced from `Cargo.toml` and propagated to `README.md`, package.json files, and header blocks.

Packaging expects these environment variables:

- `TLBX_VST3_PATH` (all platforms) points to the built `.vst3` bundle
- `TLBX_APP_PATH` (macOS) points to the `.app` bundle

## Repository Layout

- `src/` contains DSP + app code
- `src/ui/` contains Slint UI definitions
- `src/ui/tlbx1.slint` contains the main window UI definition
- `src/ui/tape_engine.slint` contains the Tape engine UI component
- `src/ui/animate_engine.slint` contains the Animate engine UI component
- `src/ui/simpkick_engine.slint` contains the SimpKick engine UI component
- `src/ui/components/viz.slint` contains visualizer and meter components
- `src/ui/components/` contains shared controls (RDS Slint UI Component Kit)
- `docs/` contains Markdown developer docs
- `docs-site/` contains the Docusaurus and Storybook tooling

## UI Component Kit

TLBX-1 uses the Resonance Designs Slint UI Component Kit for shared controls and theming:
https://github.com/resonance-designs/rds-slint-ui-kit
