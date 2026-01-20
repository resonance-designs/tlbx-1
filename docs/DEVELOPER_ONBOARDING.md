---
title: Developer Onboarding
sidebar_position: 2
---

# Developer Onboarding

Welcome! This guide covers the minimum steps to build and run GrainRust locally.

## Prerequisites

- Rust toolchain (stable)
- A working audio backend on your platform (WASAPI on Windows)
- Node.js + npm (for documentation tooling)

## Build and Run (Standalone)

```bash
npm run grainrust:dev
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
npm run grainrust:dev-docs
```

This generates `documentation/index.html` in the repo root for the app to open.

## Packaging

```bash
npm run grainrust:build
```

## Version Sync

```bash
npm run version:sync
```

The version is sourced from `Cargo.toml` and propagated to `README.md`, package.json files, and header blocks.

Packaging expects these environment variables:

- `GRAINRUST_VST3_PATH` (all platforms) points to the built `.vst3` bundle
- `GRAINRUST_APP_PATH` (macOS) points to the `.app` bundle

## Repository Layout

- `src/` contains DSP + app code
- `src/ui/` contains Slint UI definitions
- `src/ui/tape_engine.slint` contains the Tape engine UI component
- `src/ui/components/viz.slint` contains visualizer and meter components
- `docs/` contains Markdown developer docs
- `docs-site/` contains the Docusaurus and Storybook tooling
