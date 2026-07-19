use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use thiserror::Error;

use crate::Result as QueryResult;

/// Result-cache configuration or lock failure.
#[derive(Debug, Error)]
pub enum ResultCacheError {
    /// Limits or TTL are invalid.
    #[error("query result cache configuration is invalid")]
    InvalidConfiguration,
    /// Cache state was poisoned.
    #[error("query result cache state is unavailable")]
    State,
}

#[derive(Clone)]
struct Entry {
    result: QueryResult,
    size: usize,
    expires: Instant,
    last_used: Instant,
}

#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    total_bytes: usize,
}

/// TTL and LRU-bounded in-memory result cache.
pub struct ResultCache {
    maximum_bytes: usize,
    maximum_items: usize,
    ttl: Duration,
    state: Mutex<State>,
}

impl ResultCache {
    /// Creates a bounded cache.
    ///
    /// # Errors
    ///
    /// Returns an error for a sub-KiB byte bound, zero item bound, or zero TTL.
    pub fn new(
        maximum_bytes: usize,
        maximum_items: usize,
        ttl: Duration,
    ) -> std::result::Result<Self, ResultCacheError> {
        if maximum_bytes < 1_024 || maximum_items == 0 || ttl.is_zero() {
            return Err(ResultCacheError::InvalidConfiguration);
        }
        Ok(Self {
            maximum_bytes,
            maximum_items,
            ttl,
            state: Mutex::new(State::default()),
        })
    }

    /// Returns a cloned, cache-hit-marked result when present and unexpired.
    ///
    /// # Errors
    ///
    /// Returns an error if cache state was poisoned.
    pub fn get(&self, key: &str) -> std::result::Result<Option<QueryResult>, ResultCacheError> {
        let mut state = self.state.lock().map_err(|_| ResultCacheError::State)?;
        let now = Instant::now();
        if state
            .entries
            .get(key)
            .is_some_and(|entry| now >= entry.expires)
        {
            remove(&mut state, key);
            return Ok(None);
        }
        let Some(entry) = state.entries.get_mut(key) else {
            return Ok(None);
        };
        entry.last_used = now;
        let mut result = entry.result.clone();
        result.stats.cache_hit = true;
        Ok(Some(result))
    }

    /// Inserts a serializable result, evicting least-recently-used entries.
    /// Oversized or unserializable values are intentionally not cached.
    ///
    /// # Errors
    ///
    /// Returns an error if cache state was poisoned.
    pub fn put(
        &self,
        key: String,
        result: &QueryResult,
    ) -> std::result::Result<(), ResultCacheError> {
        let Ok(body) = serde_json::to_vec(result) else {
            return Ok(());
        };
        if body.len() > self.maximum_bytes {
            return Ok(());
        }
        let mut state = self.state.lock().map_err(|_| ResultCacheError::State)?;
        remove(&mut state, &key);
        let now = Instant::now();
        state.total_bytes += body.len();
        state.entries.insert(
            key,
            Entry {
                result: result.clone(),
                size: body.len(),
                expires: now + self.ttl,
                last_used: now,
            },
        );
        while state.total_bytes > self.maximum_bytes || state.entries.len() > self.maximum_items {
            let Some(oldest) = state
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            remove(&mut state, &oldest);
        }
        Ok(())
    }
}

fn remove(state: &mut State, key: &str) {
    if let Some(entry) = state.entries.remove(key) {
        state.total_bytes = state.total_bytes.saturating_sub(entry.size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Stats;

    #[test]
    fn marks_hits_and_evicts_lru() -> std::result::Result<(), ResultCacheError> {
        let cache = ResultCache::new(1_024, 1, Duration::from_mins(1))?;
        let result = QueryResult {
            columns: vec!["value".to_owned()],
            rows: vec![vec![serde_json::json!(1)]],
            stats: Stats::default(),
        };
        cache.put("one".to_owned(), &result)?;
        assert!(cache.get("one")?.is_some_and(|value| value.stats.cache_hit));
        cache.put("two".to_owned(), &result)?;
        assert!(cache.get("one")?.is_none());
        assert!(cache.get("two")?.is_some());
        Ok(())
    }
}
