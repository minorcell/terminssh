# SSH Manager

SSH Manager is a desktop SSH connection manager built with Rust, GPUI, and
gpui-component. It stores connection profiles locally and opens SSH sessions in
tabbed terminal panes.

## Features

- Save SSH profiles with password or private-key authentication.
- Search saved connections from the sidebar.
- Open multiple SSH sessions as tabs.
- Basic terminal interaction, including Tab completion and visible-grid text
  selection/copy.
- Delete saved profiles with confirmation.

## Requirements

- Rust toolchain
- macOS or Linux desktop environment supported by GPUI

## Run

```bash
cargo run
```

For a quick compile check:

```bash
cargo check
```

## Configuration

Saved profiles are written to:

```text
<system config dir>/ssh-mamaged/config.json
```

The exact base directory comes from the Rust `dirs::config_dir()` API. On macOS
this is typically under `~/Library/Application Support`; on Linux it is usually
under `~/.config`.

## Local Patches

The `patches/gpui_util` directory is intentional. `Cargo.toml` uses it through:

```toml
[patch."https://github.com/zed-industries/zed"]
gpui_util = { path = "patches/gpui_util" }
```

It replaces the upstream `gpui_util` crate from the pinned Zed/GPUI revision so
this project can build without the unstable `slice_as_array` feature. Do not
delete this directory unless the GPUI revision is upgraded or changed and
`cargo check` passes without the patch.

## Development Notes

- UI code lives mainly in `src/app.rs`, `src/ui`, and `src/terminal/view.rs`.
- SSH connection/session code lives in `src/ssh`.
- Terminal parsing and grid state live in `src/terminal`.
- The project currently has no unit tests; `cargo test` should still pass.
