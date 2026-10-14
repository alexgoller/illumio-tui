//! In-memory TTL cache. Values are stored type-erased; a `get` with the
//! wrong type is simply a miss. Keys are strings like `labels` or
//! `workloads?hostname=web`.
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Entry {
    value: Arc<dyn Any + Send + Sync>,
    inserted: Instant,
    ttl: Duration,
}

#[derive(Default)]
pub struct TtlCache {
    map: Mutex<HashMap<String, Entry>>,
    /// The invalidation clock and, per invalidated prefix (`""` for
    /// `clear`), its tick then: a value read before it is stale.
    marks: Mutex<(u64, HashMap<String, u64>)>,
}

impl TtlCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the value and its age, or `None` when missing, expired, or of
    /// another type.
    pub fn get<T: Clone + Send + Sync + 'static>(&self, key: &str) -> Option<(T, Duration)> {
        let mut map = self.map.lock().expect("cache mutex poisoned");
        let entry = map.get(key)?;
        let age = entry.inserted.elapsed();
        if age >= entry.ttl {
            map.remove(key);
            return None;
        }
        let value = entry.value.clone().downcast::<T>().ok()?;
        Some(((*value).clone(), age))
    }

    pub fn insert<T: Send + Sync + 'static>(&self, key: &str, value: T, ttl: Duration) {
        let mut map = self.map.lock().expect("cache mutex poisoned");
        map.insert(
            key.to_string(),
            Entry {
                value: Arc::new(value),
                inserted: Instant::now(),
                ttl,
            },
        );
    }

    pub fn invalidate_prefix(&self, prefix: &str) {
        let mut marks = self.marks.lock().expect("cache mutex poisoned");
        marks.0 += 1;
        let tick = marks.0;
        marks.1.insert(prefix.to_string(), tick);
        let mut map = self.map.lock().expect("cache mutex poisoned");
        map.retain(|k, _| !k.starts_with(prefix));
    }

    pub fn clear(&self) {
        self.invalidate_prefix("");
    }

    /// The invalidation clock now: taken before a long request, it tells
    /// `replace_if_fresh` whether the answer was overtaken.
    pub fn mark(&self) -> u64 {
        self.marks.lock().expect("cache mutex poisoned").0
    }

    /// `value` under `key` in place of everything under `prefix`, unless a
    /// prefix of `key` was invalidated since `mark` (a write changed what
    /// the value was read from): a stale answer stores nothing and leaves
    /// the slot as it is. Whether it was stored.
    pub fn replace_if_fresh<T: Send + Sync + 'static>(
        &self,
        prefix: &str,
        key: &str,
        value: T,
        ttl: Duration,
        mark: u64,
    ) -> bool {
        let marks = self.marks.lock().expect("cache mutex poisoned");
        if marks
            .1
            .iter()
            .any(|(p, &tick)| tick > mark && key.starts_with(p.as_str()))
        {
            return false;
        }
        let mut map = self.map.lock().expect("cache mutex poisoned");
        map.retain(|k, _| !k.starts_with(prefix));
        map.insert(
            key.to_string(),
            Entry {
                value: Arc::new(value),
                inserted: Instant::now(),
                ttl,
            },
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn hit_within_ttl_miss_after() {
        let c = TtlCache::new();
        c.insert("labels", vec![1u32, 2], Duration::from_millis(30));
        let (v, age) = c.get::<Vec<u32>>("labels").expect("hit");
        assert_eq!(v, vec![1, 2]);
        assert!(age < Duration::from_millis(30));
        std::thread::sleep(Duration::from_millis(40));
        assert!(c.get::<Vec<u32>>("labels").is_none());
    }

    #[test]
    fn wrong_type_is_a_miss_and_prefix_invalidation_works() {
        let c = TtlCache::new();
        c.insert("policy:draft", 7u8, Duration::from_secs(60));
        c.insert("policy:active", 8u8, Duration::from_secs(60));
        c.insert("labels", 9u8, Duration::from_secs(60));
        assert!(c.get::<String>("policy:draft").is_none());
        c.invalidate_prefix("policy:");
        assert!(c.get::<u8>("policy:draft").is_none());
        assert!(c.get::<u8>("policy:active").is_none());
        assert_eq!(c.get::<u8>("labels").map(|(v, _)| v), Some(9));
        c.clear();
        assert!(c.get::<u8>("labels").is_none());
    }

    #[test]
    fn an_answer_read_before_an_invalidation_is_not_stored() {
        let c = TtlCache::new();
        let ttl = Duration::from_secs(60);
        let mark = c.mark();
        assert!(c.replace_if_fresh("draft:main:", "draft:main:/q/1", 1u8, ttl, mark));
        // A run started, then a write dropped the slot: its answer is stale.
        let mark = c.mark();
        c.invalidate_prefix("draft:");
        assert!(!c.replace_if_fresh("draft:main:", "draft:main:/q/1", 2u8, ttl, mark));
        assert!(
            c.get::<u8>("draft:main:/q/1").is_none(),
            "nothing repopulated"
        );
        // Another prefix's invalidation does not matter; a run after it stores.
        let mark = c.mark();
        c.invalidate_prefix("labels");
        assert!(c.replace_if_fresh("draft:main:", "draft:main:/q/1", 3u8, ttl, mark));
        assert_eq!(c.get::<u8>("draft:main:/q/1").map(|(v, _)| v), Some(3));
        // `clear` overtakes every prefix.
        let mark = c.mark();
        c.clear();
        assert!(!c.replace_if_fresh("draft:main:", "draft:main:/q/1", 4u8, ttl, mark));
    }
}
