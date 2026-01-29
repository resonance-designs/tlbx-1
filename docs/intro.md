---
title: Developer Docs
sidebar_position: 1
---

# Developer Documentation

This section is the home for Markdown-based developer docs. Add new files under `docs/` and they will appear in the Docusaurus site automatically.

Recent UI changes include extracted engine components (`src/ui/engines/tape_engine.slint`, `src/ui/engines/animate_engine.slint`, `src/ui/engines/syndrm_engine.slint`, `src/ui/engines/void_seed_engine.slint`), lo-fi knob rendering modes for performance-sensitive layouts, a custom `RDSComboBox` in `src/ui/components/selectors.slint`, a project/library browser panel, new shared inputs (XY pad, numeric keypad, keybed), and visualizer components grouped in `src/ui/components/viz.slint`.

The UI uses the [Resonance Designs Slint UI Component Kit](https://github.com/resonance-designs/rds-slint-ui-kit) for shared controls and theming.

## Build the Docs Site

From the repo root:

```bash
npm run docs:install
npm run docs:dev
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
