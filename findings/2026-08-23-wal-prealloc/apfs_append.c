// APFS append-stall microbench: write 64 KiB chunks, time each write().
// Usage: apfs_append <path> <mode: plain|prealloc> [chunk_kb] [total_mb]
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/time.h>

static double now_ms(void) {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return tv.tv_sec * 1e3 + tv.tv_usec / 1e3;
}

int main(int argc, char **argv) {
    if (argc < 3) { fprintf(stderr, "usage: %s path plain|prealloc [chunk_kb] [total_mb]\n", argv[0]); return 2; }
    const char *path = argv[1];
    int prealloc = strcmp(argv[2], "prealloc") == 0;
    size_t chunk = (argc > 3 ? atoi(argv[3]) : 64) * 1024;
    size_t total = (argc > 4 ? atoi(argv[4]) : 48) * 1024 * 1024;

    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd < 0) { perror("open"); return 1; }

    if (prealloc) {
        fstore_t st;
        memset(&st, 0, sizeof st);
        st.fst_flags = F_ALLOCATEALL | F_ALLOCATECONTIG;
        st.fst_posmode = F_PEOFPOSMODE;
        st.fst_offset = 0;
        st.fst_length = (off_t)total;
        if (fcntl(fd, F_PREALLOCATE, &st) < 0) {
            // fall back to non-contiguous
            st.fst_flags = F_ALLOCATEALL;
            if (fcntl(fd, F_PREALLOCATE, &st) < 0) { perror("F_PREALLOCATE"); return 1; }
            printf("prealloc: non-contiguous OK (%zu MiB)\n", total >> 20);
        } else {
            printf("prealloc: contiguous OK (%zu MiB)\n", total >> 20);
        }
    }

    char *buf = calloc(1, chunk);
    size_t written = 0;
    long writes = 0, slow = 0, veryslow = 0;
    double max_ms = 0;
    while (written < total) {
        double t0 = now_ms();
        ssize_t n = write(fd, buf, chunk);
        double dt = now_ms() - t0;
        if (n != (ssize_t)chunk) { perror("write"); return 1; }
        written += chunk;
        writes++;
        if (dt > max_ms) max_ms = dt;
        if (dt > 1.0) { slow++; if (dt > 5.0) veryslow++; printf("write #%ld at %.2f MiB: %.2f ms\n", writes, written / 1048576.0, dt); }
    }
    printf("%s: %ld writes, slow(>1ms)=%ld veryslow(>5ms)=%ld max=%.2f ms\n",
           prealloc ? "prealloc" : "plain  ", writes, slow, veryslow, max_ms);
    close(fd);
    return 0;
}
