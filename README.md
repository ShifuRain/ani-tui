# AniTUI

[![CI](https://github.com/ShifuRain/ani-tui/actions/workflows/ci.yml/badge.svg)](https://github.com/ShifuRain/ani-tui/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue.svg)](LICENSE)

AniTUI is a terminal app (interactive TUI, or scriptable CLI) for searching and watching anime
in [mpv](https://mpv.io/). It searches multiple sources at once, merging results with a
colored `[Label]` badge per source so it's clear at a glance where (and in what language) each
result comes from:

- **[anidb.app](https://anidb.app)** (`ADB-1`) — Japanese audio.
- **[aniworld.to](https://aniworld.to)** (`AWT-1`) — German aggregator; German dub, then
  German sub, then English sub, whichever's available first.

Real per-episode titles are shown where available (aniworld.to natively; anidb.app via
[MyAnimeList](https://myanimelist.net)'s community [Jikan](https://jikan.moe/) API), episodes
you've watched get a `✓`, and you can jump straight to an episode number in a long-running
show. Its anidb.app request chain is modeled on
[Pystardust's ani-cli](https://github.com/pystardust/ani-cli) — thanks to that project for
figuring it out.

> **Note:** anidb.app sits behind a Cloudflare managed challenge; plain `curl` usually gets
> through, but if it starts getting blocked, install a
> [curl-impersonate](https://github.com/lexiforest/curl-impersonate) build (e.g.
> `curl_chrome136`) on `PATH` and AniTUI will use it automatically. aniworld.to has no such
> gate.

## Requirements

- [mpv](https://mpv.io/) installed and available on `PATH` — used to play episodes.

## Installation

**Prebuilt binaries** (Linux, macOS, Windows — x86_64 and aarch64 where applicable) are on the
[Releases page](https://github.com/ShifuRain/ani-tui/releases/latest). Download the archive
for your platform, then:

```console
# Linux / macOS
$ tar xzf ani-tui-*.tar.gz
$ sudo mv ani-tui-*/ani-tui /usr/local/bin/

# Windows (PowerShell) — unzip, then move ani-tui.exe onto your PATH
$ Expand-Archive ani-tui-*.zip
```

**From source**, with [Cargo](https://rustup.rs/):

```console
$ git clone https://github.com/ShifuRain/ani-tui.git
$ cd ani-tui
$ cargo install --path .
```

## Usage

Run `ani-tui` with no arguments for the TUI: type to search, arrow through results, drill into
an episode list, hit enter to play. `mpv` launches in the background, so the TUI stays
interactive right away.

| Key | Action |
| --- | --- |
| type | edit the search box |
| `enter` | search / select the highlighted item / play the selected episode |
| `up`/`down`, `j`/`k` | navigate |
| `down`/`tab` | move focus from the search box into results |
| `/` | back to the search box |
| `x` | toggle watched/unwatched on the selected episode |
| `g` | jump to an episode: type a number (`12`), or `S02E12` for shows whose numbering restarts each season, then `enter` (`esc` to cancel) |
| `esc`/`backspace` | back a screen |
| `q` | quit (while not typing) |
| `ctrl+c` | quit, from anywhere |

The same functionality is available as non-interactive subcommands for scripting. Anime are
identified by `<source:id>`; `watch`/`ep-count`/`detail` need one from `search` first:

```console
$ ani-tui search "keywords"        # lists titles with their IDs in <>
$ ani-tui detail "<ID>"            # title, description, episode count, languages
$ ani-tui ep-count "<ID>"          # just the title and episode count
$ ani-tui watch "<ID>" 1           # plays episode 1 in mpv
```

Every command accepts `-h`/`--help`; `--version` prints the app version.

## Theming

The TUI reads an optional YAML config from `$XDG_CONFIG_HOME/ani-tui/config.yml` (falls back
to `~/.config/ani-tui/config.yml`). Every field defaults on its own — only list what you want
to change — and a broken file just prints a warning and falls back to defaults rather than
refusing to start.

```yaml
theme:
  border_type: rounded    # rounded | plain | double | thick
  accent: "#89b4fa"       # focused borders, in-progress status text
  text: "#cdd6f4"         # default text color
  muted: "#6c7086"        # unfocused borders, hints, unmapped-source badges
  error: "#f38ba8"        # error status text
  warning: "#f9e2af"      # warning status text
  selection_fg: "#1e1e2e" # text color of the selected row's highlight bar
  selection_bg: "#89b4fa" # background color of the selected row's highlight bar
  sources:                # per-source badge label/color, keyed by source prefix
    ADB-1: { label: "AniDB", color: "#a6e3a1" }
    AWT-1: { label: "AniWorld", color: "#cba6f7" }
```

Colors accept `"#rrggbb"` hex or any ratatui named color (e.g. `"lightblue"`). Overriding just
`label` or just `color` on a source keeps the other at its built-in default. An unlisted source
prefix falls back to the raw prefix as label and `muted` as color.

## Watch history

Watched/unwatched state lives at `$XDG_DATA_HOME/ani-tui/watched.jsonl` (falls back to
`~/.local/share/ani-tui/watched.jsonl`), one line appended per change. AniTUI doesn't sync
this anywhere itself — point Syncthing, Nextcloud, a dotfiles git repo, or rsync at the file to
carry history between devices. Records resolve by timestamp, not file position, so even a
naive concatenation of two devices' files works. It's read once at startup, not watched live,
so the pattern is close-sync-reopen rather than using two devices at once.

---

## Contributing

1. Check open issues, or open your own describing what you'd like to change.
2. Fork the repo and branch off `main`.
3. Make your changes, with tests and docs where it makes sense.
4. Open a pull request explaining the change.
