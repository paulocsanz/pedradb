/*
 * Minimal QEMU TCG plugin: count guest instructions + guest data memory ops.
 * Not cycle-accurate. Guest insn count is the machine-independent *work*
 * metric; wall time under TCG is not.
 *
 * Build (macOS, loaded by homebrew qemu-system):
 *   clang -shared -fPIC -undefined dynamic_lookup \
 *     -I "$(brew --prefix qemu)/include" \
 *     $(pkg-config --cflags glib-2.0) \
 *     -o libinsn_count.dylib insn_count.c
 */

#include <glib.h>
#include <inttypes.h>
#include <stdio.h>
#include <qemu-plugin.h>

QEMU_PLUGIN_EXPORT int qemu_plugin_version = QEMU_PLUGIN_VERSION;

static struct qemu_plugin_scoreboard *insn_sb;
static struct qemu_plugin_scoreboard *mem_sb;
static qemu_plugin_u64 insn_count;
static qemu_plugin_u64 mem_count;

static void tb_exec(unsigned int vcpu_index, void *userdata)
{
    uint64_t n = (uint64_t)(uintptr_t)userdata;
    qemu_plugin_u64_add(insn_count, vcpu_index, n);
}

static void vcpu_tb_trans(qemu_plugin_id_t id, struct qemu_plugin_tb *tb)
{
    size_t n = qemu_plugin_tb_n_insns(tb);
    /* Callback every execution (inline scoreboard can under-count if the
     * host build elides it). n is the TB width — added on each exec. */
    qemu_plugin_register_vcpu_tb_exec_cb(
        tb, tb_exec, QEMU_PLUGIN_CB_NO_REGS, (void *)(uintptr_t)n);

    for (size_t i = 0; i < n; i++) {
        struct qemu_plugin_insn *insn = qemu_plugin_tb_get_insn(tb, i);
        qemu_plugin_register_vcpu_mem_inline_per_vcpu(
            insn, QEMU_PLUGIN_MEM_RW, QEMU_PLUGIN_INLINE_ADD_U64, mem_count, 1);
    }
    (void)id;
}

static void plugin_exit(qemu_plugin_id_t id, void *p)
{
    char buf[160];
    g_snprintf(buf, sizeof(buf),
               "guest_insns=%" PRIu64 " guest_memops=%" PRIu64 "\n",
               qemu_plugin_u64_sum(insn_count),
               qemu_plugin_u64_sum(mem_count));
    qemu_plugin_outs(buf);
    (void)id;
    (void)p;
}

QEMU_PLUGIN_EXPORT int qemu_plugin_install(qemu_plugin_id_t id,
                                           const qemu_info_t *info,
                                           int argc, char **argv)
{
    (void)argc;
    (void)argv;
    (void)info;
    insn_sb = qemu_plugin_scoreboard_new(sizeof(uint64_t));
    mem_sb = qemu_plugin_scoreboard_new(sizeof(uint64_t));
    insn_count = qemu_plugin_scoreboard_u64(insn_sb);
    mem_count = qemu_plugin_scoreboard_u64(mem_sb);
    qemu_plugin_register_vcpu_tb_trans_cb(id, vcpu_tb_trans);
    qemu_plugin_register_atexit_cb(id, plugin_exit, NULL);
    return 0;
}
