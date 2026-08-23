/*
 * QEMU 8.2 plugin API (Ubuntu 24.04 qemu-user). Counts guest insns + mem ops
 * with the pre-scoreboard inline helpers.
 */
#include <inttypes.h>
#include <qemu-plugin.h>
#include <stdio.h>

QEMU_PLUGIN_EXPORT int qemu_plugin_version = QEMU_PLUGIN_VERSION;

static uint64_t insn_count;
static uint64_t mem_count;

static void vcpu_tb_trans(qemu_plugin_id_t id, struct qemu_plugin_tb *tb)
{
    size_t n = qemu_plugin_tb_n_insns(tb);
    qemu_plugin_register_vcpu_tb_exec_inline(
        tb, QEMU_PLUGIN_INLINE_ADD_U64, &insn_count, n);
    for (size_t i = 0; i < n; i++) {
        struct qemu_plugin_insn *insn = qemu_plugin_tb_get_insn(tb, i);
        qemu_plugin_register_vcpu_mem_inline(
            insn, QEMU_PLUGIN_MEM_RW, QEMU_PLUGIN_INLINE_ADD_U64, &mem_count, 1);
    }
    (void)id;
}

static void plugin_exit(qemu_plugin_id_t id, void *p)
{
    char buf[160];
    snprintf(buf, sizeof buf,
             "guest_insns=%" PRIu64 " guest_memops=%" PRIu64 "\n",
             insn_count, mem_count);
    qemu_plugin_outs(buf);
    (void)id;
    (void)p;
}

QEMU_PLUGIN_EXPORT int qemu_plugin_install(qemu_plugin_id_t id,
                                           const qemu_info_t *info,
                                           int argc, char **argv)
{
    (void)info;
    (void)argc;
    (void)argv;
    qemu_plugin_register_vcpu_tb_trans_cb(id, vcpu_tb_trans);
    qemu_plugin_register_atexit_cb(id, plugin_exit, NULL);
    return 0;
}
