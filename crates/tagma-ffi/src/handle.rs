//! Validated index handles for the C ABI (task kooky-snub).
//!
//! The ABI used to hand out `Box::into_raw` pointers as `void *`. Nothing
//! about such a value distinguishes a live index from one already freed,
//! from a pointer into unrelated memory, or from a small integer a caller
//! invented: `tagma_index_free` had no choice but to trust it, so
//! double-free and use-after-free were undefined behaviour with no possible
//! diagnostic.
//!
//! Handles are now opaque 64-bit integers naming a slot in a
//! process-global registry, not addresses. The bit layout is
//!
//! ```text
//!   63          48 47                    16 15        0
//!  +--------------+------------------------+-----------+
//!  |  magic (16)  |    generation (32)     | slot (16) |
//!  +--------------+------------------------+-----------+
//! ```
//!
//! which makes the three failure modes detectable:
//!
//! * **Foreign or garbage** — the magic field must match `HANDLE_MAGIC`
//!   and the slot must be one the registry has actually allocated. An
//!   arbitrary integer, a real pointer, or a handle from a different
//!   library fails this. `0` is never a valid handle, so it doubles as the
//!   `NULL`-equivalent failure return of `tagma_index_new`.
//! * **Freed (double-free, use-after-free)** — the slot is empty, so any
//!   handle naming it is rejected.
//! * **Freed then reused (the aliasing case)** — freeing bumps the slot's
//!   generation, so the stale handle's generation no longer matches the one
//!   a later allocation issued for that same slot. This is the case a naive
//!   slot table without a generation gets wrong.
//!
//! # Concurrency
//!
//! The registry is a `Mutex`, and each index sits behind its own `Mutex`
//! inside an `Arc`. Every entry point may therefore be called from any
//! thread, with the same handle or different ones, without external
//! locking; operations on one index serialize, operations on different
//! indexes run concurrently. The registry lock is held only for the
//! O(1) slot bookkeeping — never across a query, an insert, or a `Drop` —
//! so it is not a contention point and cannot deadlock against the
//! per-index lock (the two are never held in a nesting that could invert:
//! the registry lock is always released before the index lock is taken).
//!
//! Freeing a handle another thread is concurrently using is safe and
//! defined: the registry drops its `Arc` immediately (so the handle is
//! dead to every later call), while the in-flight operation keeps the index
//! alive through its own `Arc` and the storage is released when it
//! finishes.
//!
//! # Growth
//!
//! Slots freed by [`release`] go on a free list and are reused, so the
//! registry grows to the caller's peak number of *simultaneously live*
//! indexes, not to the total ever created. That peak is capped at
//! `MAX_SLOTS`; beyond it allocation fails with a defined error rather
//! than growing without bound. A slot whose 32-bit generation would wrap is
//! retired instead of reused, because reuse past a wrap is exactly the
//! aliasing this design exists to prevent.

use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use tagma_core::Index;

/// The integer handle type exposed to C. Opaque: callers must treat it as a
/// token, never as an address or an index. `0` is never a valid handle.
pub type TagmaIndex = u64;

/// Tag stored in the top 16 bits of every handle this library issues.
/// Chosen only to be an implausible value for a real pointer or a small
/// integer; it is a cheap filter, not a security boundary.
/// (`0x5447` is ASCII `TG`.) Note that on every platform tagma targets a
/// real userspace pointer has zeroes in these bits, so a pointer passed by
/// mistake — including one from an older build of this ABI — is rejected
/// rather than silently misread.
const HANDLE_MAGIC: u64 = 0x5447;

const MAGIC_SHIFT: u32 = 48;
const GENERATION_SHIFT: u32 = 16;
const SLOT_MASK: u64 = 0xFFFF;
const GENERATION_MASK: u64 = 0xFFFF_FFFF;

/// Maximum number of simultaneously live indexes, fixed by the 16 bits the
/// layout gives the slot field.
///
/// Crate-private on purpose: it is an implementation limit, not part of the
/// C ABI, and exporting it would put an unprefixed `MAX_SLOTS` macro into
/// every C consumer's namespace.
pub(crate) const MAX_SLOTS: usize = (SLOT_MASK as usize) + 1;

/// Why a handle was rejected. Each variant maps to a distinct message so a
/// caller reading [`crate::tagma_last_error`] can tell the cases apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleError {
    /// The handle is `0`, the reserved never-valid value.
    Null,
    /// The magic field is wrong, or the slot was never allocated: this
    /// value was not issued by tagma.
    Foreign,
    /// The slot is empty or its generation has moved on: the index this
    /// handle named has been freed (double-free or use-after-free).
    Freed,
    /// The index's lock was poisoned by an earlier panic, so its contents
    /// are of unknown validity and it will not be used again.
    Poisoned,
    /// All `MAX_SLOTS` slots are in use.
    Exhausted,
}

impl HandleError {
    /// The message recorded for `tagma_last_error`.
    pub fn message(self) -> String {
        match self {
            HandleError::Null => "ffi: handle is null".to_string(),
            HandleError::Foreign => "ffi: handle was not issued by tagma".to_string(),
            HandleError::Freed => {
                "ffi: handle refers to a freed index (use-after-free or double-free)".to_string()
            }
            HandleError::Poisoned => {
                "ffi: index is poisoned by an earlier panic and is no longer usable".to_string()
            }
            HandleError::Exhausted => {
                format!("ffi: too many live indexes (limit {MAX_SLOTS}); free some first")
            }
        }
    }
}

/// One registry slot. `index` is `None` while the slot is free.
struct Slot {
    generation: u32,
    index: Option<Arc<Mutex<Index>>>,
}

/// Slot table plus the free list that keeps it from growing unboundedly.
struct Registry {
    slots: Vec<Slot>,
    free: Vec<usize>,
}

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(|| {
    Mutex::new(Registry {
        slots: Vec::new(),
        free: Vec::new(),
    })
});

/// Locks the registry, recovering from poisoning.
///
/// Recovery is sound here in a way it is not for an index: no user code and
/// no `Drop` ever runs under this lock, only `Vec` bookkeeping, so a panic
/// cannot leave the slot table half-updated. Propagating poison instead
/// would brick every index in the process over an unrelated panic.
fn registry() -> MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner())
}

/// Packs a slot index and generation into a handle.
fn encode(slot: usize, generation: u32) -> TagmaIndex {
    (HANDLE_MAGIC << MAGIC_SHIFT)
        | ((generation as u64 & GENERATION_MASK) << GENERATION_SHIFT)
        | (slot as u64 & SLOT_MASK)
}

/// Splits a handle into `(slot, generation)`, rejecting anything whose
/// magic field this library did not write.
fn decode(handle: TagmaIndex) -> Result<(usize, u32), HandleError> {
    if handle == 0 {
        return Err(HandleError::Null);
    }
    if handle >> MAGIC_SHIFT != HANDLE_MAGIC {
        return Err(HandleError::Foreign);
    }
    let slot = (handle & SLOT_MASK) as usize;
    let generation = ((handle >> GENERATION_SHIFT) & GENERATION_MASK) as u32;
    Ok((slot, generation))
}

/// Registers `index` and returns its handle.
pub fn allocate(index: Index) -> Result<TagmaIndex, HandleError> {
    let cell = Arc::new(Mutex::new(index));
    let mut reg = registry();
    if let Some(slot) = reg.free.pop() {
        let generation = reg.slots[slot].generation;
        reg.slots[slot].index = Some(cell);
        return Ok(encode(slot, generation));
    }
    if reg.slots.len() >= MAX_SLOTS {
        return Err(HandleError::Exhausted);
    }
    let slot = reg.slots.len();
    reg.slots.push(Slot {
        generation: 1,
        index: Some(cell),
    });
    Ok(encode(slot, 1))
}

/// Resolves a handle to its index, cloning the `Arc` out from under the
/// registry lock so the caller can work without holding it.
fn resolve(handle: TagmaIndex) -> Result<Arc<Mutex<Index>>, HandleError> {
    let (slot, generation) = decode(handle)?;
    let reg = registry();
    let Some(entry) = reg.slots.get(slot) else {
        // A well-formed handle naming a slot that was never allocated can
        // only have been fabricated.
        return Err(HandleError::Foreign);
    };
    if entry.generation != generation {
        return Err(HandleError::Freed);
    }
    match &entry.index {
        Some(cell) => Ok(Arc::clone(cell)),
        None => Err(HandleError::Freed),
    }
}

/// Runs `f` against the index named by `handle`.
///
/// The registry lock is released before `f` runs; only the per-index lock
/// is held across it.
pub fn with_index<T>(
    handle: TagmaIndex,
    f: impl FnOnce(&mut Index) -> T,
) -> Result<T, HandleError> {
    let cell = resolve(handle)?;
    let mut guard = cell.lock().map_err(|_| HandleError::Poisoned)?;
    Ok(f(&mut guard))
}

/// Invalidates `handle` and drops its index.
///
/// The `Arc` is moved out and dropped *after* the registry lock is
/// released, so an arbitrarily expensive `Drop` never blocks other threads'
/// handle operations.
pub fn release(handle: TagmaIndex) -> Result<(), HandleError> {
    let (slot, generation) = decode(handle)?;
    let taken = {
        let mut reg = registry();
        let Some(entry) = reg.slots.get_mut(slot) else {
            return Err(HandleError::Foreign);
        };
        if entry.generation != generation || entry.index.is_none() {
            return Err(HandleError::Freed);
        }
        let taken = entry.index.take();
        // On generation exhaustion the slot is deliberately NOT returned to
        // the free list: retiring it costs a few bytes of table forever,
        // whereas reusing it would let a 4-billion-allocations-old handle
        // alias a live index — the exact bug this module exists to prevent.
        if let Some(next) = entry.generation.checked_add(1) {
            entry.generation = next;
            reg.free.push(slot);
        }
        taken
    };
    drop(taken);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_handle_resolves() {
        let h = allocate(Index::new()).expect("slots available");
        assert!(with_index(h, |idx| idx.add_line("a urgent")).is_ok());
        release(h).expect("live handle frees");
    }

    #[test]
    fn zero_is_never_valid() {
        assert_eq!(decode(0), Err(HandleError::Null));
        assert_eq!(release(0), Err(HandleError::Null));
    }

    #[test]
    fn a_fabricated_handle_is_foreign() {
        // Right shape, wrong magic.
        assert_eq!(decode(1), Err(HandleError::Foreign));
        assert_eq!(decode(0xDEAD_BEEF_CAFE_F00D), Err(HandleError::Foreign));
        // Correct magic, slot never allocated.
        let never = encode(MAX_SLOTS - 1, 1);
        assert_eq!(resolve(never).unwrap_err(), HandleError::Foreign);
    }

    #[test]
    fn double_free_is_reported() {
        let h = allocate(Index::new()).expect("slots available");
        release(h).expect("first free succeeds");
        assert_eq!(release(h), Err(HandleError::Freed));
    }

    #[test]
    fn use_after_free_is_reported() {
        let h = allocate(Index::new()).expect("slots available");
        release(h).expect("first free succeeds");
        assert_eq!(
            with_index(h, |_| ()).unwrap_err(),
            HandleError::Freed,
            "a freed handle must not resolve"
        );
    }

    /// The generation-reuse case: the slot comes back, the handle must not.
    #[test]
    fn a_stale_handle_does_not_alias_the_slot_that_replaced_it() {
        let first = allocate(Index::new()).expect("slots available");
        release(first).expect("free");
        let second = allocate(Index::new()).expect("slots available");
        assert_ne!(first, second, "reused slot must issue a distinct handle");
        assert_eq!(with_index(first, |_| ()).unwrap_err(), HandleError::Freed);
        assert_eq!(release(first), Err(HandleError::Freed));
        release(second).expect("the live handle still works");
    }

    /// Without a free list the table would grow by one slot per
    /// create/destroy cycle and a long-running host would exhaust the
    /// handle space. The bound is loose because the registry is global and
    /// the rest of the suite runs concurrently against it; growing by less
    /// than a tenth of the cycle count still could not happen without
    /// reclamation.
    #[test]
    fn released_slots_are_reused_rather_than_accumulating() {
        const CYCLES: usize = 1000;
        let before = registry().slots.len();
        for _ in 0..CYCLES {
            let h = allocate(Index::new()).expect("slots available");
            release(h).expect("free");
        }
        let after = registry().slots.len();
        assert!(
            after - before < CYCLES / 10,
            "free list must bound growth: {before} -> {after} over {CYCLES} cycles"
        );
    }

    #[test]
    fn handles_are_usable_from_several_threads_at_once() {
        let h = allocate(Index::new()).expect("slots available");
        std::thread::scope(|s| {
            for t in 0..8 {
                s.spawn(move || {
                    for i in 0..64 {
                        with_index(h, |idx| idx.add_line(&format!("id{t}_{i} urgent")))
                            .expect("handle stays valid")
                            .expect("line parses");
                    }
                });
            }
        });
        let ids = with_index(h, |idx| idx.query("urgent")).expect("handle valid");
        assert_eq!(ids.expect("query compiles").len(), 8 * 64);
        release(h).expect("free");
    }
}
