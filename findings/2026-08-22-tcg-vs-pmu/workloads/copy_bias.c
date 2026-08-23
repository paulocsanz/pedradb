/*
 * Tiny CPU workload: 1 KiB memcpy vs 16 KiB memcpy, same op count.
 * Used to show TCG *wall time* ranking can invert vs native (SIMD / cache
 * / TCG helper cost), while *guest instruction count* stays a work metric.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

#ifndef OPS
#define OPS (1u << 20)
#endif

static uint64_t nsec(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

static uint64_t run(size_t n, unsigned ops)
{
    static unsigned char src[64 * 1024];
    static unsigned char dst[64 * 1024];
    memset(src, 0xA5, sizeof src);
    uint64_t t0 = nsec();
    for (unsigned i = 0; i < ops; i++) {
        memcpy(dst, src, n);
        src[i % n] = (unsigned char)(dst[0] + i);
    }
    return nsec() - t0;
}

int main(void)
{
    uint64_t a = run(1024, OPS);
    uint64_t b = run(16 * 1024, OPS);
    printf("ops=%u copy_1k_ns=%llu copy_16k_ns=%llu ratio_16k_over_1k=%.3f\n",
           OPS, (unsigned long long)a, (unsigned long long)b,
           (double)b / (double)a);
    return 0;
}
