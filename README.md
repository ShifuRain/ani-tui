# AniTUI

[![CI](https://github.com/ShifuRain/ani-tui/actions/workflows/ci.yml/badge.svg)](https://github.com/ShifuRain/ani-tui/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue.svg)](LICENSE)

AniTUI is a CLI (and, eventually, a TUI) app for searching and watching anime in [mpv](https://mpv.io/). It's a Rust rewrite of [Pystardust's ani-cli](https://github.com/pystardust/ani-cli) — thanks to that project for decoding the goload.pro/GoGoPlay link protocol this app relies on.

> **Note:** `goload.pro`, the only data source this app currently supports, is a parked/for-sale domain as of 2026 and no longer serves anime. `search`, `detail`, `ep-count`, and `watch` will not return results until the app is pointed at a working source (tracked as follow-up work). The rest of the codebase — dependencies, scraping logic, error handling — has been modernized and is otherwise in working order.

## Requirements

- [mpv](https://mpv.io/) installed and available on `PATH` — used to play episodes.

## Installation

Build from source with [Cargo](https://rustup.rs/):

```console
$ git clone https://github.com/ShifuRain/ani-tui.git
$ cd ani-tui
$ cargo install --path .
```

## Usage

AniTUI identifies anime with an ID in the format `<source:id>`. To use commands like `watch` or `ep-count` you first need an ID from `search`.

```console
$ ani-tui search "keywords"
```

The output lists titles alongside their IDs in `<>`. Copy an ID to use in the other commands.

```console
$ ani-tui detail "<ID>"
$ ani-tui ep-count "<ID>"
```

`detail` prints the most info about an anime: description, ID, episode count, and title. `ep-count` just prints the title and episode count.

```console
$ ani-tui watch "<ID>" 1
```

Watches an episode in mpv. Replace `<ID>` and `1` (the episode number) with your own values.

Every command also accepts `-h`/`--help` for usage info, and `--version` prints the app version.

---

## Contributing

1. Check open issues, or open your own describing what you'd like to change.
2. Fork the repo and branch off `main`.
3. Make your changes, with tests and docs where it makes sense.
4. Open a pull request explaining the change.
