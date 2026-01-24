/**
 * TLBX-1 - A Rust-based audio toolbox.
 * Copyright (C) 2026 Richard Bakos @ Resonance Designs.
 * Author: Richard Bakos <info@resonancedesigns.dev>
 * Website: https://resonancedesigns.dev
 * Version: 0.1.15
 * Component: Main Entry Point
 */

use nih_plug::prelude::*;
use env_logger::Env;
use tlbx1_core::TLBX1;

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .try_init()
        .ok();
    let mut args: Vec<String> = std::env::args().collect();
    #[cfg(target_os = "windows")]
    {
        let has_period_size = args.iter().any(|arg| {
            arg == "-p" || arg == "--period-size" || arg.starts_with("--period-size=")
        });
        if !has_period_size {
            // WASAPI can deliver larger buffers than the 512 default, so pick a safer size.
            args.push("--period-size".to_string());
            args.push("2048".to_string());
        }
    }
    nih_export_standalone_with_args::<TLBX1, _>(args);
}
