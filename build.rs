/**
 * GrainRust - A Rust-based granular audio sampler.
 * Copyright (C) 2026 Richard Bakos @ Resonance Designs.
 * Author: Richard Bakos <info@resonancedesigns.dev>
 * Website: https://resonancedesigns.dev
 * Version: 0.1.7
 * Component: Build Script
 */

fn main() {
    slint_build::compile("src/ui/grainrust.slint").expect("Failed to compile Slint UI");
}
