// allocstacks.c: DYLD_INSERT_LIBRARIES malloc interposer. Exact count and bytes of every malloc/calloc/realloc/
// posix_memalign/aligned_alloc/valloc call, keyed by the caller's return-address stack (frame-pointer walk,
// DEPTH frames), aggregated per thread in mmap'd open-addressing tables, dumped at exit to $ALLOCSTACKS_OUT.
// Build: clang -arch arm64 -O2 -dynamiclib -o liballocstacks.dylib allocstacks.c
// Use:   DYLD_INSERT_LIBRARIES=liballocstacks.dylib ALLOCSTACKS_OUT=/tmp/stacks.tsv <binary with the system allocator> ...
//        Run the binary directly: SIP strips DYLD_* from the environment of restricted binaries (/usr/bin/time, env),
//        so nothing launched through them is instrumented. The binary must use the system allocator (tsrs_cli
//        feature `system-alloc`); mimalloc's calls never reach these interposed symbols.
// Output: "# load <main image load address>", "# totals ...", then "count<TAB>bytes<TAB>kind<TAB>addr0 addr1 ..."
// (raw return addresses; symbolicate addr-1 with llvm-symbolizer/atos against the binary at that load address).
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <mach-o/dyld.h>
#include <pthread.h>
#include <dlfcn.h>
#include <mach-o/loader.h>
#include <mach-o/getsect.h>

#define DEPTH 12
#define TABLE_BITS 22
#define TABLE_SIZE (1u << TABLE_BITS)
enum { K_MALLOC, K_CALLOC, K_REALLOC, K_MEMALIGN, K_ALIGNED, K_VALLOC, K_COUNT };
static const char *KIND_NAME[K_COUNT] = {"malloc", "calloc", "realloc", "posix_memalign", "aligned_alloc", "valloc"};

typedef struct {
    uint64_t frames[DEPTH];
    uint64_t count;
    uint64_t bytes;
    uint32_t hash;
    uint8_t kind;
    uint8_t nframes;
    uint8_t pad[2];
} entry_t;  // 128 bytes

typedef struct {
    entry_t *entries;
    uint64_t used;
    uint64_t dropped;
    uint64_t calls[K_COUNT];
    uint64_t frees;
} table_t;

static __thread table_t *tls_table;
static __thread int in_hook;
#define MAX_TABLES 4096
static uintptr_t self_lo, self_hi;
static table_t *tables[MAX_TABLES];
static int ntables;
static int lock;

__attribute__((constructor)) static void init_self(void) {
    Dl_info info; if (dladdr((void *)&init_self, &info) && info.dli_fbase) {
        unsigned long sz = 0; const struct mach_header_64 *mh = (const struct mach_header_64 *)info.dli_fbase;
        getsegmentdata(mh, "__TEXT", &sz);
        self_lo = (uintptr_t)mh; self_hi = self_lo + (sz ? sz : (1u << 20));
    }
}
static const char *main_image(uintptr_t *load) {
    uint32_t n = _dyld_image_count();
    for (uint32_t i = 0; i < n; i++) { const struct mach_header *mh = _dyld_get_image_header(i); if (mh && mh->filetype == MH_EXECUTE) { *load = (uintptr_t)mh; return _dyld_get_image_name(i); } }
    *load = 0; return "?";
}
static void spin_lock(void) { while (__atomic_exchange_n(&lock, 1, __ATOMIC_ACQUIRE)) {} }
static void spin_unlock(void) { __atomic_store_n(&lock, 0, __ATOMIC_RELEASE); }

static table_t *get_table(void) {
    table_t *t = tls_table;
    if (t) return t;
    size_t bytes = sizeof(table_t) + (size_t)TABLE_SIZE * sizeof(entry_t);
    void *m = mmap(NULL, bytes, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    if (m == MAP_FAILED) return NULL;
    t = (table_t *)m;
    t->entries = (entry_t *)((char *)m + sizeof(table_t));
    spin_lock();
    if (ntables < MAX_TABLES) tables[ntables++] = t;
    spin_unlock();
    tls_table = t;
    return t;
}

static inline int capture(uint64_t *out) {
    uintptr_t *fp = (uintptr_t *)__builtin_frame_address(0);
    int n = 0;
    while (fp && n < DEPTH) {
        uintptr_t *prev = (uintptr_t *)fp[0];
        uintptr_t ret = fp[1] & 0x0000FFFFFFFFFFFFull;
        if (!ret) break;
        if (ret < self_lo || ret >= self_hi) out[n++] = ret;
        if (prev <= fp || (uintptr_t)prev - (uintptr_t)fp > (1u << 28)) break;
        fp = prev;
    }
    return n;
}


static inline __attribute__((always_inline)) void record(int kind, size_t size) {
    if (in_hook) return;
    in_hook = 1;
    table_t *t = get_table();
    if (t) {
        t->calls[kind]++;
        uint64_t frames[DEPTH];
        int n = capture(frames);
        uint64_t h = 0x9E3779B97F4A7C15ull ^ (uint64_t)kind;
        for (int i = 0; i < n; i++) h = (h ^ frames[i]) * 0x100000001B3ull;
        uint32_t h32 = (uint32_t)(h ^ (h >> 32));
        uint32_t idx = h32 & (TABLE_SIZE - 1);
        for (uint32_t probe = 0; probe < TABLE_SIZE; probe++, idx = (idx + 1) & (TABLE_SIZE - 1)) {
            entry_t *e = &t->entries[idx];
            if (e->count == 0) {
                if (t->used + 1 >= TABLE_SIZE) { t->dropped++; break; }
                memcpy(e->frames, frames, n * sizeof(uint64_t));
                e->nframes = (uint8_t)n; e->kind = (uint8_t)kind; e->hash = h32;
                e->count = 1; e->bytes = size; t->used++;
                break;
            }
            if (e->hash == h32 && e->kind == kind && e->nframes == n && memcmp(e->frames, frames, n * sizeof(uint64_t)) == 0) {
                e->count++; e->bytes += size;
                break;
            }
        }
    }
    in_hook = 0;
}

static void *my_malloc(size_t size) { record(K_MALLOC, size); return malloc(size); }
static void *my_calloc(size_t n, size_t size) { record(K_CALLOC, n * size); return calloc(n, size); }
static void *my_realloc(void *p, size_t size) { record(K_REALLOC, size); return realloc(p, size); }
static int my_posix_memalign(void **out, size_t align, size_t size) { record(K_MEMALIGN, size); return posix_memalign(out, align, size); }
static void *my_aligned_alloc(size_t align, size_t size) { record(K_ALIGNED, size); return aligned_alloc(align, size); }
static void *my_valloc(size_t size) { record(K_VALLOC, size); return valloc(size); }
static void my_free(void *p) {
    if (!in_hook && p) { in_hook = 1; table_t *t = get_table(); if (t) t->frees++; in_hook = 0; }
    free(p);
}

#define DYLD_INTERPOSE(_replacement, _replacee) \
    __attribute__((used)) static struct { const void *replacement; const void *replacee; } _interpose_##_replacee \
        __attribute__((section("__DATA,__interpose"))) = { (const void *)(unsigned long)&_replacement, (const void *)(unsigned long)&_replacee };
DYLD_INTERPOSE(my_malloc, malloc)
DYLD_INTERPOSE(my_calloc, calloc)
DYLD_INTERPOSE(my_realloc, realloc)
DYLD_INTERPOSE(my_posix_memalign, posix_memalign)
DYLD_INTERPOSE(my_aligned_alloc, aligned_alloc)
DYLD_INTERPOSE(my_valloc, valloc)
DYLD_INTERPOSE(my_free, free)

static char outbuf[1 << 16];
static size_t outlen;
static int outfd = -1;
static void flush(void) { if (outlen) { size_t off = 0; while (off < outlen) { ssize_t w = write(outfd, outbuf + off, outlen - off); if (w <= 0) break; off += (size_t)w; } outlen = 0; } }
static void put(const char *s, size_t n) { if (outlen + n > sizeof outbuf) flush(); memcpy(outbuf + outlen, s, n); outlen += n; }
static void put_u64(uint64_t v) { char b[32]; int i = 31; b[i] = 0; if (!v) b[--i] = '0'; while (v) { b[--i] = (char)('0' + v % 10); v /= 10; } put(b + i, 31 - i); }
static void put_hex(uint64_t v) { char b[20]; int i = 19; b[i] = 0; if (!v) b[--i] = '0'; while (v) { b[--i] = "0123456789abcdef"[v & 15]; v >>= 4; } put("0x", 2); put(b + i, 19 - i); }

__attribute__((destructor)) static void dump(void) {
    const char *path = getenv("ALLOCSTACKS_OUT");
    if (!path) return;
    in_hook = 1;
    outfd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (outfd < 0) return;
    uintptr_t load; const char *name = main_image(&load);
    put("# load ", 7); put_hex((uint64_t)load); put(" ", 1); put(name, strlen(name)); put("\n", 1);
    spin_lock();
    uint64_t calls[K_COUNT] = {0}, frees = 0, dropped = 0, used = 0;
    for (int i = 0; i < ntables; i++) { table_t *t = tables[i]; for (int k = 0; k < K_COUNT; k++) calls[k] += t->calls[k]; frees += t->frees; dropped += t->dropped; used += t->used; }
    put("# totals", 8);
    for (int k = 0; k < K_COUNT; k++) { put(" ", 1); put(KIND_NAME[k], strlen(KIND_NAME[k])); put("=", 1); put_u64(calls[k]); }
    put(" free=", 6); put_u64(frees); put(" threads=", 9); put_u64((uint64_t)ntables); put(" stacks=", 8); put_u64(used); put(" dropped=", 9); put_u64(dropped); put("\n", 1);
    for (int i = 0; i < ntables; i++) {
        table_t *t = tables[i];
        for (uint32_t j = 0; j < TABLE_SIZE; j++) {
            entry_t *e = &t->entries[j];
            if (!e->count) continue;
            put_u64(e->count); put("\t", 1); put_u64(e->bytes); put("\t", 1); put(KIND_NAME[e->kind], strlen(KIND_NAME[e->kind])); put("\t", 1);
            for (int f = 0; f < e->nframes; f++) { if (f) put(" ", 1); put_hex(e->frames[f]); }
            put("\n", 1);
        }
    }
    spin_unlock();
    flush(); close(outfd);
}
