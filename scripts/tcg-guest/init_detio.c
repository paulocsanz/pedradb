/* RFC-0052 P2.3 — PID 1: arm libdet_io.so then exec world_smoke.
 *
 * Writes DET_IO_SHM with enabled+log+drop_fsync_all (seed 42), then
 * LD_PRELOAD=/libdet_io.so (also STALL_SO). Serial must show [det_io]
 * intercepts; host compares trace_hash, never wall-clock.
 */
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/reboot.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/wait.h>
#include <unistd.h>

static void say(const char *s) { (void)write(1, s, __builtin_strlen(s)); }

static int write_shm(void) {
  char page[4096];
  memset(page, 0, sizeof(page));
  memcpy(page, "DETIO01", 7);
  uint32_t flags = 1u | 2u | 4u; /* enabled | log | drop_fsync_all */
  uint32_t seed = 42;
  memcpy(page + 16, &flags, 4);
  memcpy(page + 20, &seed, 4);
  int fd = open("/tmp/dst_io", O_CREAT | O_RDWR | O_TRUNC, 0644);
  if (fd < 0) {
    return -1;
  }
  if (write(fd, page, sizeof(page)) != (ssize_t)sizeof(page)) {
    close(fd);
    return -1;
  }
  close(fd);
  return 0;
}

int main(void) {
  (void)mkdir("/proc", 0755);
  (void)mkdir("/sys", 0755);
  (void)mkdir("/tmp", 0755);
  (void)mkdir("/dev", 0755);
  (void)mkdir("/lib", 0755);
  (void)mount("proc", "/proc", "proc", 0, 0);
  (void)mount("sysfs", "/sys", "sysfs", 0, 0);
  (void)mount("tmpfs", "/tmp", "tmpfs", 0, 0);
  if (mount("devtmpfs", "/dev", "devtmpfs", 0, 0) != 0) {
    (void)mknod("/dev/console", S_IFCHR | 0600, makedev(5, 1));
    (void)mknod("/dev/null", S_IFCHR | 0666, makedev(1, 3));
  }
  int c = open("/dev/console", O_RDWR);
  if (c >= 0) {
    (void)dup2(c, 0);
    (void)dup2(c, 1);
    (void)dup2(c, 2);
    if (c > 2) {
      (void)close(c);
    }
  }
  (void)setenv("TMPDIR", "/tmp", 1);
  (void)setenv("HOME", "/tmp", 1);
  (void)setenv("LD_LIBRARY_PATH", "/lib", 1);
  if (write_shm() != 0) {
    say("DET_IO_SHM_FAIL\n");
  }
  (void)setenv("DET_IO_SHM", "/tmp/dst_io", 1);
  (void)setenv("STALL_SO", "/libdet_io.so", 1);
  (void)setenv("LD_PRELOAD", "/libdet_io.so", 1);
  (void)setenv("DET_LOG", "1", 1);
  (void)setenv("DETIO_STALL_MS", "1", 1);
  (void)setenv("DETIO_STALL_EVERY", "1", 1);
  (void)setenv("DETIO_COUNT_FILE", "/tmp/detio_count", 1);
  say("TCG_DETIO_BEGIN\n");
  say("STALL_SO=/libdet_io.so\n");
  pid_t p = fork();
  if (p == 0) {
    char *argv[] = {"/world_smoke", "42", 0};
    execv("/world_smoke", argv);
    perror("exec world_smoke");
    _exit(127);
  }
  if (p < 0) {
    perror("fork");
  } else {
    int st = 0;
    (void)waitpid(p, &st, 0);
  }
  say("TCG_DETIO_END\n");
  (void)sync();
  (void)reboot(RB_POWER_OFF);
  for (;;) {
    (void)pause();
  }
}
