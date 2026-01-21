---
title: Developer Docs
sidebar_position: 1
---

# Developer Documentation

This section is the home for Markdown-based developer docs. Add new files under `docs/` and they will appear in the Docusaurus site automatically.

Recent UI changes include the extracted `TapeEngine` component (`src/ui/tape_engine.slint`), lo‑fi knob rendering modes for performance-sensitive layouts, and visualizer components grouped in `src/ui/components/viz.slint`.

## Build the Docs Site

From the repo root:

```bash
npm run docs:install
npm run docs:dev
```

## Build Storybook (End-User MDX)

```bash
npm run storybook
```

## Packaging

Run the cross-platform packaging pipeline (OS-specific output):

```bash
npm run tlbx:build
```

## Version Sync

```bash
npm run version:sync
```

## Local Docs Deployment

```bash
npm run tlbx:dev-docs
```
