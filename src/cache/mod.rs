//! Cache layer - Optional Moka-based caching

#[cfg(feature = "moka-cache")]
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
    #[cfg(feature = "moka-cache")]
    Moka(moka_impl::MokaCache<K, V>),
    None(std::marker::PhantomData<(K, V)>),
}

impl<K, V> CacheLayer<K, V> 
where
    K: Send + Sync + std::hash::Hash + Eq + Clone + 'static,
    V: Send + Sync + 'static,
{
    /// Create a new moka-based cache if the feature is enabled, otherwise return a no-op cache.
    pub fn new_moka(max_capacity: u64, ttl_seconds: u64) -> Self {
        #[cfg(feature = "moka-cache")]
        {
            Self::Moka(moka_impl::MokaCache::new(max_capacity, ttl_seconds))
        }
        #[cfg(not(feature = "moka-cache"))]
        {
            let _ = (max_capacity, ttl_seconds); // Suppress unused warnings
            Self::none()
        }
    }

    /// Create a no-op (dummy) cache.
    pub fn none() -> Self {
        Self::None(std::marker::PhantomData)
    }

    pub async fn get(&self, _key: &K) -> Option<Arc<V>> {
        match self {
            #[cfg(feature = "moka-cache")]
            Self::Moka(c) => c.get(_key).await,
            Self::None(_) => None,
        }
    }

    pub async fn insert(&self, _key: K, _value: Arc<V>) {
        match self {
            #[cfg(feature = "moka-cache")]
            Self::Moka(c) => c.insert(_key, _value).await,
            Self::None(_) => {}
        }
    }

    pub async fn invalidate(&self, _key: &K) {
        match self {
            #[cfg(feature = "moka-cache")]
            Self::Moka(c) => c.invalidate(_key).await,
            Self::None(_) => {}
        }
    }

    pub async fn try_get_with<F, Fut, E>(&self, _key: K, init: F) -> Result<Arc<V>, Arc<E>>
    where
        F: FnOnce() -> Fut + Send,
        Fut: std::future::Future<Output = Result<Arc<V>, E>> + Send,
        E: Send + Sync + 'static,
    {
        match self {
            #[cfg(feature = "moka-cache")]
            Self::Moka(c) => c.try_get_with(_key, init).await,
            Self::None(_) => init().await.map_err(Arc::new),
        }
    }
}