# RISO

**Remote Interface for SSH Operations**

RISO is an open-source graphical SSH client designed to make managing remote servers simple and efficient.

It combines an SSH terminal with an integrated SFTP file explorer, allowing users to interact with remote systems and manage their files from a single interface.

## Features

- **Multiple SSH connections** — Manage multiple remote sessions through tabs.
- **Integrated SFTP explorer** — Browse and manage remote files and directories alongside the SSH terminal.
- **File operations** — Copy, paste, rename, move, and delete remote files and directories.
- **Drag & drop** — Upload and download files between local and remote systems.
- **Local file editing** — Open remote files with local applications and automatically synchronize changes back to the server.
- **SSH authentication** — Support for passwords and SSH keys.

## Installation

### Arch Linux

You can build and install the Arch package directly using `makepkg`:

```bash
makepkg -D packaging/arch -sic
```

Or, if the package file has already been generated:

```bash
sudo pacman -U packaging/arch/riso-*.pkg.tar.zst
```
