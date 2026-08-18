use ani_tui::anime_repo::{Detail, Episode, GlobalId};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// How many series' detail+episode-list stay cached at once. Bounded rather than unlimited
/// since a long TUI session could otherwise open hundreds of series over time; this is sized
/// for "recently opened", not "everything ever opened".
const MAX_ENTRIES: usize = 20;

/// In-memory, session-only cache of `(Detail, episodes)` per series, keyed by
/// [`GlobalId::as_repr`]. Purely a speed optimization for re-opening a series you already
/// looked at (e.g. backing out to search and back in): it never persists to disk and starts
/// empty every run, so it can't go stale across restarts, only within one. Staleness within a
/// run (e.g. a new episode airs) is handled by the caller invalidating an entry and re-fetching
/// on manual refresh — see the `r` key on the episodes screen.
///
/// A plain [`Mutex`] (not `tokio::sync::Mutex`) is enough here: every lock is held only for a
/// quick map operation, never across an `.await`, so there's no risk of blocking the async
/// runtime — the same reasoning that would apply to any other short, synchronous critical
/// section from sync code called into from async tasks.
#[derive(Default)]
pub struct SeriesCache {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<String, (Detail, Vec<Episode>)>,
    /// Insertion order of `entries`' keys, oldest first, for FIFO eviction once [`MAX_ENTRIES`]
    /// is exceeded. Not a true LRU (a cache *hit* doesn't move an entry back to the front) —
    /// unnecessary precision for a cache this small and short-lived.
    order: VecDeque<String>,
}

impl SeriesCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a clone of the cached `(Detail, episodes)` for `id`, if present.
    pub fn get(&self, id: &GlobalId) -> Option<(Detail, Vec<Episode>)> {
        self.inner.lock().unwrap().entries.get(&id.as_repr()).cloned()
    }

    /// Caches `(detail, episodes)` for `id`, evicting the oldest entry first if this would grow
    /// past [`MAX_ENTRIES`]. Overwriting an existing entry doesn't change its eviction position.
    pub fn insert(&self, id: &GlobalId, detail: Detail, episodes: Vec<Episode>) {
        let key = id.as_repr();
        let mut inner = self.inner.lock().unwrap();
        let is_new = inner.entries.insert(key.clone(), (detail, episodes)).is_none();
        if is_new {
            inner.order.push_back(key);
            if inner.order.len() > MAX_ENTRIES {
                if let Some(oldest) = inner.order.pop_front() {
                    inner.entries.remove(&oldest);
                }
            }
        }
    }

    /// Drops any cached entry for `id`, so the next lookup is a miss. Used for manual refresh.
    pub fn invalidate(&self, id: &GlobalId) {
        let key = id.as_repr();
        let mut inner = self.inner.lock().unwrap();
        inner.entries.remove(&key);
        inner.order.retain(|k| k != &key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: &str) -> GlobalId {
        GlobalId { prefix: "ADB-1".to_string(), raw: raw.to_string() }
    }

    fn detail(title: &str) -> Detail {
        Detail {
            title: title.to_string(),
            description: String::new(),
            episode_count: 0,
            languages: vec![],
        }
    }

    #[test]
    fn miss_on_an_unseen_id_returns_none() {
        let cache = SeriesCache::new();
        assert!(cache.get(&id("x")).is_none());
    }

    #[test]
    fn insert_then_get_round_trips() {
        let cache = SeriesCache::new();
        cache.insert(&id("x"), detail("Bocchi"), vec![]);

        let (detail, episodes) = cache.get(&id("x")).expect("should be cached");
        assert_eq!(detail.title, "Bocchi");
        assert!(episodes.is_empty());
    }

    #[test]
    fn invalidate_forces_a_future_miss() {
        let cache = SeriesCache::new();
        cache.insert(&id("x"), detail("Bocchi"), vec![]);
        cache.invalidate(&id("x"));

        assert!(cache.get(&id("x")).is_none());
    }

    #[test]
    fn oldest_entry_is_evicted_past_capacity() {
        let cache = SeriesCache::new();
        for n in 0..=MAX_ENTRIES {
            cache.insert(&id(&n.to_string()), detail(&n.to_string()), vec![]);
        }

        assert!(cache.get(&id("0")).is_none(), "oldest entry should have been evicted");
        assert!(cache.get(&id(&MAX_ENTRIES.to_string())).is_some(), "newest entry should remain");
    }
}
