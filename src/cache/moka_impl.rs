//! Moka-based cache implementation
#![cfg(feature = "moka-cache")]

use std::time::Duration;
use std::hash::Hash;
use std::sync::Arc;
use async_trait::async_trait;

use moka::future::Cache as MokaCacheInner;

use super::AsyncCache;

/// Moka-based async cache implementation with TTL and LRU eviction
pub struct MokaCache<K, V> {
    inner: MokaCacheInner<K, Arc<V>>,
}

impl<K, V> MokaCache<K, V>
where
    K: Hash + Eq + Send + Sync + Clone + 'static,
    V: Send + Sync + 'static,
{
    pub fn new(max_capacity: u64, ttl_seconds: u64) -> Self {
        let inner = MokaCacheInner::builder()
            .max_capacity(max_capacity)
            .time_to_live(Duration::from_secs(ttl_seconds))
            .build();
        Self { inner }
    }
}

#[async_trait]
impl<K, V> AsyncCache<K, V> for MokaCache<K, V>
where
    K: Hash + Eq + Send + Sync + Clone + 'static,
    V: Send + Sync + 'static,
{
    async fn get(&self, key: &K) -> Option<Arc<V>> {
        self.inner.get(key).await
    }

    async fn insert(&self, key: K, value: Arc<V>) {
        self.inner.insert(key, value).await;
    }

    async fn invalidate(&self, key: &K) {
        self.inner.invalidate(key).await;
    }

    async fn try_get_with<F, Fut, E>(&self, key: K, init: F) -> Result<Arc<V>, Arc<E>>
    where
        F: FnOnce() -> Fut + Send,
        Fut: std::future::Future<Output = Result<Arc<V>, E>> + Send,
        E: Send + Sync + 'static,
        V: Send + Sync + 'static,
        K: Send + 'static,
    {
        self.inner.try_get_with(key, async move {
            init().await
        }).await
    }
}