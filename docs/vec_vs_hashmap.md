# Systems-Level Guide: Vec, HashMap, and Data Structure Selection for Latency-Critical Rust

## Part 1 — The Hardware You're Programming Against

Before discussing any data structure, you need to understand the machine. Every data structure decision is ultimately a bet on how the CPU, memory hierarchy, and OS will handle your access patterns.

### 1.1 The Memory Hierarchy

Modern CPUs don't talk to RAM directly. They talk to a cache hierarchy, and the latency differences are staggering:

```
┌─────────────────────────────────────────────────────────────────────┐
│  Level       │  Typical Size    │  Latency       │  Bandwidth      │
├──────────────┼──────────────────┼────────────────┼─────────────────┤
│  Register    │  ~1 KB           │  0 cycles      │  —              │
│  L1 Cache    │  32–64 KB/core   │  ~1 ns (4 cyc) │  ~500 GB/s      │
│  L2 Cache    │  256 KB–1 MB     │  ~3–5 ns       │  ~200 GB/s      │
│  L3 Cache    │  8–64 MB shared  │  ~10–20 ns     │  ~100 GB/s      │
│  DRAM        │  16–512 GB       │  ~50–100 ns    │  ~50 GB/s       │
│  NVMe SSD    │  TB range        │  ~10–25 μs     │  ~7 GB/s        │
│  Network     │  —               │  ~25–500 μs    │  ~12.5 GB/s     │
└─────────────────────────────────────────────────────────────────────┘
```

The key insight: **L1 is 50–100× faster than DRAM**. A cache miss on the hot path doesn't just cost you one lookup — it stalls the entire CPU pipeline for ~50–100 ns while the memory controller fetches 64 bytes from main memory. During that stall, a 4 GHz core could have executed 200–400 instructions.

### 1.2 Cache Lines — The Unit of Memory Movement

The CPU never reads a single byte from memory. It reads in **cache lines**, which are 64 bytes on all modern x86-64 and ARM processors.

```
Address:  0x00  0x08  0x10  0x18  0x20  0x28  0x30  0x38
         ┌─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────┐
Line 0:  │  a  │  b  │  c  │  d  │  e  │  f  │  g  │  h  │  64 bytes
         └─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┘
         ┌─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────┐
Line 1:  │  i  │  j  │  k  │  l  │  m  │  n  │  o  │  p  │  64 bytes
         └─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┘
```

This means:

- If you access `a`, the CPU loads `a` through `h` into cache for free. Accessing `b` next is essentially zero-cost.
- If you access `a`, then jump to some random address 10 KB away, you pay the full DRAM latency again.
- **Contiguous, sequential access patterns exploit this perfectly. Random access patterns waste it entirely.**

The hardware prefetcher detects sequential access patterns (stride-1 or stride-N) and speculatively loads the next cache lines before you request them. A `Vec` iterated linearly triggers the prefetcher. A `HashMap` with scattered buckets defeats it.

### 1.3 TLB and Virtual Memory Pages

The OS maps your process's virtual addresses to physical RAM through **page tables**. The CPU caches recent translations in the **Translation Lookaside Buffer (TLB)**:

```
Virtual Address  →  [TLB Lookup]  →  Physical Address  →  [Cache Lookup]  →  Data
                     (~0.5–1 ns        miss: ~10–50 ns
                      hit)              page walk)
```

- Standard pages: 4 KB
- Huge pages: 2 MB (Linux `madvise(MADV_HUGEPAGE)` or `mmap` with `MAP_HUGETLB`)
- Gigantic pages: 1 GB

TLB capacity is small (~64–1536 entries depending on level). With 4 KB pages, 1024 TLB entries cover only 4 MB. If your data structure spans 100 MB with random access, you'll get constant TLB misses — each costing ~10–50 ns for a page table walk.

**Implication for data structures**: A 5.6 MB flat array (65K × 88 bytes) spans ~1,400 standard pages. If accesses are random (many different symbols), you may TLB-miss frequently. With 2 MB huge pages, the same array spans only 3 pages — virtually eliminating TLB misses. This is why lithos mmap work with huge pages matters.

### 1.4 Branch Prediction and Speculative Execution

Modern CPUs speculatively execute instructions past branches before knowing the branch outcome. When the prediction is wrong, the pipeline flushes — costing ~15–20 cycles (~4–5 ns).

Every `if`, `match`, `Option::unwrap()`, `Result::unwrap()`, bounds check, or hash probe comparison is a branch. On the hot path, eliminating branches is eliminating latency.

```
// This has a branch (bounds check + Option discriminant):
if let Some(state) = states.get(id) { ... }

// This has one branch (bounds check, elided in release with get_unchecked):
let state = &states[id as usize];

// This has zero branches (unsafe, no bounds check):
let state = unsafe { states.get_unchecked(id as usize) };
```

---

## Part 2 — How `Vec<T>` Works at the Systems Level

### 2.1 Memory Layout

A `Vec<T>` is three words on the stack (24 bytes on 64-bit):

```
Stack (Vec metadata):            Heap (contiguous buffer):
┌──────────┐                     ┌────┬────┬────┬────┬────┬─────────┐
│ ptr ─────────────────────────→ │ T₀ │ T₁ │ T₂ │ T₃ │ T₄ │ (unused)│
│ len: 5   │                     └────┴────┴────┴────┴────┴─────────┘
│ cap: 8   │                     ◄──── capacity × sizeof(T) ────────►
└──────────┘
```

- `ptr`: Raw pointer to a heap-allocated buffer, aligned to `align_of::<T>()`
- `len`: Number of initialized elements (0 ≤ len ≤ cap)
- `cap`: Total allocated slots

The buffer is a **single contiguous allocation** from the global allocator (typically jemalloc or the system allocator, which calls `mmap`/`brk` under the hood).

### 2.2 Indexing — What Actually Happens

```rust
let x = &vec[i];
```

Compiles to (conceptually):

```
1. Bounds check:  if i >= vec.len { panic!("index out of bounds") }   // 1 branch
2. Address calc:  addr = vec.ptr + i * size_of::<T>()                 // 1 multiply + add
3. Load:          read memory at addr                                  // 1 memory access
```

In release mode with `get_unchecked`, steps 1 is removed. The entire operation becomes a single `MOV` or `LEA` instruction with a scaled index — the fastest possible memory access pattern the CPU supports.

For `[T; N]` (fixed-size array stored inline), there's no heap pointer to dereference — the data lives directly in the struct. Even the pointer indirection of `Vec` is eliminated:

```rust
pub struct MarketStateManager {
    markets: [MarketsState; 256],  // 256 × 88 = 22,528 bytes, inline
}

// Access: struct_base_addr + field_offset + (i * 88)
// Single instruction, zero indirection
```

### 2.3 Vec Growth — The Allocation Tax

When `len == cap` and you push, Vec must:

1. Allocate a new buffer of `cap * 2` (or `cap * 2` rounded to allocator size classes)
2. `memcpy` the old buffer to the new one
3. Deallocate the old buffer
4. Update `ptr` and `cap`

This is O(n) and involves a `mmap`/`brk` syscall for large buffers — potentially microseconds. On the hot path, **this must never happen**.

The fix: pre-allocate to the maximum needed capacity at startup.

```rust
let mut v = Vec::with_capacity(MAX_SYMBOLS);  // One allocation, done forever
```

Or better: use a fixed-size array `[T; N]` which never allocates at all.

### 2.4 Vec Size Limits

Theoretical maximum size of a `Vec<T>`:

```
Max elements = isize::MAX / size_of::<T>()
             = (2^63 - 1) / sizeof(T)
```

Rust enforces that no single allocation exceeds `isize::MAX` bytes (~9.2 exabytes on 64-bit). In practice, you're bounded by:

- **Physical RAM**: `Vec::with_capacity(n)` needs `n × sizeof(T)` contiguous bytes
- **Virtual address space**: 48-bit addressing = 256 TB (Linux default); 57-bit = 128 PB (5-level paging)
- **Allocator limits**: glibc malloc can handle up to ~64 TB on 64-bit Linux
- **Overcommit policy**: Linux by default overcommits (`/proc/sys/vm/overcommit_memory = 0`), so `Vec::with_capacity(huge)` may succeed but OOM-kill when you actually touch the pages.

Practical limits by element count and type size:

```
sizeof(T)   Max elements (8 GB RAM)    Vec size
─────────   ───────────────────────    ────────
1 byte      ~8 billion                 8 GB
8 bytes     ~1 billion                 8 GB
88 bytes    ~90 million                ~8 GB
1 KB        ~8 million                 8 GB
```

For MarketStateManager: 65,536 × 88 bytes = 5.6 MB. This is trivial — 0.07% of 8 GB RAM.

### 2.5 When Vec Excels

Vec is the optimal choice when:

1. **Keys are dense integers starting near 0** (can be used as direct indices)
2. **Access pattern is sequential or random within a bounded range**
3. **The dataset fits in cache** (L1: <64 KB, L2: <1 MB, L3: <32 MB)
4. **No insertions/deletions on the hot path** (or only at the end — `push`/`pop`)
5. **Element count is known or bounded at startup**

Vec is suboptimal when:

1. Keys are sparse, non-integer, or unbounded (you'd waste enormous memory)
2. Frequent insertions/deletions in the middle (O(n) shifts)
3. The collection must grow unpredictably during performance-critical sections

---

## Part 3 — How `HashMap<K, V>` Works at the Systems Level

### 3.1 Rust's HashMap Implementation — hashbrown (Swiss Table)

Since Rust 1.36, `std::collections::HashMap` uses **hashbrown**, Google's Swiss Table design. This is fundamentally different from textbook separate-chaining hash maps.

#### Memory Layout

```
Control bytes (metadata):    Slots (key-value storage):
┌───┬───┬───┬───┬───┬───┐   ┌──────────┬──────────┬──────────┬──────────┐
│ h₀│ h₁│ E │ h₃│ D │ E │   │ (K₀,V₀)  │ (K₁,V₁)  │ (empty)  │ (K₃,V₃)  │
└───┴───┴───┴───┴───┴───┘   └──────────┴──────────┴──────────┴──────────┘
 1 byte each                  sizeof(K) + sizeof(V) each

 h = top 7 bits of hash      E = EMPTY (0xFF)
                              D = DELETED (0x80)
```

The layout is split into two arrays:

- **Control bytes**: 1 byte per slot. Stores either `EMPTY` (0xFF), `DELETED` (0x80), or the top 7 bits of the hash (0x00–0x7F). Grouped into **groups of 16** for SIMD probing.
- **Slots**: `(K, V)` pairs stored in a separate contiguous array, aligned to the larger of `align_of::<K>()` and `align_of::<V>()`.

Total heap allocation: `capacity × (1 + sizeof(K) + sizeof(V))` plus alignment padding.

#### 3.2 The Lookup Procedure — Step by Step

When you call `map.get(&key)`:

```
Step 1: Hash the key
        ────────────
        h = SipHash::hash(key)           // SipHash-1-3: ~15–25 ns
                                          // or FxHash: ~3–5 ns
        h1 = h >> 57                      // top 7 bits → control byte match
        h2 = h & (capacity - 1)          // low bits → starting group index

Step 2: Probe groups (SIMD accelerated)
        ──────────────────────────────────
        group_index = h2 / 16
        loop {
            ctrl_group = load_16_bytes(&control[group_index * 16])
                                          // 1 cache line holds 64 control bytes = 4 groups
            
            // SIMD: compare h1 against all 16 control bytes simultaneously
            matches = _mm_cmpeq_epi8(ctrl_group, splat(h1))
            match_mask = _mm_movemask_epi8(matches)
                                          // match_mask is a 16-bit bitmask
            
            // For each set bit in match_mask:
            for bit in match_mask.iter_ones() {
                slot_index = group_index * 16 + bit
                if slots[slot_index].key == key {   // ← Full key comparison
                    return Some(&slots[slot_index].value)
                }
            }
            
            // Check if group has any EMPTY slots (means key doesn't exist)
            empties = _mm_cmpeq_epi8(ctrl_group, splat(EMPTY))
            if _mm_movemask_epi8(empties) != 0 {
                return None
            }
            
            group_index = (group_index + 1) & group_mask   // Linear probing
        }
```

Even in the best case (key found in the first group, first match), this involves:

1. **Hash computation**: 15–25 ns (SipHash) or 3–5 ns (FxHash)
2. **Control byte load**: 1 memory access (likely L1 hit if table is hot)
3. **SIMD comparison**: 1–2 cycles
4. **Slot load**: 1 memory access (may be L1 miss — slots are in a different cache region than control bytes)
5. **Key comparison**: 1 branch + compare

Worst case (many collisions, long probe chain): multiply steps 2–5 by the probe length, with each group potentially causing a cache miss.

### 3.3 SipHash vs FxHash — The Hasher Tax

**SipHash-1-3** (std default):
- Designed to resist HashDoS (adversarial collision attacks)
- 2 rounds of SipRound per 8-byte block + 1 finalization round
- Cost: ~15–25 ns for small keys
- Necessary when keys come from untrusted input (web servers, network protocols)
- Completely unnecessary when keys are `u16` SymbolIds assigned by your own system

**FxHash** (multiply-shift):
- `hash = key.wrapping_mul(0x517cc1b727220a95)` — one multiply instruction
- Cost: ~1–3 ns for small keys
- Zero collision resistance — easily defeated by adversarial input
- Perfect for integer keys in trusted, performance-critical contexts

**For SymbolId (u16)**: Even FxHash is wasted computation. The hash of a `u16` is a u64, which is then masked to get a bucket index. But the `u16` *is already* a valid index into a 65K-slot array. You're computing a function to scatter keys into buckets when they already map 1:1 to array positions.

### 3.4 Capacity, Load Factor, and Resizing

HashMap maintains a **load factor** — the ratio of occupied slots to total capacity:

```
load_factor = len / capacity
```

hashbrown resizes when `load_factor > 7/8` (87.5%). Resizing involves:

1. Allocate a new table with `2× capacity`
2. Rehash every existing key-value pair
3. Insert each pair into the new table
4. Deallocate the old table

For a HashMap with 10,000 entries, resize means:

```
10,000 × (hash_cost + insert_cost) = 10,000 × ~30 ns = ~300 μs
```

Plus the allocation cost (potentially a `mmap` syscall: ~1–10 μs).

This is a **latency spike** — totally unpredictable from the caller's perspective. In HFT, a 300 μs stall during a market data burst means thousands of missed or stale price updates.

Mitigation: `HashMap::with_capacity(n)` pre-allocates, but you must know `n` upfront and it still over-allocates by `1/0.875 ≈ 1.14×` to stay under the load factor.

### 3.5 HashMap Size Limits

Maximum capacity: bounded by the same `isize::MAX` rule as `Vec`:

```
Max capacity = isize::MAX / (1 + sizeof(K) + sizeof(V) + padding)
```

Memory overhead per entry (hashbrown):

```
Per entry = sizeof(K) + sizeof(V) + 1 byte (control) + padding
          + amortized empty slot cost (table is 87.5% full max → ~14% waste)
```

For `HashMap<u16, MarketsState>` (K=2 bytes + 6 padding, V=88 bytes, control=1 byte):

```
Per entry ≈ 8 + 88 + 1 = 97 bytes effective
+ ~14% waste from load factor
= ~110 bytes per entry

For 65,536 entries: ~7.2 MB (vs 5.6 MB for flat array)
```

The overhead seems small in absolute terms, but the constant factor on lookup — hashing + probing + branching — is what kills you, not the memory footprint.

### 3.6 When HashMap Excels

HashMap is the right choice when:

1. **Keys are sparse, non-integer, or unbounded** — strings, UUIDs, composite keys where direct indexing is impossible
2. **Key space is enormous relative to active entries** — e.g., 1,000 active users out of 2^64 possible user IDs
3. **Keys come from untrusted input** (use SipHash for DoS resistance)
4. **Insertion and deletion are frequent** — HashMap handles this in amortized O(1)
5. **You don't know the maximum size upfront** — HashMap grows dynamically

HashMap is the wrong choice when:

1. Keys are small dense integers (use array/Vec indexing)
2. You're on a latency-critical hot path (hash + probe + branch overhead)
3. Deterministic worst-case latency is required (resize spikes)
4. Cache efficiency is paramount (scattered memory access pattern)

---

## Part 4 — Other Data Structures in the Spectrum

### 4.1 `BTreeMap<K, V>` — Cache-Friendly Ordered Map

Memory layout: B-tree nodes with branching factor ~11 (each node holds up to B-1 keys). Nodes are heap-allocated and connected by pointers.

```
                    ┌──────────────────┐
                    │ [k3, k7, k12]    │   Root node
                    │ [p0, p1, p2, p3] │   Child pointers
                    └──┬────┬────┬──┬──┘
                       │    │    │  │
              ┌────────┘    │    │  └────────┐
              ▼             ▼    ▼           ▼
         ┌─────────┐  ┌────────┐  ┌────────┐  ┌────────┐
         │ [k1,k2] │  │[k4,k5] │  │[k8,k9] │  │[k13..] │
         └─────────┘  └────────┘  └────────┘  └────────┘
```

- Lookup: O(log n) — but each node access may be a cache miss
- For n=65,536: ~4 levels → ~4 potential cache misses → ~200–400 ns
- Ordered iteration is efficient (in-order traversal)
- Insertion: O(log n) with occasional node splits

**Use when**: You need ordered iteration, range queries, or `floor`/`ceiling` operations. Never on a nanosecond-sensitive hot path.

### 4.2 `Vec<Option<T>>` — Direct Indexing with Presence Tracking

```
Memory layout (Vec<Option<T>> where T is 88 bytes):

Index: [  0  ] [  1  ] [  2  ] [  3  ] [  4  ]
       ┌──┬───┬──┬───┬──┬───┬──┬───┬──┬───┐
       │D │ T │D │ T │D │pad│D │ T │D │pad│
       │1 │88B│1 │88B│0 │88B│1 │88B│0 │88B│
       └──┴───┴──┴───┴──┴───┴──┴───┴──┴───┘
        ▲                 ▲
        D = discriminant   Empty slot (None): discriminant = 0, padding = wasted
        (1 byte + 7 pad)
```

`Option<T>` uses the **niche optimization** for some types (e.g., `Option<&T>` is the same size as `&T` because null is the niche). But for arbitrary structs like `MarketsState`, the discriminant adds 1 byte + alignment padding — potentially 8 bytes per element.

More importantly, every access requires:

```rust
match &vec[i] {
    Some(state) => { /* use state */ }    // Branch: is discriminant 1?
    None => { /* handle missing */ }      // Branch: is discriminant 0?
}
```

On a hot path with millions of events, that branch is a guaranteed ~0.5–1 ns tax per lookup — even with perfect branch prediction (because the CPU still must check the prediction was correct).

**Use when**: Some indices may genuinely be unoccupied AND you need to distinguish "present" from "absent" at the type level. Not for pre-initialized dense collections.

### 4.3 Bitset (e.g., `bitvec`, `fixedbitset`) — Compact Presence Tracking

```
For 65,536 symbols:
  u64 array: [u64; 1024]  →  8 KB
  
  Symbol 1234 active?
    word  = 1234 / 64 = 19
    bit   = 1234 % 64 = 18
    active = (bits[19] >> 18) & 1   // Single shift + AND
```

Cost: 8 KB for the entire symbol universe. Fits in L1 cache. Iteration over set bits uses `count_trailing_zeros` (hardware instruction, 1 cycle) to skip to the next active symbol in O(1).

**Use for**: The secondary "active symbols" index in MarketStateManager — tracking which of the 65K slots are actually subscribed without polluting the hot path with Option discriminants.

### 4.4 `SlotMap` / Arena Allocators

For entity systems where objects are created/destroyed frequently (order book entries, connection handles), a SlotMap provides:

- Generational indices (catch use-after-free bugs)
- Dense internal storage for cache-friendly iteration
- O(1) insert, remove, and lookup

```
SlotMap internal:
  entries: Vec<Entry<T>>         // Dense storage
  slots:   Vec<(generation, index)>  // Sparse index → dense index mapping
```

**Use when**: You need stable handles to dynamically created/destroyed objects AND cache-friendly iteration over all live objects. Order books, connection pools, entity-component systems.

---

## Part 5 — The Full Picture: How It All Fits in OnyxEngine

### 5.1 Data Flow and Access Patterns

```
Market Data Feed (UDP multicast)
        │
        ▼
┌─────────────────────┐
│  Network I/O Layer   │   io_uring / epoll
│  (kernel → userspace)│   Latency: ~1–5 μs (kernel bypass: ~100 ns)
└────────┬────────────┘
         │  Raw bytes
         ▼
┌─────────────────────┐
│  Deserializer        │   Zero-copy parse: read fields from buffer directly
│  (bytes → TopOfBook) │   Latency: ~20–50 ns
└────────┬────────────┘
         │  TOB event with SymbolId
         ▼
┌──────────────────────────────────────────────────┐
│  MarketStateManager                               │
│                                                    │
│  markets: [MarketsState; 256]                     │
│           ▲                                        │
│           │  self.markets[symbol_id.0 as usize]   │
│           │  = single indexed memory access        │
│           │  Latency: <1 ns (L1 hit)               │
│                                                    │
│  active_symbols: FixedBitSet (256 bits = 32 bytes)│
│  → only used for iteration/display, never on hot   │
│    path                                            │
└────────┬──────────────────────────────────────────┘
         │  Updated MarketsState
         ▼
┌─────────────────────┐
│  Matching Engine     │   Price-time priority matching
│  (order book ops)    │   Latency target: <1 μs
└────────┬────────────┘
         │  Fills, cancels, acks
         ▼
┌─────────────────────┐
│  Order Manager       │   Position tracking, risk checks
│  + Risk Engine       │   Uses its own flat arrays indexed by SymbolId
└─────────────────────┘
```

Every component on the hot path uses the same principle: **if the key is a small dense integer, index directly. Reserve HashMap for configuration, setup, and cold paths.**

### 5.2 Hot Path vs Cold Path Classification

```
HOT PATH (millions/sec, nanosecond budget):
├── Market state lookup by SymbolId          → flat array index
├── Order book price level lookup            → sorted Vec or BTreeMap (if needed)
├── Sequence number generation               → atomic u64 increment
├── Timestamp capture                        → rdtscp (single instruction)
└── Lock-free queue push/pop                 → Disruptor ring buffer (array)

WARM PATH (thousands/sec, microsecond budget):
├── New order validation                     → HashMap for account lookup is OK
├── WebSocket message serialization          → serde (allocation acceptable)
├── Risk limit checks                        → flat array by SymbolId
└── Logging                                  → async channel, batched I/O

COLD PATH (per second or less, millisecond budget):
├── Symbol subscription/unsubscription       → HashMap, BTreeMap, whatever
├── Config reload                            → file I/O, JSON parse
├── Connection management                    → HashMap<ConnectionId, Session>
├── Dashboard/monitoring data aggregation    → iterate active_symbols bitset
└── Kafka publishing                         → buffered, async
```

### 5.3 Decision Framework — What to Use When

```
START
  │
  ├── Is the key a small, dense integer (u8/u16/u32)?
  │     │
  │     ├── YES: Is the max key value small enough to allocate? (< ~10M entries)
  │     │     │
  │     │     ├── YES: ──→  Vec<T> or [T; N] indexed by key
  │     │     │             ✓ O(1), zero overhead, cache-perfect
  │     │     │             Memory: max_key × sizeof(T)
  │     │     │
  │     │     └── NO:  ──→  FxHashMap<K, V> (or perfect hash if keys are static)
  │     │                   Key space too large for direct indexing
  │     │
  │     └── NO: Is it on the hot path?
  │           │
  │           ├── YES: ──→  FxHashMap<K, V>
  │           │             Fastest general-purpose map, no DoS resistance needed
  │           │             if keys are trusted
  │           │
  │           └── NO:  ──→  std::HashMap<K, V>
  │                         Safe default, DoS-resistant, fine for cold paths
  │
  ├── Do you need ordered iteration or range queries?
  │     │
  │     └── YES: ──→  BTreeMap<K, V>
  │                   O(log n) but ordered, range scans are efficient
  │
  ├── Do you need to track presence/absence of items in a set?
  │     │
  │     ├── Small integer keys: ──→  BitSet / FixedBitSet
  │     │                            1 bit per key, SIMD-friendly iteration
  │     │
  │     └── Other keys: ──→  HashSet<K>
  │
  └── Do you have dynamic entity creation/destruction with stable handles?
        │
        └── YES: ──→  SlotMap / Arena
                      Generational indices, dense iteration
```

### 5.4 Concrete Sizing Guidelines

| Active Symbols | Structure | Memory | Cache Level | Lookup Cost |
|---|---|---|---|---|
| ≤ 256 | `[T; 256]` | ~22 KB | Fits L1 | <1 ns guaranteed |
| ≤ 4,096 | `[T; 4096]` | ~352 KB | Fits L2 | <1–3 ns |
| ≤ 65,536 | `Vec<T>` pre-alloc | ~5.6 MB | Fits L3 | <1–10 ns depending on access pattern |
| ≤ 1M | `Vec<T>` pre-alloc | ~88 MB | Exceeds L3, DRAM | ~50–100 ns on miss |
| > 1M sparse | `FxHashMap` | ~110 bytes/entry | Depends on working set | ~5–15 ns |
| > 10M sparse | `FxHashMap` + sharding | ~110 bytes/entry | Definitely DRAM | ~15–50 ns |

The crossover point where HashMap beats a flat array: **when the key space is so large that the flat array doesn't fit in L3 cache AND access is sparse.** For u16 keys, this never happens — 5.6 MB always fits in L3.

### 5.5 Hardware-Specific Optimizations

**Cache Line Alignment**: Ensure `MarketsState` starts on a cache line boundary to avoid split-line loads:

```rust
#[repr(C, align(64))]  // Align to cache line boundary
pub struct MarketsState {
    // Hot fields (accessed on every TOB update) — first 64 bytes
    pub mid_x2: i64,
    pub spread_ticks: i64,
    pub last_update_ns: u64,
    pub symbol_id: u16,
    pub _pad: [u8; 38],        // Fill to 64 bytes
    
    // Cold fields (accessed less frequently) — second cache line
    pub last_tob: TopOfBook,   // 48 bytes
}
```

This ensures that the most frequently accessed fields are in the same cache line, loaded together in one memory access.

**Prefetch Hints**: For batch processing multiple symbols:

```rust
// When processing symbol i, prefetch symbol i+4's cache line
unsafe {
    let next = &self.markets[(symbol_id + 4) as usize] as *const MarketsState;
    std::arch::x86_64::_mm_prefetch(next as *const i8, std::arch::x86_64::_MM_HINT_T0);
}
```

**Huge Pages**: For the full 65K array (5.6 MB), request 2 MB huge pages to reduce TLB pressure:

```rust
use libc::{mmap, MAP_HUGETLB, MAP_ANONYMOUS, MAP_PRIVATE, PROT_READ, PROT_WRITE};

let size = 65536 * std::mem::size_of::<MarketsState>();
let ptr = unsafe {
    mmap(
        std::ptr::null_mut(),
        size,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS | MAP_HUGETLB,
        -1,
        0,
    )
};
```

---

## Part 6 — Summary: The Systems Perspective

Every data structure choice is fundamentally a negotiation between three resources:

1. **Memory**: How much space does the structure consume?
2. **CPU cycles**: How many instructions per operation?
3. **Cache residency**: Does the access pattern exploit the memory hierarchy or fight it?

For MarketStateManager on VEX-CORE's hot path:

| Resource | Flat `[T; N]` | HashMap |
|---|---|---|
| Memory | 22 KB (N=256) to 5.6 MB (N=65K) — trivial | ~7.2 MB + overhead — similar |
| CPU per lookup | 1–2 instructions | 15–30+ instructions |
| Cache behavior | Perfect sequential layout, prefetcher-friendly | Scattered: control bytes ≠ slot locations, probe chains jump around |
| Worst-case latency | Deterministic: always 1 memory access | Unbounded: resize, long probe chains |
| Branches per lookup | 0 (with unsafe) or 1 (bounds check) | 3–10+ (hash, probe, compare, empty check) |

The flat array wins on every axis that matters for the hot path. HashMap's flexibility — arbitrary keys, dynamic sizing, DoS resistance — are virtues that are simply irrelevant when your keys are `u16` SymbolIds assigned by your own system.

**The rule**: On the hot path, match your data structure to your access pattern and your key type. Don't pay for generality you don't need. Off the hot path, use whatever is clearest and most maintainable — `HashMap` is usually the right default.