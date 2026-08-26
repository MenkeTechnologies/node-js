//! rkyv-backed bytecode cache for compiled JS scripts (mirrors the fleet's
//! pythonrs/zshrs/rubylang design). Every ordinary `node foo.js` run is
//! transparently cached: the source is hashed, the shard consulted, and on a hit
//! the compiled `fusevm::Chunk`s run directly — lex/parse/lower are skipped
//! entirely. On a miss the program is compiled, stored, then run. `node --build`
//! warms the same shard ahead of time.
//!
//! Layout: a single shard at `~/.node-js/scripts.rkyv`. The *outer* container is
//! a zero-copy rkyv archive (`Shard`), validated on load; each *inner* entry blob
//! is a bincode-encoded `CProg` (the compiled `fusevm::Chunk`s + func/try
//! tables), because `fusevm::Chunk` is serde-owned, not `rkyv::Archive`. The key
//! is a 64-bit hash of the source plus a schema version, the release version and
//! an identity for the running BINARY, so a source, format, release or codegen
//! change misses cleanly instead of loading stale bytecode.

use crate::compiler::Program;
use crate::host::{FuncDef, TryDef};
use fusevm::Chunk;
use rkyv::{Archive, Deserialize as RkyvDe, Serialize as RkyvSer};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// Bump on any incompatible change to `CProg` / the lowering / the shard layout.
/// v1: initial JS bytecode cache — a Chunk/func/try layout change here must miss
///     cleanly so an older cached `.js` never loads incompatible bytecode.
/// v2: BigInt/RegExp/tagged-template/for-await lowering — new builtin ops
///     (MKBIGINT/MKREGEX/NUM_STEP/TAG_TMPL/…) and the type-preserving `++`/`--`
///     codegen; old cached bytecode would run the stale POS-based increment.
/// v6: `FuncDef.is_method` (a method owns no `prototype`) and the class-body
///     emission order (methods before static fields). A v5 blob deserializes
///     with `is_method: false` and replays the old source order, so every class
///     and every object method would report the wrong own-property set.
/// v7: NamedEvaluation (10.2.9 SetFunctionName) at every site the grammar calls
///     for it — assignment to an identifier, object property definitions and
///     concise methods/accessors, class fields, and destructuring/parameter
///     defaults — plus the new `NAMED_EVAL` builtin and `DEF_FIELD`'s fourth
///     argument. A v6 blob calls `DEF_FIELD` with three arguments and emits no
///     naming, so every affected function would keep the empty `.name` and the
///     field's flag would be read off the wrong stack slot. v7 also carries the
///     class-body environment (15.7.14 step 17), whose `PUSH_SCOPE`/`DECLARE`
///     pair a v6 blob does not emit, so a static initializer reading the class
///     by name would still throw `ReferenceError`.
/// v8: `**` lowers to `CallBuiltin(ops::POW, 2)` instead of the native
///     `Op::Pow`. fusevm's native op is IEEE-754 `pow`, which answers 1 for
///     `(-1) ** Infinity` and `1 ** NaN` where the spec says NaN; a v7 blob
///     still carries `Op::Pow` and would keep replaying the IEEE answer from
///     cache long after the source fix. (The `Math.*` additions in the same
///     change need no bump: a `Math.f(..)` call emits the name as a constant
///     and dispatches on the string at run time, so `--dump-bytecode` for a
///     known and an unknown method name is byte-identical.)
/// v9: locals that no closure can reach are addressed as fusevm frame slots
///     (`Op::GetSlot`/`SetSlot`) instead of `CallBuiltin(GETLOCAL)` by name —
///     see `crate::slots`. A v8 blob is still CORRECT, since it carries the
///     name-lookup form and nothing else changed about it; it is simply the
///     slow bytecode, and a cache that kept replaying it would hide the whole
///     change from every script already run once. The bump is what makes the
///     speedup reach existing scripts.
/// v10: the entry carries the compiler's SIDE TABLES (call-site texts, yield-site
///     iterator depths). They live in thread-local registries that only
///     `finish_chunk` fills, so every cache hit ran without them: a generator's
///     parked `for…of`/`yield*` iterators were not closed when a `.return()` or
///     `.throw()` was injected, and their `finally` never ran — the same script
///     printed one thing on its first run and another on its second. A v9 blob
///     has no tables to restore, so it must not be replayed.
const SCHEMA: u64 = 10;

/// The outer, rkyv-archived shard: a flat list of (key, bincode-blob) entries.
#[derive(Archive, RkyvSer, RkyvDe, Default)]
#[archive(check_bytes)]
struct Shard {
    entries: Vec<Entry>,
}

#[derive(Archive, RkyvSer, RkyvDe)]
#[archive(check_bytes)]
struct Entry {
    key: u64,
    /// A second, independent hash of the source. A cache hit requires BOTH `key`
    /// and `verify` to match, so an `FxHash` collision on `key` can never return
    /// a different program's bytecode (which would silently produce wrong
    /// results — far worse than a cache miss).
    verify: u64,
    /// The [`build_id`] that wrote this entry. Every key already mixes the build
    /// id in, so an entry from a DIFFERENT build can never be hit again — it is
    /// dead weight from the moment the binary is rebuilt. Recording it lets
    /// `store` drop those entries instead of accumulating one full copy of the
    /// shard per rebuild, which matters because `load_shard` reads and
    /// deserializes the WHOLE file on every lookup.
    build: u64,
    blob: Vec<u8>,
}

/// The inner, serde/bincode form of a compiled program.
#[derive(Serialize, Deserialize)]
struct CProg {
    main: Chunk,
    functions: Vec<(String, FuncDef)>,
    tries: Vec<TryDef>,
    /// The compiler's side tables — call-site texts and yield-site iterator
    /// depths — which a cache hit would otherwise never build. See
    /// [`crate::host::SiteTables`] for what silently degrades without them.
    #[serde(default)]
    sites: crate::host::SiteTables,
}

/// The release this binary was built as, hashed into every cache key so a shard
/// written by one release can never be read by another.
const BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");

/// An identity for the BINARY doing the lookup — its own mtime, read once.
///
/// `SCHEMA` and `BUILD_VERSION` are both bumped BY HAND, and a codegen change
/// that forgets either ships a binary that silently replays the previous
/// build's bytecode for every script already run once. That failure is
/// invisible: the right answer for the old program, no error, and the symptom
/// is "my change did not take". Measured on this crate: with the key depending
/// on `(SCHEMA, src)` alone, lowering `**` to a deliberately wrong opcode and
/// rebuilding still printed the OLD result for a previously-cached script,
/// while a byte-different script printed the new (wrong) one — the binary had
/// changed and the cache had not noticed.
///
/// The mtime changes on every rebuild without anyone having to remember, and is
/// stable for an installed binary, so it costs one `stat` per process and
/// nothing else. `0` when the path or metadata is unreadable, which degrades to
/// the previous behavior rather than failing the run.
fn build_id() -> u64 {
    use std::sync::OnceLock;
    static ID: OnceLock<u64> = OnceLock::new();
    *ID.get_or_init(|| {
        std::env::current_exe()
            .and_then(|p| p.metadata())
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    })
}

/// A stable content key for a source string (fast `FxHash`, used for lookup).
pub fn key_for(src: &str) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    SCHEMA.hash(&mut h);
    BUILD_VERSION.hash(&mut h);
    build_id().hash(&mut h);
    src.hash(&mut h);
    h.finish()
}

/// An independent verification hash (std `DefaultHasher`/SipHash), so a hit
/// requires both hashes to agree — collision-proof for correctness.
fn verify_for(src: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    let mut h = DefaultHasher::new();
    SCHEMA.hash(&mut h);
    BUILD_VERSION.hash(&mut h);
    build_id().hash(&mut h);
    src.len().hash(&mut h);
    src.hash(&mut h);
    h.finish()
}

fn shard_path() -> Option<PathBuf> {
    let dir = dirs::home_dir()?.join(".node-js");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("scripts.rkyv"))
}

fn load_shard() -> Shard {
    let Some(path) = shard_path() else {
        return Shard::default();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Shard::default();
    };
    rkyv::from_bytes::<Shard>(&bytes).unwrap_or_default()
}

fn write_shard(shard: &Shard) -> Result<(), String> {
    let path = shard_path().ok_or("no home dir for cache")?;
    let bytes = rkyv::to_bytes::<_, 4096>(shard).map_err(|e| format!("cache serialize: {e}"))?;
    // Atomic replace (write temp + rename) so a concurrent reader — up to 16
    // instances run against the shared shard — never sees a torn file. A losing
    // concurrent writer just drops its entry (recompiled next run); it can never
    // corrupt the shard. The temp name is unique per WRITE (pid + a monotonic
    // counter), not just per process, so concurrent writers within one process
    // (e.g. parallel test threads) never clobber each other's temp file.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("rkyv.tmp.{}.{n}", std::process::id()));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("cache write: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cache rename: {e}")
    })
}

/// The shard, resident in memory for the life of the process.
///
/// It used to be read and fully deserialized from disk on every `load`, and
/// read-modify-WRITTEN on every `store`. That was affordable only because
/// exactly one lookup happened per run — the top-level script. Caching each
/// `require`d module makes it 118 lookups for an express tree, and measured on
/// a 3 MB shard a single `load` costs 67-347 ms, so the disk-per-call design
/// would have turned a 138 ms saving into a 16 SECOND regression. Reading once
/// and writing once is what makes per-module caching possible at all.
#[derive(Default)]
struct ShardMem {
    entries: rustc_hash::FxHashMap<u64, (u64, Vec<u8>)>,
    /// Whether this process added anything, so an all-hits run writes nothing.
    dirty: bool,
}

thread_local! {
    static SHARD: std::cell::RefCell<Option<ShardMem>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` against the resident shard, loading it from disk on first use.
fn with_shard<T>(f: impl FnOnce(&mut ShardMem) -> T) -> T {
    SHARD.with(|c| {
        let mut slot = c.borrow_mut();
        let mem = slot.get_or_insert_with(|| {
            let build = build_id();
            let mut mem = ShardMem::default();
            for e in load_shard().entries {
                // Entries from another build can never be hit (the build id is
                // part of every key), so they are not worth holding in memory
                // and are dropped on the next write.
                if e.build == build {
                    mem.entries.insert(e.key, (e.verify, e.blob));
                }
            }
            mem
        });
        f(mem)
    })
}

/// Look up a compiled program for `src`, if present and current.
pub fn load(src: &str) -> Option<Program> {
    let key = key_for(src);
    let verify = verify_for(src);
    let blob = with_shard(|m| {
        m.entries
            .get(&key)
            .filter(|(v, _)| *v == verify)
            .map(|(_, b)| b.clone())
    })?;
    let cp: CProg = bincode::deserialize(&blob).ok()?;
    let mut prog = Program {
        main: cp.main,
        functions: cp.functions,
        tries: cp.tries,
    };
    // `Chunk::op_hash` is `#[serde(skip)]` in fusevm — it is a CACHE of the
    // hash of ops+constants, computed by `ChunkBuilder::build`, so every chunk
    // that comes back from a blob carries 0. Anything keyed by it then looks up
    // the wrong entry: the compiler's side tables below, and fusevm's own JIT
    // cache, which would see every cached chunk as the same key. Recomputing it
    // with `build`'s own algorithm is what makes a loaded chunk indistinguishable
    // from a compiled one.
    rehash(&mut prog);
    // A hit skips lex/parse/lower, and with it every `register_*` the compiler
    // would have run — so the tables come back from the entry instead.
    crate::host::restore_site_tables(&cp.sites);
    Some(prog)
}

/// Recompute `op_hash` on every chunk of `prog`, exactly as
/// `fusevm::ChunkBuilder::build` does: `DefaultHasher` over `ops` then
/// `constants`.
///
/// The two must stay in step; a blob is only ever read back by the binary that
/// wrote it (the cache key carries `BUILD_VERSION` and the binary's own mtime),
/// so the hasher cannot change underneath an entry.
fn rehash(prog: &mut Program) {
    fn one(c: &mut Chunk) {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        c.ops.hash(&mut h);
        c.constants.hash(&mut h);
        c.op_hash = h.finish();
    }
    one(&mut prog.main);
    for (_, f) in &mut prog.functions {
        one(&mut f.chunk);
    }
    for t in &mut prog.tries {
        one(&mut t.block);
        if let Some((_, h)) = &mut t.handler {
            one(h);
        }
        if let Some(f) = &mut t.finalizer {
            one(f);
        }
    }
}

/// Record `prog` (compiled from `src`) in the resident shard. Reaches disk at
/// [`flush`], not here.
pub fn store(src: &str, prog: &Program) -> Result<(), String> {
    let cp = CProg {
        main: prog.main.clone(),
        functions: prog.functions.clone(),
        tries: prog.tries.clone(),
        // Taken after the compile that produced `prog`, so the entry carries
        // what that compile registered.
        sites: crate::host::site_tables(),
    };
    let blob = bincode::serialize(&cp).map_err(|e| format!("cache encode: {e}"))?;
    let key = key_for(src);
    let verify = verify_for(src);
    with_shard(|m| {
        m.entries.insert(key, (verify, blob));
        m.dirty = true;
    });
    Ok(())
}

/// Write the resident shard back, once, at the end of the run.
///
/// The on-disk shard is re-read and MERGED rather than overwritten: up to 16
/// instances share it, and a plain overwrite would drop whatever a peer stored
/// while this process was running. A losing writer still only loses entries
/// (they recompile next run); it can never corrupt the file, since the write
/// itself is a temp-plus-rename.
pub fn flush() {
    let build = build_id();
    let pending = SHARD.with(|c| {
        let mut slot = c.borrow_mut();
        match slot.as_mut() {
            Some(m) if m.dirty => {
                m.dirty = false;
                Some(m.entries.clone())
            }
            _ => None,
        }
    });
    let Some(mut merged) = pending else { return };
    for e in load_shard().entries {
        if e.build == build {
            merged.entry(e.key).or_insert((e.verify, e.blob));
        }
    }
    let shard = Shard {
        entries: merged
            .into_iter()
            .map(|(key, (verify, blob))| Entry {
                key,
                verify,
                build,
                blob,
            })
            .collect(),
    };
    let _ = write_shard(&shard);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both cache hashes must depend on the BUILD, not on `SCHEMA` alone.
    ///
    /// `SCHEMA` and `BUILD_VERSION` are bumped by hand, so the codegen change
    /// that forgets one would otherwise read the previous build's bytecode out
    /// of the shared shard and run the wrong program with no error. This was a
    /// real, reproduced failure: with the key depending on `(SCHEMA, src)`
    /// alone, lowering `**` to a wrong opcode and rebuilding still printed the
    /// OLD answer for an already-cached script.
    ///
    /// Each hash is recomputed here with a component left out and required to
    /// differ, so DELETING any one of the four `hash` lines in `key_for` /
    /// `verify_for` fails this test rather than silently restoring the bug.
    #[test]
    fn cache_keys_depend_on_the_build_not_just_the_schema() {
        use std::collections::hash_map::DefaultHasher;
        let src = "console.log(1)\n";

        // key_for without the version+build id.
        let mut bare = rustc_hash::FxHasher::default();
        SCHEMA.hash(&mut bare);
        src.hash(&mut bare);
        assert_ne!(
            key_for(src),
            bare.finish(),
            "key_for must hash the build identity, not just SCHEMA"
        );

        // key_for with the version but WITHOUT the per-build id: this is what
        // the fleet's version-only design hashes, and it is what leaves two dev
        // builds of one version sharing a shard.
        let mut version_only = rustc_hash::FxHasher::default();
        SCHEMA.hash(&mut version_only);
        BUILD_VERSION.hash(&mut version_only);
        src.hash(&mut version_only);
        assert_ne!(
            key_for(src),
            version_only.finish(),
            "key_for must hash the per-build id, so two dev builds of one \
             version cannot share cached bytecode"
        );

        // verify_for, same two omissions.
        let mut bare = DefaultHasher::new();
        SCHEMA.hash(&mut bare);
        src.len().hash(&mut bare);
        src.hash(&mut bare);
        assert_ne!(
            verify_for(src),
            bare.finish(),
            "verify_for must hash the build identity, not just SCHEMA"
        );

        let mut version_only = DefaultHasher::new();
        SCHEMA.hash(&mut version_only);
        BUILD_VERSION.hash(&mut version_only);
        src.len().hash(&mut version_only);
        src.hash(&mut version_only);
        assert_ne!(
            verify_for(src),
            version_only.finish(),
            "verify_for must hash the per-build id"
        );

        // The version that is hashed is THIS build's, so a release bump rotates
        // the whole shard.
        assert_eq!(BUILD_VERSION, env!("CARGO_PKG_VERSION"));
        // A `0` build id means the stat failed and the per-build guarantee is
        // gone; under `cargo test` the binary is always readable.
        assert_ne!(build_id(), 0, "build_id must read the running binary");
        // Distinct sources still land on distinct keys.
        assert_ne!(key_for(src), key_for("console.log(2)\n"));
    }
}
