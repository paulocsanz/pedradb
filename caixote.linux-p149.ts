import { defineProject, service } from "@caixote/iac";

/** RFC-0149 P2.1: Linux 4 vCPU coluna A majority >3×. One-shot worker. */
export default defineProject("pedradb-linux-p149", () => ({
  services: [
    service("linux-gate-p149", {
      type: "container",
      role: "worker",
      image: "ghcr.io/paulocsanz/pedradb-linux-gate:p149a",
      region: "brasil",
      cpus: 4,
      memory_mb: 4096,
      disk_mb: 8192,
    }),
  ],
}));
