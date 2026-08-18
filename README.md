# AniTUI

[![CI](https://github.com/ShifuRain/ani-tui/actions/workflows/ci.yml/badge.svg)](https://github.com/ShifuRain/ani-tui/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue.svg)](LICENSE)

AniTUI is an interactive terminal app (with a scriptable CLI mode too) for searching and
watching anime in [mpv](https://mpv.io/). It searches multiple sources at once and merges the
results; each result gets a distinctly colored `[Label]` badge in the TUI so it's obvious at a
glance which source (and likely language) it came from. Its request chain for anidb.app is
modeled on
[Pystardust's ani-cli](https://github.com/pystardust/ani-cli) — thanks to that project for
figuring it out.

Currently registered sources:

- **[anidb.app](https://anidb.app)** (`ADB-1`) — Japanese audio only for now.
- **[aniworld.to](https://aniworld.to)** (`AWT-1`) — a German aggregator; prefers German dub,
  then German sub, then English sub, whichever is available first.

`detail` (and the TUI's episode screen) shows a `Languages:` line listing what's available for
a given anime — informational only for now, there's no way yet to pick a specific one.

> **Note:** anidb.app sits behind a Cloudflare managed challenge. Requests are made with plain
> `curl`, which is usually enough, but if Cloudflare starts blocking it for you, install a
> [curl-impersonate](https://github.com/lexiforest/curl-impersonate) build (e.g.
> `curl_chrome136`) and make sure it's on `PATH` — AniTUI will pick it up automatically and use
> it instead, matching ani-cli's own fallback behavior. aniworld.to has no such gate.

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

Run `ani-tui` with no arguments for the interactive TUI: type a search, arrow through
results, drill into an episode list, hit enter to play. `mpv` launches in the background, so
the TUI stays interactive right away instead of freezing until it closes.

| Key | Action |
| --- | --- |
| type | edit the search box |
| `enter` | search / select the highlighted item |
| `up`/`down`, `j`/`k` | navigate |
| `down`/`tab` | move focus from the search box into results |
| `/` | back to the search box |
| `esc`/`backspace` | back a screen |
| `q` | quit (while not typing) |
| `ctrl+c` | quit, from anywhere |

For scripting, the same functionality is available as non-interactive subcommands. AniTUI
identifies anime with an ID in the format `<source:id>`; `watch`/`ep-count`/`detail` need one
from `search` first:

```console
$ ani-tui search "keywords"
```

The output lists titles alongside their IDs in `<>`. Copy an ID to use in the other commands.

```console
$ ani-tui detail "<ID>"
$ ani-tui ep-count "<ID>"
```

`detail` prints the most info about an anime: description, ID, episode count, and title.
`ep-count` just prints the title and episode count.

```console
$ ani-tui watch "<ID>" 1
```

Watches an episode in mpv. Replace `<ID>` and `1` (the episode number) with your own values.

Every command also accepts `-h`/`--help` for usage info, and `--version` prints the app version.

## Theming

The interactive TUI reads an optional YAML config file from
`$XDG_CONFIG_HOME/ani-tui/config.yml` (falls back to `~/.config/ani-tui/config.yml`) if
present. Nothing is required — every field defaults on its own, so a config only needs to list
what it wants to change. If the file exists but fails to parse, AniTUI prints a warning and
falls back to the defaults shown below rather than refusing to start.

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
    ADB-1:
      label: "AniDB"
      color: "#a6e3a1"
    AWT-1:
      label: "AniWorld"
      color: "#cba6f7"
```

Colors accept `"#rrggbb"` hex, or any of ratatui's named colors (e.g. `"lightblue"`). Under
`sources`, overriding just `label` or just `color` for a source keeps the other at its built-in
default — you don't need to repeat both. Any source prefix not listed falls back to showing its
raw prefix as the label and `muted` as the color, so a future third source still looks sane
with zero config changes.

---

## Contributing

1. Check open issues, or open your own describing what you'd like to change.
2. Fork the repo and branch off `main`.
3. Make your changes, with tests and docs where it makes sense.
4. Open a pull request explaining the change.
