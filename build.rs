/**
 * TLBX-1 - A Rust-based audio toolbox.
 * Copyright (C) 2026 Richard Bakos @ Resonance Designs.
 * Author: Richard Bakos <info@resonancedesigns.dev>
 * Website: https://resonancedesigns.dev
 * Version: 0.1.17
 * Component: Build Script
 */

fn main() {
    slint_build::compile("src/ui/tlbx1.slint").expect("Failed to compile Slint UI");
}
