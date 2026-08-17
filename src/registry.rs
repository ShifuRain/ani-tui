use crate::anime_repo::{
    AnimeRepository, AnimeRepositoryError, Detail, Episode, GlobalId, Result, SearchResult,
};
use crate::websites::anidb_app::AnidbApp;
use futures::future::join_all;

/// Holds every registered [`AnimeRepository`] and routes requests to the right one by
/// [`GlobalId`] prefix.
pub struct Registry {
    /// Every source this registry knows about
    sources: Vec<Box<dyn AnimeRepository>>,
}

impl Registry {
    /// Builds a registry with every source this build supports.
    pub fn new() -> Self {
        Self {
            sources: vec![Box::new(AnidbApp::new())],
        }
    }

    /// Registers an additional source.
    pub fn add(&mut self, source: Box<dyn AnimeRepository>) {
        self.sources.push(source);
    }

    /// Finds the registered source with the given [`AnimeRepository::prefix`], if any.
    fn find(&self, prefix: &str) -> Option<&dyn AnimeRepository> {
        self.sources
            .iter()
            .find(|source| source.prefix() == prefix)
            .map(|source| source.as_ref())
    }

    /// Searches every registered source concurrently. Returns one result per source, tagged
    /// with that source's prefix, so a failure in one source doesn't hide results from the
    /// others.
    pub async fn search(&self, query: &str) -> Vec<(&'static str, Result<Vec<SearchResult>>)> {
        join_all(
            self.sources
                .iter()
                .map(|source| async move { (source.prefix(), source.search(query).await) }),
        )
        .await
    }

    /// Lists episodes for `id`, routed to the source that produced it.
    pub async fn list_eps(&self, id: &GlobalId) -> Result<Vec<Episode>> {
        self.find(&id.prefix)
            .ok_or(AnimeRepositoryError::Unsupported)?
            .list_eps(&id.raw)
            .await
    }

    /// Fetches details for `id`, routed to the source that produced it.
    pub async fn detail(&self, id: &GlobalId) -> Result<Detail> {
        self.find(&id.prefix)
            .ok_or(AnimeRepositoryError::Unsupported)?
            .detail(&id.raw)
            .await
    }

    /// Resolves a watch link for `id`, routed to the source that produced it.
    pub async fn watch_link(&self, id: &GlobalId) -> Result<String> {
        self.find(&id.prefix)
            .ok_or(AnimeRepositoryError::Unsupported)?
            .watch_link(&id.raw)
            .await
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_registered_source_by_prefix() {
        let registry = Registry::new();
        assert!(registry.find("ADB-1").is_some());
        assert!(registry.find("no-such-source").is_none());
    }

    /// A fake [`AnimeRepository`] for exercising [`Registry`] without any network access.
    struct MockSource {
        prefix: &'static str,
        fails: bool,
    }

    #[async_trait]
    impl AnimeRepository for MockSource {
        fn prefix(&self) -> &'static str {
            self.prefix
        }

        async fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
            if self.fails {
                return Err(AnimeRepositoryError::DatasourceError);
            }
            Ok(vec![SearchResult {
                title: format!("{query} from {}", self.prefix),
                id: GlobalId {
                    prefix: self.prefix.to_string(),
                    raw: "1".to_string(),
                },
            }])
        }

        async fn list_eps(&self, _raw_id: &str) -> Result<Vec<Episode>> {
            Ok(vec![])
        }

        async fn detail(&self, _raw_id: &str) -> Result<Detail> {
            Ok(Detail {
                title: "mock".to_string(),
                description: String::new(),
                episode_count: 0,
            })
        }

        async fn watch_link(&self, _raw_id: &str) -> Result<String> {
            Ok("mock-link".to_string())
        }
    }

    fn mock_registry() -> Registry {
        Registry {
            sources: vec![
                Box::new(MockSource {
                    prefix: "OK-1",
                    fails: false,
                }),
                Box::new(MockSource {
                    prefix: "BAD-1",
                    fails: true,
                }),
            ],
        }
    }

    #[tokio::test]
    async fn search_aggregates_across_sources_and_tolerates_partial_failure() {
        let registry = mock_registry();
        let results = registry.search("query").await;

        assert_eq!(results.len(), 2);

        let (_, ok_result) = results
            .iter()
            .find(|(prefix, _)| *prefix == "OK-1")
            .unwrap();
        let ok_result = ok_result.as_ref().expect("OK-1 should succeed");
        assert_eq!(ok_result[0].title, "query from OK-1");

        let (_, bad_result) = results
            .iter()
            .find(|(prefix, _)| *prefix == "BAD-1")
            .unwrap();
        assert!(
            bad_result.is_err(),
            "BAD-1 failing shouldn't affect OK-1's result"
        );
    }

    #[tokio::test]
    async fn routes_to_the_matching_source() {
        let registry = mock_registry();
        let id = GlobalId {
            prefix: "OK-1".to_string(),
            raw: "1".to_string(),
        };

        assert!(registry.list_eps(&id).await.is_ok());
        assert!(registry.detail(&id).await.is_ok());
        assert!(registry.watch_link(&id).await.is_ok());
    }

    #[tokio::test]
    async fn errors_on_unregistered_prefix() {
        let registry = mock_registry();
        let id = GlobalId {
            prefix: "NO-SUCH-SOURCE".to_string(),
            raw: "1".to_string(),
        };

        assert!(matches!(
            registry.detail(&id).await,
            Err(AnimeRepositoryError::Unsupported)
        ));
    }
}
