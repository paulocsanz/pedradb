/* RFC-0050 P2.3 — PID 1: format or run world_smoke on an IDE disk.
 *
 * Kernel cmdline:
 *   pedra_mode=format  — mkfs.vfat on the first disk, poweroff
 *   pedra_mode=run     — mount and exec /world_smoke 42 (TMPDIR=/data)
 */
#define _GNU_SOURCE
#include <fcntl.h>
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

static int run(char *const argv[]) {
  pid_t p = fork();
  if (p == 0) {
    execv(argv[0], argv);
    perror(argv[0]);
    _exit(127);
  }
  if (p < 0) {
    perror("fork");
    return 1;
  }
  int st = 0;
  (void)waitpid(p, &st, 0);
  if (WIFEXITED(st)) {
    return WEXITSTATUS(st);
  }
  return 1;
}

static int cmdline_has(const char *needle) {
  int fd = open("/proc/cmdline", O_RDONLY);
  if (fd < 0) {
    return 0;
  }
  char buf[512];
  ssize_t n = read(fd, buf, sizeof(buf) - 1);
  close(fd);
  if (n <= 0) {
    return 0;
  }
  buf[n] = 0;
  return strstr(buf, needle) != NULL;
}

static const char *find_disk(void) {
  static const char *cands[] = {"/dev/sda", "/dev/hda", "/dev/vda", "/dev/sdb", 0};
  struct stat st;
  for (int i = 0; cands[i]; i++) {
    if (stat(cands[i], &st) == 0) {
      return cands[i];
    }
  }
  return 0;
}

int main(void) {
  (void)mkdir("/proc", 0755);
  (void)mkdir("/sys", 0755);
  (void)mkdir("/tmp", 0755);
  (void)mkdir("/dev", 0755);
  (void)mkdir("/data", 0755);
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
  (void)setenv("HOME", "/tmp", 1);

  /* virtio_blk + vfat are modules in linux-virt. */
  {
    char *ins[][8] = {
        {"/busybox", "insmod", "/mod/nls_cp437.ko", 0},
        {"/busybox", "insmod", "/mod/nls_iso8859-1.ko", 0},
        {"/busybox", "insmod", "/mod/nls_utf8.ko", 0},
        {"/busybox", "insmod", "/mod/fat.ko", 0},
        {"/busybox", "insmod", "/mod/vfat.ko", 0},
        {"/busybox", "insmod", "/mod/virtio_blk.ko", 0},
        {0},
    };
    for (int i = 0; ins[i][0]; i++) {
      (void)run(ins[i]);
    }
    char *mdev[] = {"/busybox", "mdev", "-s", 0};
    (void)run(mdev);
    for (int i = 0; i < 40 && !find_disk(); i++) {
      usleep(50000);
      (void)run(mdev);
    }
  }

  const char *disk = find_disk();
  if (!disk) {
    say("NO_DISK\n");
    goto off;
  }
  say("disk=");
  say(disk);
  say("\n");

  if (cmdline_has("pedra_mode=format")) {
    say("BLK_FORMAT_BEGIN\n");
    char *mk[] = {"/busybox", "mkfs.vfat", (char *)disk, 0};
    int rc = run(mk);
    if (rc != 0) {
      say("MKFS_FAIL\n");
    } else {
      say("FORMAT_OK\n");
    }
    goto off;
  }

  say("BLK_RUN_BEGIN\n");
  char *mnt[] = {"/busybox", "mount", "-t", "vfat", (char *)disk, "/data", 0};
  if (run(mnt) != 0) {
    say("MOUNT_FAIL\n");
    goto off;
  }
  (void)setenv("TMPDIR", "/data", 1);
  pid_t p = fork();
  if (p == 0) {
    char *argv[] = {"/world_smoke", "42", 0};
    execv("/world_smoke", argv);
    perror("exec world_smoke");
    _exit(127);
  }
  if (p > 0) {
    int st = 0;
    (void)waitpid(p, &st, 0);
  }
  (void)umount("/data");
  say("BLK_RUN_END\n");

off:
  (void)sync();
  (void)reboot(RB_POWER_OFF);
  for (;;) {
    (void)pause();
  }
}
