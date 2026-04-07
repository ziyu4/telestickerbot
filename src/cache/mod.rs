//! Cache layer - Optional Moka-based caching

pub mod moka_impl;



use std::sync::Arc;
use async_trait::async_trait;

/// Async Cache trait for key-value storage with coalescing support
#[async_trait]
pub trait AsyncCache<K, V>: Send + Sync {
    async fn get(&self, key: &K) -> Option<Arc<V>>;
    async fn insert(&self, key: K, value: Arc<V>);
    async fn invalidate(&self, key: &K);
    
    /// Get or insert a value into the cache with coalescing
    async fn try_get_with<F, Fut, E>(&self, key: K, init: F) -> Result<Arc<V>, Arc<E>>
    where
        F: FnOnce() -> Fut + Send,
        Fut: std::future::Future<Output = Result<Arc<V>, E>> + Send,
        E: Send + Sync + 'static,
        V: Send + Sync + 'static,
        K: Send + 'static;
}

/// A concrete enum implementation of caching strategies to avoid dyn incompatibility
/// and provide high performance.
pub enum CacheLayer<K, V> {
    Moka(moka_impl::MokaCache<K, V>),
    None,
}

impl<K, V> CacheLayer<K, V> 
where
    K: Send + Sync + std::hash::Hash + Eq + Clone + 'static,
    V: Send + Sync + 'static,
{
    pub async fn get(&self, key: &K) -> Option<Arc<V>> {
        match self {
            Self::Moka(c) => c.get(key).await,
            Self::None => None,
        }
    }

    pub async fn insert(&self, key: K, value: Arc<V>) {
        match self {
            Self::Moka(c) => c.insert(key, value).await,
            Self::None => {}
        }
    }

    pub async fn invalidate(&self, key: &K) {
        match self {
            Self::Moka(c) => c.invalidate(key).await,
            Self::None => {}
        }
    }

    pub async fn try_get_with<F, Fut, E>(&self, key: K, init: F) -> Result<Arc<V>, Arc<E>>
    where
        F: FnOnce() -> Fut + Send,
        Fut: std::future::Future<Output = Result<Arc<V>, E>> + Send,
        E: Send + Sync + 'static,
    {
        match self {
            Self::Moka(c) => c.try_get_with(key, init).await,
            Self::None => init().await.map_err(Arc::new),
        }
    }
}