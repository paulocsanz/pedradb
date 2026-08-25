/* RFC-0052 P2.1 — PID 1 in the TCG Linux guest.
 *
 * Mounts a tmpfs, execs the static musl `world_smoke` with seed 42, prints
 * begin/end markers on the serial console, then poweroffs. No wall-clock
 * is printed; the host compares `trace_hash` only.
 */
#define _GNU_SOURCE
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mount.h>
#include <sys/reboot.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/wait.h>
#include <unistd.h>

static void say(const char *s) { (void)write(1, s, __builtin_strlen(s)); }

int main(void) {
  (void)mkdir("/proc", 0755);
  (void)mkdir("/sys", 0755);
  (void)mkdir("/tmp", 0755);
  (void)mkdir("/dev", 0755);
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
  say("TCG_WORLD_SMOKE_BEGIN\n");
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
  say("TCG_WORLD_SMOKE_END\n");
  (void)sync();
  (void)reboot(RB_POWER_OFF);
  for (;;) {
    (void)pause();
  }
}
