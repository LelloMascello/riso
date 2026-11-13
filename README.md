# RISO

**Remote Interface for SSH Operations**

RISO is an open-source SSH client designed to make managing remote servers simple and efficient.
It comes in two flavours built from the same codebase:

| Package    | Binary     | Frontend                          |
|------------|------------|-----------------------------------|
| `riso-gui` | `riso-gui` | GTK4 / libadwaita graphical app   |
| `riso-tui` | `riso-tui` | Terminal application (no GTK)     |

Both share the same connection history (`~/.config/riso/connections.json`).

## Features

### riso-gui

- **Multiple SSH connections** — Manage multiple remote sessions through tabs.
- **Integrated SFTP explorer** — Browse and manage remote files and directories alongside the SSH terminal.
- **File operations** — Copy, paste, rename, move, and delete remote files and directories.
- **Drag & drop** — Upload and download files between local and remote systems.
- **Local file editing** — Open remote files with local applications and automatically synchronize changes back to the server.
- **SSH authentication** — Support for passwords and SSH keys.

### riso-tui

- **Full Ratatui workspace** — Multi-tab SSH sessions with an embedded VT100 terminal and collapsible SFTP file explorer (`Alt+b` / `Alt+s`).
- **Native terminal ANSI colours** — Chrome uses only the host terminal's foreground/background and 16 ANSI colours (`Color::Reset` + ANSI 0–15), adapting automatically to your terminal palette.
- **Integrated SFTP explorer & local editing** — Browse, copy, cut, paste, rename, create, delete, upload, download, and edit remote files in `$EDITOR` / `xdg-open` with automatic upload on save.
- **Drag & drop** — Drop local files or `file://` URIs onto the terminal/explorer to upload, drop a local folder into the download prompt, or drag SFTP entries onto folders (move), the terminal (insert path), or `[↓ Scarica]`.
- **Shared history** — Connections opened in either frontend show up in `~/.config/riso/connections.json`.

## Project layout

```
src/
├── main.rs        entry point, picks the frontend (cargo features `gui` / `tui`)
├── config.rs      saved connections, host history, `ssh` argument parsing   ┐
├── files.rs       remote directory entry model, file helpers                │ shared,
├── line_edit.rs   prompt line-editing helpers                               │ toolkit-free
├── paths.rs       XDG directories, user / host name                         │
├── remote.rs      `ssh` control master + background SFTP worker             │
├── sftp.rs        SFTP v3 protocol client                                   ┘
├── gui/           riso-gui only (GTK4, libadwaita, VTE)
└── tui/           riso-tui only
```

## Building

```bash
cargo gui            # run riso-gui   (= cargo run --no-default-features --features gui)
cargo tui            # run riso-tui   (= cargo run --no-default-features --features tui)
cargo build-gui      # release build of riso-gui
cargo build-tui      # release build of riso-tui
```

Cargo always produces `target/release/riso`; the packages install it as
`riso-gui` or `riso-tui`. With both features enabled (`--all-features`) the
frontend is chosen by `--gui` / `--tui` or by the executable name.

## Installation

### Arch Linux

Each flavour has its own `PKGBUILD`:

```bash
makepkg -D packaging/arch/riso-gui -sic
makepkg -D packaging/arch/riso-tui -sic
```

Or, if the package file has already been generated:

```bash
sudo pacman -U packaging/arch/riso-gui/riso-gui-*.pkg.tar.zst
sudo pacman -U packaging/arch/riso-tui/riso-tui-*.pkg.tar.zst
```

`riso-gui` replaces the former `riso` package.
