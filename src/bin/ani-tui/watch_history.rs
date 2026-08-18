use ani_tui::anime_repo::GlobalId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Locally persisted "which episodes have been watched" state.
///
/// Storage is deliberately just a flat, append-only file rather than anything resembling a
/// database or a network sync service: `$XDG_DATA_HOME/ani-tui/watched.jsonl`, one JSON
/// object per line. Point any existing sync tool (Syncthing, Nextcloud, a dotfiles git repo,
/// rsync) at that file to carry watch history between devices — this app never talks to a
/// network for it. Because entries are folded by *timestamp* rather than by file position (see
/// [`fold`]), even a naive concatenation of two devices' files resolves correctly. The one real
/// limitation: this is read once at startup, not watched live, so the usual pattern is
/// close-sync-reopen rather than using the app on two devices at once.
pub struct WatchHistory {
    watched: HashMap<String, WatchRecord>,
    path: Option<PathBuf>,
}

/// One line of the watch-history file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WatchRecord {
    /// [`GlobalId::as_repr`] of the episode this record is about
    id: String,
    /// Whether it's watched (`false` = an explicit unmark, not the absence of a record)
    watched: bool,
    /// Unix seconds this record was written. Ties are broken in favor of whichever record is
    /// read later, which only matters for genuinely simultaneous writes and is not worth
    /// worrying about further.
    at: u64,
}

impl WatchHistory {
    /// Loads watch history from disk, or starts empty if the file doesn't exist (or the
    /// platform's data directory can't be determined) — either way, not a hard error.
    pub fn load() -> Self {
        let path = data_path();
        let watched = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|contents| fold(parse_lines(&contents)))
            .unwrap_or_default();
        Self { watched, path }
    }

    /// Every currently-watched id, as [`GlobalId::as_repr`] strings.
    pub fn watched_ids(&self) -> std::collections::HashSet<String> {
        self.watched
            .values()
            .filter(|record| record.watched)
            .map(|record| record.id.clone())
            .collect()
    }

    /// Updates `id`'s watched status, both in memory and (best-effort) on disk. A failure to
    /// write is silently ignored — the in-memory state still reflects the change for the rest
    /// of this run, it just won't persist for next time.
    pub fn set_watched(&mut self, id: &GlobalId, watched: bool) {
        let record = WatchRecord { id: id.as_repr(), watched, at: now_unix() };
        self.watched.insert(record.id.clone(), record.clone());
        self.append(&record);
    }

    fn append(&self, record: &WatchRecord) {
        let Some(path) = &self.path else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(line) = serde_json::to_string(record) else { return };
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// Path to the watch-history file: user data, not a regeneratable cache, so it belongs under
/// the platform's data dir.
fn data_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "ani-tui")
        .map(|dirs| dirs.data_dir().join("watched.jsonl"))
}

/// Parses every well-formed line of a watch-history file into records, silently skipping
/// malformed ones rather than failing the whole load.
fn parse_lines(contents: &str) -> Vec<WatchRecord> {
    contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Folds records into a map keyed by id, keeping the highest-`at` record per id regardless of
/// its position in the input — this is what makes a naive concatenation of two devices' files
/// safe: file order doesn't matter, only timestamps do.
fn fold(records: Vec<WatchRecord>) -> HashMap<String, WatchRecord> {
    let mut map: HashMap<String, WatchRecord> = HashMap::new();
    for record in records {
        match map.get(&record.id) {
            Some(existing) if existing.at >= record.at => {}
            _ => {
                map.insert(record.id.clone(), record);
            }
        }
    }
    map
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, watched: bool, at: u64) -> WatchRecord {
        WatchRecord { id: id.to_string(), watched, at }
    }

    #[test]
    fn fold_keeps_the_highest_timestamp_regardless_of_order() {
        let records = vec![
            record("<ADB-1:1>", true, 100),
            record("<ADB-1:1>", false, 50), // older, appears later in the file — should lose
        ];
        let folded = fold(records);
        assert!(folded["<ADB-1:1>"].watched);
    }

    #[test]
    fn fold_applies_a_later_unmark_over_an_earlier_mark() {
        let records = vec![record("<ADB-1:1>", true, 50), record("<ADB-1:1>", false, 100)];
        let folded = fold(records);
        assert!(!folded["<ADB-1:1>"].watched);
    }

    #[test]
    fn parse_lines_skips_malformed_entries() {
        let contents = "{\"id\":\"<ADB-1:1>\",\"watched\":true,\"at\":1}\nnot json\n";
        let records = parse_lines(contents);
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn set_watched_persists_and_reloads_via_a_real_file() {
        let path = std::env::temp_dir()
            .join(format!("ani-tui-test-watch-history-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let id = GlobalId { prefix: "ADB-1".to_string(), raw: "1".to_string() };
        {
            let mut history = WatchHistory { watched: HashMap::new(), path: Some(path.clone()) };
            assert!(history.watched_ids().is_empty());
            history.set_watched(&id, true);
            assert_eq!(history.watched_ids(), std::collections::HashSet::from([id.as_repr()]));
        }

        // Reload from the same file as a fresh instance, as a real run would on next launch.
        let contents = std::fs::read_to_string(&path).expect("file should have been written");
        let reloaded = WatchHistory {
            watched: fold(parse_lines(&contents)),
            path: Some(path.clone()),
        };
        assert_eq!(reloaded.watched_ids(), std::collections::HashSet::from([id.as_repr()]));

        let _ = std::fs::remove_file(&path);
    }
}
