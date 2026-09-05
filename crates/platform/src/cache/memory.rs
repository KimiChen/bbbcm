use super::*;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

struct CacheEntry {
    value: String,
    expires: Instant,
    accessed: u64,
}
#[derive(Default)]
struct CacheState {
    entries: HashMap<PublicCacheKey, CacheEntry>,
    clock: u64,
}
/// Bounded LRU for local single-process reads; never a shared security mechanism.
pub struct MemoryCache {
    state: Mutex<CacheState>,
    max_entries: usize,
}
impl Default for MemoryCache {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_MAX_ENTRIES)
    }
}
impl MemoryCache {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            state: Mutex::default(),
            max_entries: max_entries.max(1),
        }
    }
    pub fn len(&self) -> Result<usize, CacheError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| CacheError::Unavailable)?
            .entries
            .len())
    }
    pub fn is_empty(&self) -> Result<bool, CacheError> {
        Ok(self.len()? == 0)
    }
}
impl CacheStore for MemoryCache {
    fn get<'a>(
        &'a self,
        key: &'a PublicCacheKey,
    ) -> BoxFuture<'a, Result<Option<String>, CacheError>> {
        Box::pin(async move {
            let mut state = self.state.lock().map_err(|_| CacheError::Unavailable)?;
            state.clock = state.clock.saturating_add(1);
            let clock = state.clock;
            let Some(entry) = state.entries.get_mut(key) else {
                return Ok(None);
            };
            if Instant::now() >= entry.expires {
                state.entries.remove(key);
                return Ok(None);
            }
            entry.accessed = clock;
            Ok(Some(entry.value.clone()))
        })
    }
    fn set<'a>(
        &'a self,
        key: &'a PublicCacheKey,
        value: String,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<(), CacheError>> {
        Box::pin(async move {
            validate_cache_write(&value, ttl)?;
            let now = Instant::now();
            let mut state = self.state.lock().map_err(|_| CacheError::Unavailable)?;
            state.entries.retain(|_, entry| entry.expires > now);
            if !state.entries.contains_key(key) && state.entries.len() >= self.max_entries {
                let victim = state
                    .entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.accessed)
                    .map(|(key, _)| key.clone());
                if let Some(victim) = victim {
                    state.entries.remove(&victim);
                }
            }
            state.clock = state.clock.saturating_add(1);
            let accessed = state.clock;
            state.entries.insert(
                key.clone(),
                CacheEntry {
                    value,
                    expires: now + ttl,
                    accessed,
                },
            );
            Ok(())
        })
    }
    fn invalidate<'a>(&'a self, entity: &'a CacheEntity) -> BoxFuture<'a, Result<u64, CacheError>> {
        Box::pin(async move {
            let mut state = self.state.lock().map_err(|_| CacheError::Unavailable)?;
            let before = state.entries.len();
            state.entries.retain(|key, _| key.entity() != entity);
            Ok((before - state.entries.len()) as u64)
        })
    }
}

#[derive(Default)]
pub struct MemoryWakeup {
    notify: tokio::sync::Notify,
}
impl MemoryWakeup {
    pub fn new() -> Self {
        Self::default()
    }
}
impl Wakeup for MemoryWakeup {
    fn notify(&self) -> BoxFuture<'_, Result<(), CacheError>> {
        Box::pin(async {
            self.notify.notify_one();
            Ok(())
        })
    }
    fn wait(&self, timeout: Duration) -> BoxFuture<'_, Result<bool, CacheError>> {
        Box::pin(async move {
            Ok(tokio::time::timeout(timeout, self.notify.notified())
                .await
                .is_ok())
        })
    }
}
/// The no-Redis assembly deliberately keeps the database's polling schedule.
pub struct PollingWakeup;
impl Wakeup for PollingWakeup {
    fn notify(&self) -> BoxFuture<'_, Result<(), CacheError>> {
        Box::pin(async { Ok(()) })
    }
    fn wait(&self, timeout: Duration) -> BoxFuture<'_, Result<bool, CacheError>> {
        Box::pin(async move {
            tokio::time::sleep(timeout).await;
            Ok(false)
        })
    }
}

#[derive(Default)]
pub struct MemoryLease {
    held: Mutex<HashMap<LeaseKey, (LeaseToken, Instant)>>,
}
impl MemoryLease {
    pub fn new() -> Self {
        Self::default()
    }
}
impl Lease for MemoryLease {
    fn acquire<'a>(
        &'a self,
        key: &'a LeaseKey,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<Option<LeaseToken>, CacheError>> {
        Box::pin(async move {
            ttl_millis(ttl)?;
            let now = Instant::now();
            let mut held = self.held.lock().map_err(|_| CacheError::Unavailable)?;
            held.retain(|_, (_, expires)| *expires > now);
            if held.contains_key(key) || held.len() >= DEFAULT_MAX_ENTRIES {
                return Ok(None);
            }
            let token = LeaseToken::fresh();
            held.insert(key.clone(), (token.clone(), now + ttl));
            Ok(Some(token))
        })
    }
    fn renew<'a>(
        &'a self,
        key: &'a LeaseKey,
        token: &'a LeaseToken,
        ttl: Duration,
    ) -> BoxFuture<'a, Result<bool, CacheError>> {
        Box::pin(async move {
            ttl_millis(ttl)?;
            let now = Instant::now();
            let mut held = self.held.lock().map_err(|_| CacheError::Unavailable)?;
            match held.get_mut(key) {
                Some((current, expires)) if current == token && *expires > now => {
                    *expires = now + ttl;
                    Ok(true)
                }
                _ => Ok(false),
            }
        })
    }
    fn release<'a>(
        &'a self,
        key: &'a LeaseKey,
        token: &'a LeaseToken,
    ) -> BoxFuture<'a, Result<bool, CacheError>> {
        Box::pin(async move {
            let mut held = self.held.lock().map_err(|_| CacheError::Unavailable)?;
            if held
                .get(key)
                .is_some_and(|(current, expires)| current == token && *expires > Instant::now())
            {
                held.remove(key);
                Ok(true)
            } else {
                Ok(false)
            }
        })
    }
}
