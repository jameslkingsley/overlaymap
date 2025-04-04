//! A two-layered map structure where the foreground is mutable and the
//! background is preserved.
//!
//! OverlayMap lets you insert values without cloning, while keeping a single
//! layer of historical context. Each key has a current value (foreground) and
//! may have a previous value (background), which is automatically managed
//! during updates.
//!
//! ```rust
//! use overlay_map::Overlay;
//!
//! let mut door = Overlay::new_fg("Alice");
//! println!("Present: {:?}, {:?}", door.fg(), door.bg());
//!
//! for name in ["Bob", "Carol", "Dave", "Eve"] {
//!     if let Some(evicted) = door.swap(name) {
//!         println!("{evicted} left");
//!     }
//!     println!("Present: {:?}, {:?}", door.bg(), door.fg());
//! }
//!
//! while let Some(pulled) = door.pull() {
//!     println!("{pulled} left");
//! }
//!
//! println!("Present: {:?}, {:?}", door.bg(), door.fg());
//! ```

use std::{
    hash::{BuildHasher, Hash},
    mem::MaybeUninit,
};

use hashbrown::{DefaultHashBuilder, HashMap, hash_map::RawEntryMut};

/// A two-layered map where each key has a mutable foreground and an optional
/// background value.
///
/// When inserting a new value for a key, the previous value (if any) is
/// automatically moved to the background. Background values are preserved but
/// not cloned.
///
/// This map is not thread-safe for mutation. It may be shared across threads
/// for read-only access.
#[derive(Debug, Default)]
pub struct OverlayMap<K, V, S = DefaultHashBuilder>
where
    K: Eq + Hash,
{
    map: HashMap<K, Overlay<V>, S>,
}

unsafe impl<K, V, S> Sync for OverlayMap<K, V, S>
where
    K: Eq + Hash + Sync,
    S: Sync,
{
}

impl<K, V, S> OverlayMap<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher + Default,
{
    /// Creates a new, empty `OverlayMap` with the default hasher.
    pub fn new() -> Self {
        Self::with_hasher(Default::default())
    }

    /// Creates an empty `OverlayMap` with the specified capacity and default hasher.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_capacity_and_hasher(capacity, Default::default())
    }

    /// Creates an empty `OverlayMap` that will use the given hasher.
    pub fn with_hasher(hasher: S) -> Self {
        Self {
            map: HashMap::with_hasher(hasher),
        }
    }

    /// Creates an empty `OverlayMap` with the specified capacity and hasher.
    pub fn with_capacity_and_hasher(capacity: usize, hasher: S) -> Self {
        Self {
            map: HashMap::with_capacity_and_hasher(capacity, hasher),
        }
    }

    /// Number of unique keys in the map.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Check if the map is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Get an immutable reference to the value associated with the key.
    ///
    /// Returns `None` if the key was not found in the map.
    #[inline]
    pub fn fg(&self, key: &K) -> Option<&V> {
        self.map.get(key).map(|entry| entry.fg_unchecked())
    }

    /// Get an immutable reference to the value associated with the key in the background layer.
    ///
    /// Returns `None` if the key was not found in the background layer.
    #[inline]
    pub fn bg(&self, key: &K) -> Option<&V> {
        self.map.get(key).and_then(|entry| entry.bg())
    }

    /// Push a value into the foreground layer, preserving the previous value in
    /// the background.
    ///
    /// If the key was already present, the current foreground is moved to the
    /// background slot, and the new value becomes the new foreground. No
    /// cloning occurs. The old background value is dropped if it was present.
    ///
    /// Returns `true` if there was already a foreground value (i.e. a
    /// background now definitely exists).
    #[inline]
    pub fn push(&mut self, key: K, value: V) -> bool {
        match self.map.raw_entry_mut().from_key(&key) {
            RawEntryMut::Occupied(mut occupied) => {
                occupied.get_mut().push(value);
                true
            }
            RawEntryMut::Vacant(vacant) => {
                vacant.insert(key, Overlay::new_fg(value));
                false
            }
        }
    }

    /// Conditionally push a new value into the foreground based on the current
    /// value.
    ///
    /// If the key exists, the current foreground value is passed to the
    /// predicate. If the predicate returns `Some(new_val)`, the new value is
    /// pushed and the old one is preserved in the background. If it returns
    /// `None`, nothing is changed.
    ///
    /// Returns `true` if a new value was pushed.
    pub fn push_if<F>(&mut self, key: &K, predicate: F) -> bool
    where
        F: FnOnce(&V) -> Option<V>,
    {
        let entry = match self.map.get_mut(key) {
            Some(e) => e,
            None => return false,
        };

        match predicate(entry.fg_unchecked()) {
            Some(new) => {
                entry.push(new);
                true
            }
            None => false,
        }
    }

    /// Pulls the foreground value for a key, promoting the background to foreground if present.
    ///
    /// This removes and returns the current foreground value for the given key. If a background
    /// value exists, it is promoted to foreground. If the key has no background after the pull,
    /// the key is removed from the map entirely.
    ///
    /// # Returns
    ///
    /// - `Some(value)` if the key existed and a foreground value was pulled.
    /// - `None` if the key did not exist.
    ///
    /// # Invariants
    ///
    /// - After this operation, the key is only retained if a background value was available
    ///   to promote.
    /// - Keys in the map always have at least one value (foreground), unless removed by `pull`.
    ///
    /// # Example
    ///
    /// ```
    /// use overlay_map::OverlayMap;
    ///
    /// let mut map = OverlayMap::<&str, i32>::new();
    /// map.push("key", 1);
    /// map.push("key", 2);
    ///
    /// assert_eq!(map.fg(&"key"), Some(&2));
    /// assert_eq!(map.bg(&"key"), Some(&1));
    ///
    /// let pulled = map.pull(&"key");
    /// assert_eq!(pulled, Some(2));
    /// assert_eq!(map.fg(&"key"), Some(&1)); // background promoted
    ///
    /// let pulled = map.pull(&"key");
    /// assert_eq!(pulled, Some(1));
    /// assert_eq!(map.fg(&"key"), None); // entry removed
    /// ```
    #[inline]
    pub fn pull(&mut self, key: &K) -> Option<V> {
        match self.map.raw_entry_mut().from_key(key) {
            RawEntryMut::Occupied(mut occupied) => {
                let entry = occupied.get_mut();
                let evicted = entry.pull_unchecked();
                if entry.is_empty() {
                    occupied.remove();
                }
                Some(evicted)
            }
            RawEntryMut::Vacant(_) => None,
        }
    }

    /// Conditionally pulls the foreground value for a key, promoting the background if present.
    ///
    /// If the key exists and the provided predicate returns `true` for the current foreground,
    /// this removes and returns the foreground value. The background (if any) is promoted to
    /// foreground, and the key is removed from the map if no background remains.
    ///
    /// If the predicate returns `false` or the key does not exist, the map is left unchanged.
    ///
    /// # Returns
    ///
    /// - `Some(value)` if the predicate matched and the foreground was pulled.
    /// - `None` if the key was not found or the predicate returned `false`.
    ///
    /// # Invariants
    ///
    /// - After this operation, the key is only retained if a background value was available
    ///   to promote.
    /// - Keys in the map always have at least one value (foreground), unless removed by `pull_if`.
    ///
    /// # Example
    ///
    /// ```
    /// use overlay_map::OverlayMap;
    ///
    /// let mut map = OverlayMap::<&str, i32>::new();
    /// map.push("key", 10);
    /// map.push("key", 20);
    ///
    /// // Only pull if the foreground is 20
    /// let pulled = map.pull_if(&"key", |v| *v == 20);
    /// assert_eq!(pulled, Some(20));
    /// assert_eq!(map.fg(&"key"), Some(&10));
    ///
    /// // Predicate does not match: nothing is pulled
    /// let pulled = map.pull_if(&"key", |v| *v == 999);
    /// assert_eq!(pulled, None);
    /// assert_eq!(map.fg(&"key"), Some(&10));
    ///
    /// // Pull remaining value, removing the key
    /// let pulled = map.pull_if(&"key", |_| true);
    /// assert_eq!(pulled, Some(10));
    /// assert_eq!(map.fg(&"key"), None);
    /// ```
    pub fn pull_if<F>(&mut self, key: &K, predicate: F) -> Option<V>
    where
        F: FnOnce(&V) -> bool,
    {
        match self.map.raw_entry_mut().from_key(key) {
            RawEntryMut::Occupied(mut occupied) => {
                let entry = occupied.get_mut();
                if predicate(entry.fg_unchecked()) {
                    let evicted = entry.pull_unchecked();
                    if entry.is_empty() {
                        occupied.remove();
                    }
                    Some(evicted)
                } else {
                    None
                }
            }
            RawEntryMut::Vacant(_) => None,
        }
    }

    /// Swap a value into the foreground layer, preserving the previous value in
    /// the background, and returning the evicted background value if present.
    ///
    /// If the key was already present, the current foreground is moved to the
    /// background slot, and the new value becomes the new foreground. No
    /// cloning occurs. The old background value is returned if present.
    #[inline]
    pub fn swap(&mut self, key: K, value: V) -> Option<V> {
        match self.map.raw_entry_mut().from_key(&key) {
            RawEntryMut::Occupied(mut occupied) => occupied.get_mut().swap(value),
            RawEntryMut::Vacant(vacant) => {
                vacant.insert(key, Overlay::new_fg(value));
                None
            }
        }
    }

    /// Conditionally swap a new value into the foreground based on the current
    /// value.
    ///
    /// If the key exists, the current foreground value is passed to the
    /// predicate. If the predicate returns `Some(new_val)`, the new value is
    /// pushed and the old one is preserved in the background. If it returns
    /// `None`, nothing is changed.
    ///
    /// The evicted background value is returned if present.
    pub fn swap_if<F>(&mut self, key: &K, predicate: F) -> Option<V>
    where
        F: FnOnce(&V) -> Option<V>,
    {
        let entry = self.map.get_mut(key)?;
        match predicate(entry.fg_unchecked()) {
            Some(new) => entry.swap(new),
            None => None,
        }
    }

    /// Overlay multiple values onto the map.
    ///
    /// Each key-value pair is pushed into the foreground layer. If the key was
    /// already present, the existing foreground value is moved to the
    /// background. This does not clone or retain old values beyond the
    /// background layer.
    ///
    /// Returns the number of keys that already existed (i.e. pushes that
    /// replaced a foreground).
    pub fn overlay<I>(&mut self, iter: I) -> usize
    where
        I: IntoIterator<Item = (K, V)>,
    {
        let mut replaced = 0;
        for (key, val) in iter {
            replaced += self.push(key, val) as usize;
        }
        replaced
    }
}

const SLOT0_PRESENT: u8 = 1 << 0;
const SLOT1_PRESENT: u8 = 1 << 1;
const FG_SLOT: u8 = 1 << 2;

/// A two-layer value container used by [`OverlayMap`] to manage current and historical values.
///
/// `Overlay<T>` stores up to two values:
///
/// - A **foreground** value representing the current state.
/// - An optional **background** value representing the previous state.
///
/// When used through [`OverlayMap`], each key maps to an `Overlay<T>` to track updates
/// without requiring clones or reallocations. You can also use `Overlay<T>` standalone
/// to manage two-layer state transitions for any value type.
///
/// Values are moved, never cloned. All operations (push, pull, swap) are zero-cost and
/// memory-efficient.
///
/// # Use Cases
///
/// - Managing current and previous state in UI or simulation logic
/// - Efficient delta tracking for configs, game state, etc.
/// - Avoiding `Option<(T, T)>` or custom wrappers with cloning overhead
///
/// # Examples
///
/// ```
/// use overlay_map::Overlay;
///
/// let mut entry = Overlay::new_fg("current");
/// entry.push("next"); // moves "current" into background
///
/// assert_eq!(entry.fg(), Some(&"next"));
/// assert_eq!(entry.bg(), Some(&"current"));
///
/// let pulled = entry.pull();
/// assert_eq!(pulled, Some("next"));
/// assert_eq!(entry.fg(), Some(&"current"));
/// ```
///
/// [`OverlayMap`]: crate::OverlayMap
#[derive(Debug)]
pub struct Overlay<T> {
    bits: u8,
    slots: [MaybeUninit<T>; 2],
}

impl<T> Overlay<T> {
    /// Creates a new `Overlay` with no values.
    pub fn new_empty() -> Self {
        Self {
            bits: 0,
            slots: [MaybeUninit::uninit(), MaybeUninit::uninit()],
        }
    }

    /// Creates a new `Overlay` with a foreground value and no background.
    pub fn new_fg(val: T) -> Self {
        Self {
            bits: SLOT0_PRESENT,
            slots: [MaybeUninit::new(val), MaybeUninit::uninit()],
        }
    }

    /// Creates a new `Overlay` with both foreground and background values.
    pub fn new_both(fg: T, bg: T) -> Self {
        Self {
            bits: SLOT0_PRESENT | SLOT1_PRESENT,
            slots: [MaybeUninit::new(fg), MaybeUninit::new(bg)],
        }
    }

    /// Returns a reference to the current foreground value, if present.
    ///
    /// This returns `Some(&T)` only if the foreground slot contains a value.
    /// If the slot is logically empty, returns `None`. This is a safe version that
    /// checks the presence bits before accessing memory.
    ///
    /// # Safety
    /// This function is fully safe and performs a presence check before dereferencing.
    ///
    /// # Returns
    /// - `Some(&T)` if the foreground slot is initialized
    /// - `None` if the foreground slot is uninitialized
    #[inline]
    pub fn fg(&self) -> Option<&T> {
        let idx = self.fg_index();
        if self.is_slot_present(idx) {
            Some(unsafe { self.slots[idx].assume_init_ref() })
        } else {
            None
        }
    }

    /// Returns a reference to the foreground value **without checking** if it is present.
    ///
    /// # Safety
    /// This function **assumes** the foreground slot is initialized. Calling this when
    /// the slot is uninitialized (i.e. after a `pull()` without a background, or
    /// after constructing an empty `Overlay`) results in **undefined behavior**.
    ///
    /// Use [`fg`](Self::fg) if you are not certain the slot is populated.
    ///
    /// # Panics
    /// Never panics, but causes UB if the foreground slot is not present.
    #[inline]
    pub fn fg_unchecked(&self) -> &T {
        let idx = self.fg_index();
        unsafe { self.slots[idx].assume_init_ref() }
    }

    /// Returns a reference to the background value, if present.
    ///
    /// Returns `Some(&T)` only if the background slot is initialized.
    #[inline]
    pub fn bg(&self) -> Option<&T> {
        let idx = self.bg_index();
        if self.is_slot_present(idx) {
            Some(unsafe { self.slots[idx].assume_init_ref() })
        } else {
            None
        }
    }

    /// Returns a reference to the background value **without checking** if it is present.
    ///
    /// # Safety
    /// This assumes the background slot is initialized. Calling this when it is not
    /// will cause **undefined behavior**.
    ///
    /// Prefer [`bg`](Self::bg) if you're unsure whether the background is set.
    #[inline]
    pub fn bg_unchecked(&self) -> &T {
        let idx = self.bg_index();
        unsafe { self.slots[idx].assume_init_ref() }
    }

    /// Returns `true` if both slots are empty.
    ///
    /// This is used to determine whether the entry contains any values
    /// at all. It does not consider which slot is foreground or background.
    #[inline]
    pub fn is_empty(&self) -> bool {
        (self.bits & (SLOT0_PRESENT | SLOT1_PRESENT)) == 0
    }

    /// Push a value into the foreground layer, preserving the previous foreground in the background.
    ///
    /// If the foreground slot already contains a value, it is moved into the background slot.
    /// The new value is then written into the foreground slot. Any previous background value
    /// will be dropped to make room—no cloning is performed at any point.
    ///
    /// This operation is always safe, even if the entry is empty. If no foreground is currently
    /// present, the value will simply be inserted.
    ///
    /// # Example
    /// ```
    /// use overlay_map::Overlay;
    /// let mut entry = Overlay::new_fg("a");
    /// entry.push("b");
    /// assert_eq!(entry.fg(), Some(&"b"));
    /// assert_eq!(entry.bg(), Some(&"a"));
    /// ```
    #[inline]
    pub fn push(&mut self, val: T) {
        self.push_fg_to_bg();
        let idx = self.fg_index();
        self.slots[idx] = MaybeUninit::new(val);
        self.bits |= 1 << idx;
    }

    /// Safely pull the current foreground value out, promoting the background to foreground if present.
    ///
    /// If the foreground value is present, it is removed and returned. The background (if any) is
    /// promoted to the foreground. If neither value remains, the entry becomes empty.
    ///
    /// Returns `None` if the foreground was not present.
    ///
    /// # Example
    /// ```
    /// use overlay_map::Overlay;
    /// let mut entry = Overlay::new_both("a", "b");
    /// let pulled = entry.pull();
    /// assert_eq!(pulled, Some("a"));
    /// assert_eq!(entry.fg(), Some(&"b")); // background promoted
    /// ```
    #[inline]
    pub fn pull(&mut self) -> Option<T> {
        let fgi = self.fg_index();
        if !self.is_slot_present(fgi) {
            return None;
        }

        let evicted = unsafe { self.slots[fgi].assume_init_read() };
        self.bits &= !(1 << fgi);
        self.flip();
        Some(evicted)
    }

    /// Pull the current foreground value without checking if it is present.
    ///
    /// # Safety
    /// The caller must ensure the foreground slot is present. If it is not, this will result
    /// in undefined behavior.
    ///
    /// See [`Self::pull`] for a safe alternative.
    #[inline]
    pub fn pull_unchecked(&mut self) -> T {
        let fgi = self.fg_index();
        let evicted = unsafe { self.slots[fgi].assume_init_read() };
        self.bits &= !(1 << fgi);
        self.flip();
        evicted
    }

    /// Swap in a new foreground value, returning the old background if present.
    ///
    /// If a background value exists, it is evicted and returned. The new value is written into
    /// the background slot, which is then promoted to become the new foreground. The current
    /// foreground is preserved in-place.
    ///
    /// If no background was present, this behaves like a standard push operation,
    /// and `None` is returned.
    ///
    /// # Example
    /// ```
    /// use overlay_map::Overlay;
    /// let mut entry = Overlay::new_both("a", "b");
    /// let evicted = entry.swap("c");
    /// assert_eq!(evicted, Some("b"));
    /// assert_eq!(entry.fg(), Some(&"c"));
    /// assert_eq!(entry.bg(), Some(&"a"));
    /// ```
    #[inline]
    pub fn swap(&mut self, val: T) -> Option<T> {
        let bgi = self.bg_index();
        if self.is_slot_present(bgi) {
            let evicted = unsafe { self.slots[bgi].assume_init_read() };
            self.slots[bgi] = MaybeUninit::new(val);
            self.flip();
            Some(evicted)
        } else {
            self.push(val);
            None
        }
    }

    #[inline]
    fn fg_index(&self) -> usize {
        ((self.bits & FG_SLOT) >> 2) as usize
    }

    #[inline]
    fn bg_index(&self) -> usize {
        self.fg_index() ^ 1
    }

    #[inline]
    fn is_slot_present(&self, idx: usize) -> bool {
        (self.bits & (1 << idx)) != 0
    }

    /// Moves the current foreground value to the background slot, dropping any
    /// previous background.
    ///
    /// This operation updates only internal bits and memory slots. The value
    /// itself is not cloned or moved in memory. If a background value already
    /// exists, it is dropped before being replaced.
    #[inline]
    fn push_fg_to_bg(&mut self) {
        let bgi = self.bg_index();
        if self.is_slot_present(bgi) {
            unsafe {
                self.slots[bgi].assume_init_drop();
            }
        }
        self.flip();
    }

    /// Flip the foreground/background logical mapping
    #[inline]
    fn flip(&mut self) {
        self.bits ^= FG_SLOT;
    }
}

impl<V> Drop for Overlay<V> {
    fn drop(&mut self) {
        if (self.bits & SLOT0_PRESENT) != 0 {
            unsafe { self.slots[0].assume_init_drop() };
        }

        if (self.bits & SLOT1_PRESENT) != 0 {
            unsafe { self.slots[1].assume_init_drop() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_get_foreground() {
        let mut map = OverlayMap::<&str, i32>::new();
        assert!(map.fg(&"key").is_none());
        map.push("key", 42);
        assert_eq!(*map.fg(&"key").expect("Entry was just pushed"), 42);
    }

    #[test]
    fn push_and_get_background() {
        let mut map = OverlayMap::<&str, i32>::new();
        assert!(map.fg(&"key").is_none());
        map.push("key", 99);
        assert_eq!(*map.fg(&"key").expect("Entry was just pushed"), 99);
    }

    #[test]
    fn push_if_no_change_foreground() {
        let mut map = OverlayMap::<&str, i32>::new();
        map.push("key", 100);

        // Try swap but do nothing
        map.push_if(&"key", |old| {
            if *old == 100 {
                None // no change
            } else {
                Some(999)
            }
        });

        // Still foreground = 100, no background
        assert_eq!(*map.fg(&"key").expect("Entry should still exist"), 100);
    }

    #[test]
    fn push_if_foreground_to_background() {
        let mut map = OverlayMap::<&str, i32>::new();
        map.push("key", 50);
        map.push_if(&"key", |old| if *old == 50 { Some(123) } else { None });
        assert_eq!(*map.fg(&"key").expect("Entry should still exist"), 123);
        assert_eq!(
            *map.bg(&"key").expect("Old value not found in background"),
            50
        );
    }

    #[test]
    fn push_if_missing_key() {
        let mut map = OverlayMap::<&str, i32>::new();
        map.push_if(&"missing", |_| Some(999));
        assert!(map.fg(&"missing").is_none());
    }

    #[test]
    fn pull_with_remainder_test() {
        let mut map = OverlayMap::<&str, i32>::new();
        map.push("key", 42);
        map.push("key", 24);
        assert_eq!(Some(&24), map.fg(&"key"));
        assert_eq!(Some(&42), map.bg(&"key"));
        assert_eq!(Some(24), map.pull(&"key"));
        assert_eq!(Some(&42), map.fg(&"key"));
        assert_eq!(None, map.bg(&"key"));
        assert_eq!(1, map.len());
    }

    #[test]
    fn pull_without_remainder_test() {
        let mut map = OverlayMap::<&str, i32>::new();
        map.push("key", 42);
        assert_eq!(Some(&42), map.fg(&"key"));
        assert_eq!(Some(42), map.pull(&"key"));
        assert_eq!(None, map.fg(&"key"));
        assert_eq!(None, map.bg(&"key"));
        assert_eq!(0, map.len());
    }

    #[test]
    fn swap_test() {
        let mut map = OverlayMap::<&str, i32>::new();
        map.push("key", 42);
        assert_eq!(*map.fg(&"key").expect("Entry was just pushed"), 42);
        assert_eq!(None, map.swap("key", 100));
        let old_value = map.swap("key", 150);
        assert_eq!(old_value, Some(42));
        assert_eq!(*map.fg(&"key").expect("Entry was just pushed"), 150);
        assert_eq!(*map.bg(&"key").expect("Entry was just pushed"), 100);
    }

    #[test]
    fn swap_if_test() {
        let mut map = OverlayMap::<&str, i32>::new();
        map.push("key", 50);
        map.push("key", 100);
        let evicted = map.swap_if(&"key", |old| if *old == 100 { Some(123) } else { None });
        assert_eq!(*map.fg(&"key").expect("Entry should still exist"), 123);
        assert_eq!(*map.bg(&"key").expect("Entry should still exist"), 100);
        assert_eq!(evicted, Some(50));
    }

    #[test]
    fn overlay_test() {
        // Initialize an OverlayMap with:
        // - "fg_key" in foreground
        // - "bg_key" in background
        // - "absent_key" absent
        let mut map = OverlayMap::<&str, i32>::new();
        map.push("fg_key", 10);
        map.push("bg_key", 20);

        // Prepare updates:
        // - "fg_key" -> 100
        // - "bg_key" -> 200
        // - "none_key" -> 300 (was absent in map)
        let updates = vec![("fg_key", 100), ("bg_key", 200), ("none_key", 300)];

        // Perform the merge
        map.overlay(updates);

        // Check that "fg_key" was in foreground, so old value (10) moved to background.
        // New value (100) should be in foreground.
        match map.fg(&"fg_key") {
            Some(val) => assert_eq!(*val, 100),
            _ => panic!("Expected 'fg_key' = 100 in foreground"),
        }
        match map.bg(&"fg_key") {
            Some(val) => assert_eq!(*val, 10),
            None => panic!("Expected old 'fg_key' value = 10 in background"),
        }

        // Check that "none_key" was absent, so it is now in the foreground with 300
        match map.fg(&"none_key") {
            Some(val) => assert_eq!(*val, 300),
            _ => panic!("Expected 'none_key' = 300 in foreground"),
        }
        // It shouldn't exist in background
        assert!(map.bg(&"none_key").is_none());
    }
}
