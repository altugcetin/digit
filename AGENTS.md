# Repository Guidelines

## Project Structure & Module Organization

Digit is a Rust 2024/GTK4 Wayland desktop for Hyprland and niri. The Cargo workspace contains eight crates:

- `crates/kit`: shared UI, styling, motion, preferences, and compositor integration.
- `crates/shell`: the `digit` shell, panels, notifications, locking, and recording engine.
- `crates/ctl`: the lightweight `digitctl` command client.
- `crates/{settings,store,files,snap,rec}`: companion applications.

Source lives in each crate's `src/`. Tests are usually inline `#[cfg(test)]` modules; Settings also uses `src/tests.rs`. Assets and compositor templates live in `data/`; the security model is documented in `docs/security.md`.

## Build, Test, and Development Commands

Requirements: Rust 1.97+, GTK 4.18+, gtk4-layer-shell 1.2+, and Wayland development libraries.

```bash
cargo check --workspace                 # Check all crates
cargo test --workspace                  # Run workspace tests
cargo test -p digit-settings             # Test one application
cargo build --release -p digit-shell     # Build target/release/digit
cargo run -p digit-settings -- screenshots # Open a Settings page
digit msg status                        # Query the running shell
./install.sh --dry-run                   # Preview installation
```

Add `--offline` when dependencies are cached. Avoid launching a second shell alongside the installed instance.

## Coding Style & Design

Use four-space indentation, `snake_case` for functions/modules, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Follow neighboring code's compact layout; avoid unrelated formatting. Use rustfmt for new modules and `cargo clippy --workspace --all-targets` for lint review.

Read `.claude/skills/digit-design-principles/SKILL.md` before UI changes or reviews. Reuse existing components and palette roles, honor roundness/surface preferences, and keep UI strings short and English. Use system portal file choosers rather than substituting custom pickers.

## Testing Guidelines

Use Rust's built-in test framework with descriptive behavior names. No coverage threshold is configured. Add meaningful regressions for persistence failures, focus, selection, and cancellation. GTK checks require a display and may skip without one. Isolate configuration-writing tests with temporary XDG directories.

## Commit & Pull Request Guidelines

Follow history's descriptive titles, such as `Settings: ...` or `Shell: ...`. Keep commits focused. PR descriptions should explain the problem, resulting behavior, and validation; link relevant issues and include screenshots when available for visual changes.

## Security & Agent Workflow

Follow `docs/security.md`: never run Settings as root, use PackageKit/polkit for privileged operations, and write preferences atomically.

Preserve existing uncommitted work. Installing updated binaries and restarting affected applications or the shell is authorized when necessary; report these actions. Leave interactive testing and screenshots to the maintainer unless requested; do not start nested compositors. Ask before committing, and push only when explicitly authorized.
