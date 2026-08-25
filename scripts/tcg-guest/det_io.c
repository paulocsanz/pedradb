// Vendored from determinismo/determinism-hooks/det_io.c for CI (RFC-0052 P2.3).
// det_io.c — Deterministic I/O fault proxy via LD_PRELOAD (QEMU TCG guest-safe).
//
// Intercepts libc disk I/O so RocksDB/C++ under TCG can:
//   1. Pass-through (I/O works OK) when policy idle / flags disabled
//   2. Inject deterministic faults from DET_IO_SHM (seed + call seq)
//   3. Drop fsync (return 0 without sync)
//   4. v2 BUFFER mode: hold writes in RAM until fsync; process kill loses them
//   5. Linux: generic `syscall()` hook for SYS_io_uring_enter (io-uring crate
//      path — named pwrite/fsync hooks miss the ring). Optional deterministic
//      STALL after a real enter returns (DETIO_STALL_MS / DETIO_STALL_EVERY),
//      same model as pedradb-dst/rbs-dst det_io_uring_stall.c.
//
// Determinism: decision = f(seed, seq, op). Same seed ⇒ same fault stream.
//
// SHM DET_IO_SHM (default /tmp/dst_io), 4096 bytes LE:
//   [0..7]   magic "DETIO01\0"
//   [8..15]  u64  seq
//   [16..19] u32  flags
//            bit0=enabled  bit1=log  bit2=drop_fsync_all
//            bit3=buffer_until_fsync  bit4=crash_discard_on_drop_fsync
//   [20..23] u32  seed
//   [24..27] u32  drop_fsync_mod
//   [28..31] u32  fail_write_mod
//   [32..35] u32  fail_write_rem
//   [36..39] u32  short_write_mod
//   [40..43] u32  delay_us
//   [44..47] u32  enospc_mod
//   [48..51] u32  path_hash_filter (0=all regular files, for buffer mode)
//   [52..55] u32  drop_fsync_path_filter (0=drop fsync on ALL files;
//            nonzero = only drop fsyncs on fds whose path_hash matches)

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <errno.h>
#include <unistd.h>
#include <fcntl.h>
#include <dlfcn.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/uio.h>
#include <stdarg.h>
#include <pthread.h>
#include <time.h>
#ifdef __linux__
#include <sys/syscall.h>
#endif

#ifndef O_CLOEXEC
#define O_CLOEXEC 0
#endif

typedef int (*open_t)(const char *, int, ...);
typedef int (*openat_t)(int, const char *, int, ...);
typedef int (*close_t)(int);
typedef ssize_t (*write_t)(int, const void *, size_t);
typedef ssize_t (*pwrite_t)(int, const void *, size_t, off_t);
typedef ssize_t (*writev_t)(int, const struct iovec *, int);
typedef ssize_t (*read_t)(int, void *, size_t);
typedef ssize_t (*pread_t)(int, void *, size_t, off_t);
typedef int (*fsync_t)(int);
typedef int (*fdatasync_t)(int);
typedef off_t (*lseek_t)(int, off_t, int);

static open_t real_open;
static openat_t real_openat;
static close_t real_close;
static write_t real_write;
static pwrite_t real_pwrite;
static writev_t real_writev;
static read_t real_read;
static pread_t real_pread;
static fsync_t real_fsync;
static fdatasync_t real_fdatasync;
static lseek_t real_lseek;

#ifdef __linux__
typedef long (*syscall_t)(long number, ...);
static syscall_t real_syscall;
static long uring_stall_ms = 0;
static long uring_stall_every = 1;
static unsigned long uring_enter_count = 0;
static char uring_count_file[512];
static int uring_have_count_file = 0;
#endif

static volatile uint8_t *ctrl = NULL;
static int det_disabled = 0;
static int det_log = 0;

#define FD_TAB 2048

// Buffered region (for buffer-until-fsync mode)
typedef struct BufChunk {
    off_t off;
    size_t len;
    char *data;
    struct BufChunk *next;
} BufChunk;

typedef struct {
    uint32_t path_hash;
    int is_reg;
    int buffering; // 1 if we buffer this fd
    off_t pos;     // logical cursor for write()
    off_t size;    // logical size including buffer
    BufChunk *chunks;
    char note[48];
} FdState;

static FdState fd_tab[FD_TAB];
static pthread_mutex_t fd_mu = PTHREAD_MUTEX_INITIALIZER;

enum {
    OFF_MAGIC = 0,
    OFF_SEQ = 8,
    OFF_FLAGS = 16,
    OFF_SEED = 20,
    OFF_DROP_FSYNC_MOD = 24,
    OFF_FAIL_WRITE_MOD = 28,
    OFF_FAIL_WRITE_REM = 32,
    OFF_SHORT_WRITE_MOD = 36,
    OFF_DELAY_US = 40,
    OFF_ENOSPC_MOD = 44,
    OFF_PATH_FILTER = 48,
    OFF_DROP_FSYNC_PATH_FILTER = 52,
};

#define FLAG_ENABLED 1u
#define FLAG_LOG 2u
#define FLAG_DROP_FSYNC_ALL 4u
#define FLAG_BUFFER 8u
#define FLAG_CRASH_DISCARD 16u

static uint32_t load_u32(size_t off) {
    uint32_t v;
    memcpy(&v, (const void *)(ctrl + off), 4);
    return v;
}
static uint64_t load_u64(size_t off) {
    uint64_t v;
    memcpy(&v, (const void *)(ctrl + off), 8);
    return v;
}
static void store_u64(size_t off, uint64_t v) {
    memcpy((void *)(ctrl + off), &v, 8);
}

static uint32_t fnv1a(const char *s) {
    uint32_t h = 2166136261u;
    if (!s) return 0;
    for (; *s; s++) {
        h ^= (uint8_t)*s;
        h *= 16777619u;
    }
    return h ? h : 1;
}

static int enabled(void) {
    if (det_disabled || !ctrl) return 0;
    return (load_u32(OFF_FLAGS) & FLAG_ENABLED) != 0;
}
static int want_log(void) {
    return det_log || (ctrl && (load_u32(OFF_FLAGS) & FLAG_LOG));
}
static int buffer_mode(void) {
    return ctrl && (load_u32(OFF_FLAGS) & FLAG_BUFFER);
}
static int crash_discard(void) {
    return ctrl && (load_u32(OFF_FLAGS) & FLAG_CRASH_DISCARD);
}

static uint64_t bump_seq(void) {
    uint64_t s = load_u64(OFF_SEQ) + 1;
    store_u64(OFF_SEQ, s);
    return s;
}

static void maybe_delay(void) {
    uint32_t us = load_u32(OFF_DELAY_US);
    if (us) usleep(us);
}

static int path_allowed_hash(uint32_t h) {
    uint32_t filt = load_u32(OFF_PATH_FILTER);
    if (filt == 0) return 1;
    return h == filt;
}

// 0=ok, 1=EIO/ENOSPC, 2=short
static int decide_write(uint64_t seq) {
    uint32_t mod = load_u32(OFF_FAIL_WRITE_MOD);
    uint32_t rem = load_u32(OFF_FAIL_WRITE_REM);
    if (mod != 0 && (seq % mod) == rem) return 1;
    uint32_t sm = load_u32(OFF_SHORT_WRITE_MOD);
    if (sm != 0 && (seq % sm) == 0) return 2;
    return 0;
}

static int decide_drop_fsync(uint64_t seq, uint32_t path_hash) {
    // Path-aware filtering: if drop_fsync_path_filter is set (nonzero), only
    // drop fsyncs on fds whose path_hash matches. This lets us target
    // "drop fsync on kv_db files but NOT on raft_db files" — essential for
    // testing the clear_meta_in_kv_and_raft crash window.
    uint32_t path_filt = load_u32(OFF_DROP_FSYNC_PATH_FILTER);
    if (path_filt != 0 && path_hash != path_filt) return 0;

    if (load_u32(OFF_FLAGS) & FLAG_DROP_FSYNC_ALL) return 1;
    uint32_t mod = load_u32(OFF_DROP_FSYNC_MOD);
    if (mod != 0 && (seq % mod) == 0) return 1;
    return 0;
}

static void free_chunks(FdState *st) {
    BufChunk *c = st->chunks;
    while (c) {
        BufChunk *n = c->next;
        free(c->data);
        free(c);
        c = n;
    }
    st->chunks = NULL;
}

static void clear_fd(int fd) {
    if (fd < 0 || fd >= FD_TAB) return;
    free_chunks(&fd_tab[fd]);
    memset(&fd_tab[fd], 0, sizeof(FdState));
}

static void remember_fd(int fd, const char *path) {
    if (fd < 0 || fd >= FD_TAB) return;
    struct stat stbuf;
    int is_reg = 0;
    if (fstat(fd, &stbuf) == 0 && S_ISREG(stbuf.st_mode)) is_reg = 1;

    pthread_mutex_lock(&fd_mu);
    free_chunks(&fd_tab[fd]);
    memset(&fd_tab[fd], 0, sizeof(FdState));
    fd_tab[fd].path_hash = fnv1a(path);
    fd_tab[fd].is_reg = is_reg;
    fd_tab[fd].buffering = is_reg && buffer_mode() && path_allowed_hash(fd_tab[fd].path_hash);
    fd_tab[fd].pos = 0;
    fd_tab[fd].size = is_reg ? (off_t)stbuf.st_size : 0;
    if (path) {
        strncpy(fd_tab[fd].note, path, sizeof(fd_tab[fd].note) - 1);
    }
    pthread_mutex_unlock(&fd_mu);
}

static int buf_add(FdState *st, off_t off, const void *buf, size_t n) {
    BufChunk *c = (BufChunk *)malloc(sizeof(BufChunk));
    if (!c) return -1;
    c->data = (char *)malloc(n);
    if (!c->data) {
        free(c);
        return -1;
    }
    memcpy(c->data, buf, n);
    c->off = off;
    c->len = n;
    c->next = st->chunks;
    st->chunks = c;
    if (off + (off_t)n > st->size) st->size = off + (off_t)n;
    return 0;
}

// Apply buffered chunks over a read into buf[0..n) at file offset base.
static void buf_overlay(FdState *st, off_t base, char *buf, size_t n) {
    for (BufChunk *c = st->chunks; c; c = c->next) {
        off_t cs = c->off, ce = c->off + (off_t)c->len;
        off_t bs = base, be = base + (off_t)n;
        off_t s = cs > bs ? cs : bs;
        off_t e = ce < be ? ce : be;
        if (s >= e) continue;
        size_t dst = (size_t)(s - bs);
        size_t src = (size_t)(s - cs);
        size_t len = (size_t)(e - s);
        memcpy(buf + dst, c->data + src, len);
    }
}

// Flush all chunks to real fd via pwrite (does not fsync).
static int buf_flush_to_disk(int fd, FdState *st) {
    if (!real_pwrite) real_pwrite = (pwrite_t)dlsym(RTLD_NEXT, "pwrite");
    // Flush in reverse list order is fine; later chunks overwrite earlier in overlay
    // but for disk we should write all. Walk and write each.
    for (BufChunk *c = st->chunks; c; c = c->next) {
        size_t left = c->len;
        size_t done = 0;
        while (left) {
            ssize_t w = real_pwrite(fd, c->data + done, left, c->off + (off_t)done);
            if (w < 0) return -1;
            if (w == 0) {
                errno = EIO;
                return -1;
            }
            done += (size_t)w;
            left -= (size_t)w;
        }
    }
    free_chunks(st);
    return 0;
}

static void init_hooks(void) __attribute__((constructor));
static void init_hooks(void) {
    if (getenv("DET_LOG") && getenv("DET_LOG")[0] == '1') det_log = 1;
    if (getenv("DET_IO_LOG") && getenv("DET_IO_LOG")[0] == '1') det_log = 1;

    // Always resolve pass-through symbols (incl. syscall for io_uring stall).
    real_open = (open_t)dlsym(RTLD_NEXT, "open");
    real_openat = (openat_t)dlsym(RTLD_NEXT, "openat");
    real_close = (close_t)dlsym(RTLD_NEXT, "close");
    real_write = (write_t)dlsym(RTLD_NEXT, "write");
    real_pwrite = (pwrite_t)dlsym(RTLD_NEXT, "pwrite");
    real_writev = (writev_t)dlsym(RTLD_NEXT, "writev");
    real_read = (read_t)dlsym(RTLD_NEXT, "read");
    real_pread = (pread_t)dlsym(RTLD_NEXT, "pread");
    real_fsync = (fsync_t)dlsym(RTLD_NEXT, "fsync");
    real_fdatasync = (fdatasync_t)dlsym(RTLD_NEXT, "fdatasync");
    real_lseek = (lseek_t)dlsym(RTLD_NEXT, "lseek");
#ifdef __linux__
    real_syscall = (syscall_t)dlsym(RTLD_NEXT, "syscall");
    {
        const char *ms = getenv("DETIO_STALL_MS");
        if (ms) uring_stall_ms = atol(ms);
        const char *every = getenv("DETIO_STALL_EVERY");
        if (every) uring_stall_every = atol(every);
        if (uring_stall_every <= 0) uring_stall_every = 1;
        const char *cf = getenv("DETIO_COUNT_FILE");
        if (cf && cf[0]) {
            strncpy(uring_count_file, cf, sizeof(uring_count_file) - 1);
            uring_count_file[sizeof(uring_count_file) - 1] = '\0';
            uring_have_count_file = 1;
        }
        if (det_log || uring_stall_ms > 0 || uring_have_count_file) {
            fprintf(stderr,
                    "[det_io] io_uring syscall hook ready stall_ms=%ld stall_every=%ld "
                    "SYS_io_uring_enter=%d count_file=%s\n",
                    uring_stall_ms, uring_stall_every, (int)SYS_io_uring_enter,
                    uring_have_count_file ? uring_count_file : "(none)");
        }
    }
#endif

    const char *dis = getenv("DET_IO_DISABLE");
    if (dis && dis[0] == '1') {
        // Named-I/O fault injection off; syscall/io_uring stall still active.
        det_disabled = 1;
        return;
    }

    const char *shm = getenv("DET_IO_SHM");
    if (!shm) shm = "/tmp/dst_io";

    int fd = open(shm, O_RDWR);
    if (fd < 0) {
        fd = open(shm, O_CREAT | O_RDWR, 0644);
        if (fd < 0) {
            det_disabled = 1;
            return;
        }
        char page[4096];
        memset(page, 0, sizeof(page));
        memcpy(page, "DETIO01", 7);
        if (write(fd, page, sizeof(page)) != (ssize_t)sizeof(page)) {
            close(fd);
            det_disabled = 1;
            return;
        }
    }
    if (lseek(fd, 0, SEEK_END) < 4096) ftruncate(fd, 4096);
    void *mapped = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    close(fd);
    if (mapped == MAP_FAILED) {
        det_disabled = 1;
        return;
    }
    ctrl = (volatile uint8_t *)mapped;
    if (memcmp((const void *)ctrl, "DETIO01", 7) != 0) {
        memcpy((void *)ctrl, "DETIO01", 7);
    }
    if (want_log()) {
        fprintf(stderr, "[det_io] ok shm=%s flags=0x%x\n", shm,
                (unsigned)load_u32(OFF_FLAGS));
    }
}

// --- open / close / lseek ---

int open(const char *path, int flags, ...) {
    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list ap;
        va_start(ap, flags);
        mode = (mode_t)va_arg(ap, int);
        va_end(ap);
    }
    if (!real_open) real_open = (open_t)dlsym(RTLD_NEXT, "open");
    int fd = (flags & O_CREAT) ? real_open(path, flags, mode) : real_open(path, flags);
    if (fd >= 0) remember_fd(fd, path);
    return fd;
}

int openat(int dirfd, const char *path, int flags, ...) {
    mode_t mode = 0;
    if (flags & O_CREAT) {
        va_list ap;
        va_start(ap, flags);
        mode = (mode_t)va_arg(ap, int);
        va_end(ap);
    }
    if (!real_openat) real_openat = (openat_t)dlsym(RTLD_NEXT, "openat");
    int fd = (flags & O_CREAT) ? real_openat(dirfd, path, flags, mode)
                               : real_openat(dirfd, path, flags);
    if (fd >= 0) remember_fd(fd, path);
    return fd;
}

int close(int fd) {
    if (!real_close) real_close = (close_t)dlsym(RTLD_NEXT, "close");
    // Do NOT auto-flush on close: un-fsynced buffer is lost (crash fidelity).
    pthread_mutex_lock(&fd_mu);
    if (fd >= 0 && fd < FD_TAB && fd_tab[fd].chunks && want_log()) {
        fprintf(stderr, "[det_io] close fd=%d dropping %s buffered data (no fsync)\n",
                fd, fd_tab[fd].note);
    }
    clear_fd(fd);
    pthread_mutex_unlock(&fd_mu);
    return real_close(fd);
}

off_t lseek(int fd, off_t offset, int whence) {
    if (!real_lseek) real_lseek = (lseek_t)dlsym(RTLD_NEXT, "lseek");
    pthread_mutex_lock(&fd_mu);
    FdState *st = (fd >= 0 && fd < FD_TAB) ? &fd_tab[fd] : NULL;
    if (st && st->buffering) {
        off_t np = st->pos;
        if (whence == SEEK_SET) np = offset;
        else if (whence == SEEK_CUR) np = st->pos + offset;
        else if (whence == SEEK_END) np = st->size + offset;
        if (np < 0) {
            pthread_mutex_unlock(&fd_mu);
            errno = EINVAL;
            return (off_t)-1;
        }
        st->pos = np;
        // keep kernel offset roughly in sync for non-buffered paths
        real_lseek(fd, np, SEEK_SET);
        pthread_mutex_unlock(&fd_mu);
        return np;
    }
    pthread_mutex_unlock(&fd_mu);
    return real_lseek(fd, offset, whence);
}

// --- write ---

static ssize_t do_buffered_write(int fd, const void *buf, size_t n, off_t off, int use_off) {
    pthread_mutex_lock(&fd_mu);
    FdState *st = &fd_tab[fd];
    if (!st->buffering) {
        pthread_mutex_unlock(&fd_mu);
        if (use_off) {
            if (!real_pwrite) real_pwrite = (pwrite_t)dlsym(RTLD_NEXT, "pwrite");
            return real_pwrite(fd, buf, n, off);
        }
        if (!real_write) real_write = (write_t)dlsym(RTLD_NEXT, "write");
        return real_write(fd, buf, n);
    }
    off_t at = use_off ? off : st->pos;
    if (buf_add(st, at, buf, n) != 0) {
        pthread_mutex_unlock(&fd_mu);
        errno = ENOMEM;
        return -1;
    }
    if (!use_off) st->pos = at + (off_t)n;
    pthread_mutex_unlock(&fd_mu);
    return (ssize_t)n;
}

ssize_t write(int fd, const void *buf, size_t n) {
    if (!real_write) real_write = (write_t)dlsym(RTLD_NEXT, "write");
    if (!enabled() || fd < 0 || fd >= FD_TAB) return real_write(fd, buf, n);

    uint64_t seq = bump_seq();
    int d = decide_write(seq);
    if (want_log())
        fprintf(stderr, "[det_io] write seq=%llu fd=%d n=%zu d=%d\n",
                (unsigned long long)seq, fd, n, d);
    maybe_delay();
    if (d == 1) {
        errno = (load_u32(OFF_ENOSPC_MOD) && (seq % load_u32(OFF_ENOSPC_MOD)) == 0) ? ENOSPC
                                                                                      : EIO;
        return -1;
    }
    size_t wn = n;
    if (d == 2 && n > 1) wn = n / 2 ? n / 2 : 1;

    if (buffer_mode() && fd_tab[fd].buffering)
        return do_buffered_write(fd, buf, wn, 0, 0);
    return real_write(fd, buf, wn);
}

ssize_t pwrite(int fd, const void *buf, size_t n, off_t off) {
    if (!real_pwrite) real_pwrite = (pwrite_t)dlsym(RTLD_NEXT, "pwrite");
    if (!enabled() || fd < 0 || fd >= FD_TAB) return real_pwrite(fd, buf, n, off);

    uint64_t seq = bump_seq();
    int d = decide_write(seq);
    if (want_log())
        fprintf(stderr, "[det_io] pwrite seq=%llu fd=%d n=%zu off=%lld d=%d\n",
                (unsigned long long)seq, fd, n, (long long)off, d);
    maybe_delay();
    if (d == 1) {
        errno = EIO;
        return -1;
    }
    size_t wn = n;
    if (d == 2 && n > 1) wn = n / 2 ? n / 2 : 1;

    if (buffer_mode() && fd_tab[fd].buffering)
        return do_buffered_write(fd, buf, wn, off, 1);
    return real_pwrite(fd, buf, wn, off);
}

#ifdef __linux__
ssize_t pwrite64(int fd, const void *buf, size_t n, off_t off) {
    return pwrite(fd, buf, n, off);
}
#endif

ssize_t writev(int fd, const struct iovec *iov, int iovcnt) {
    if (!real_writev) real_writev = (writev_t)dlsym(RTLD_NEXT, "writev");
    if (!enabled() || fd < 0 || fd >= FD_TAB || !buffer_mode() || !fd_tab[fd].buffering)
        return real_writev(fd, iov, iovcnt);
    // Expand writev into buffered writes
    ssize_t total = 0;
    for (int i = 0; i < iovcnt; i++) {
        ssize_t w = write(fd, iov[i].iov_base, iov[i].iov_len);
        if (w < 0) return total > 0 ? total : -1;
        total += w;
        if ((size_t)w < iov[i].iov_len) break;
    }
    return total;
}

// --- read (serve overlay from buffer) ---

ssize_t read(int fd, void *buf, size_t n) {
    if (!real_read) real_read = (read_t)dlsym(RTLD_NEXT, "read");
    if (!enabled() || fd < 0 || fd >= FD_TAB || !buffer_mode() || !fd_tab[fd].buffering)
        return real_read(fd, buf, n);

    pthread_mutex_lock(&fd_mu);
    FdState *st = &fd_tab[fd];
    off_t at = st->pos;
    pthread_mutex_unlock(&fd_mu);

    // Read underlying file then overlay buffer
    ssize_t r = real_read(fd, buf, n);
    if (r < 0) return r;
    // If file short but buffer has data past EOF, synthesize zeros + overlay
    size_t got = r > 0 ? (size_t)r : 0;
    if (got < n) {
        // check if buffer has anything beyond
        pthread_mutex_lock(&fd_mu);
        if (st->size > at + (off_t)got) {
            size_t need = n - got;
            off_t avail = st->size - (at + (off_t)got);
            if ((off_t)need > avail) need = (size_t)avail;
            memset((char *)buf + got, 0, need);
            got += need;
            r = (ssize_t)got;
        }
        pthread_mutex_unlock(&fd_mu);
    }
    pthread_mutex_lock(&fd_mu);
    buf_overlay(st, at, (char *)buf, got);
    st->pos = at + (off_t)got;
    pthread_mutex_unlock(&fd_mu);
    return r;
}

ssize_t pread(int fd, void *buf, size_t n, off_t off) {
    if (!real_pread) real_pread = (pread_t)dlsym(RTLD_NEXT, "pread");
    if (!enabled() || fd < 0 || fd >= FD_TAB || !buffer_mode() || !fd_tab[fd].buffering)
        return real_pread(fd, buf, n, off);

    ssize_t r = real_pread(fd, buf, n, off);
    if (r < 0) return r;
    size_t got = r > 0 ? (size_t)r : 0;
    pthread_mutex_lock(&fd_mu);
    FdState *st = &fd_tab[fd];
    if (got < n && st->size > off + (off_t)got) {
        size_t need = n - got;
        off_t avail = st->size - (off + (off_t)got);
        if ((off_t)need > avail) need = (size_t)avail;
        memset((char *)buf + got, 0, need);
        got += need;
        r = (ssize_t)got;
    }
    buf_overlay(st, off, (char *)buf, got);
    pthread_mutex_unlock(&fd_mu);
    return r;
}

// --- fsync ---

static int inject_fsync(int fd, const char *tag, fsync_t real_fn) {
    if (!real_fn) return -1;
    if (!enabled() || fd < 0 || fd >= FD_TAB) return real_fn(fd);

    uint64_t seq = bump_seq();
    int drop = decide_drop_fsync(seq, fd_tab[fd].path_hash);
    if (want_log())
        fprintf(stderr, "[det_io] %s seq=%llu fd=%d drop=%d buffer=%d path=%s\n", tag,
                (unsigned long long)seq, fd, drop, fd_tab[fd].buffering,
                fd_tab[fd].note);
    maybe_delay();

    pthread_mutex_lock(&fd_mu);
    FdState *st = &fd_tab[fd];
    if (st->buffering && st->chunks) {
        if (drop && crash_discard()) {
            // Power loss of dirty buffer: discard, report success.
            free_chunks(st);
            pthread_mutex_unlock(&fd_mu);
            return 0;
        }
        if (drop && !crash_discard()) {
            // Old v1 semantics: leave data in page-cache-ish buffer but skip sync.
            // Still keep buffer (visible in-process) — for true crash use CRASH_DISCARD.
            pthread_mutex_unlock(&fd_mu);
            return 0;
        }
        // Commit buffer then real fsync
        if (buf_flush_to_disk(fd, st) != 0) {
            pthread_mutex_unlock(&fd_mu);
            return -1;
        }
    }
    pthread_mutex_unlock(&fd_mu);

    if (drop) return 0; // drop fsync after already flushed (weaker) or no buffer
    return real_fn(fd);
}

int fsync(int fd) {
    if (!real_fsync) real_fsync = (fsync_t)dlsym(RTLD_NEXT, "fsync");
    return inject_fsync(fd, "fsync", real_fsync);
}

int fdatasync(int fd) {
    if (!real_fdatasync) real_fdatasync = (fdatasync_t)dlsym(RTLD_NEXT, "fdatasync");
    if (!real_fdatasync) {
        if (!real_fsync) real_fsync = (fsync_t)dlsym(RTLD_NEXT, "fsync");
        real_fdatasync = real_fsync;
    }
    return inject_fsync(fd, "fdatasync", real_fdatasync);
}

#ifdef __linux__
// --- generic syscall(): reach io-uring crate (SYS_io_uring_enter) ---
// The io-uring crate (default build, no direct-syscall) calls libc syscall().
// Named pwrite/fsync hooks never see ring I/O. Stall is post-completion delay
// only (real enter already returned; CQ already updated).

static void uring_sleep_ms(long ms) {
    struct timespec ts;
    ts.tv_sec = ms / 1000;
    ts.tv_nsec = (ms % 1000) * 1000000L;
    nanosleep(&ts, NULL);
}

__attribute__((destructor))
static void det_io_uring_fini(void) {
    if (!uring_have_count_file) return;
    FILE *f = fopen(uring_count_file, "w");
    if (!f) return;
    fprintf(f, "%lu\n", uring_enter_count);
    fclose(f);
}

long syscall(long number, ...) {
    if (!real_syscall) real_syscall = (syscall_t)dlsym(RTLD_NEXT, "syscall");

    va_list ap;
    va_start(ap, number);
    long a1 = va_arg(ap, long);
    long a2 = va_arg(ap, long);
    long a3 = va_arg(ap, long);
    long a4 = va_arg(ap, long);
    long a5 = va_arg(ap, long);
    long a6 = va_arg(ap, long);
    va_end(ap);

    long ret = real_syscall(number, a1, a2, a3, a4, a5, a6);

    if (number == SYS_io_uring_enter) {
        uring_enter_count++;
        int do_stall =
            uring_stall_ms > 0 &&
            (uring_enter_count % (unsigned long)uring_stall_every) == 0;
        if (det_log) {
            fprintf(stderr,
                    "[det_io] SYS_io_uring_enter #%lu fd=%ld to_submit=%ld "
                    "min_complete=%ld ret=%ld stall=%s\n",
                    uring_enter_count, a1, a2, a3, ret,
                    do_stall ? "yes" : "no");
        }
        if (do_stall) uring_sleep_ms(uring_stall_ms);
    }

    return ret;
}
#endif
