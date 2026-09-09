import { defineProject, service } from "@caixote/iac";

/** RFC-0185 P0.3: Linux 4 vCPU cartaz — overwrite_mc4 3-round min >1.0. */
export default defineProject("pedradb-linux-p149", () => ({
  services: [
    service("linux-gate-p185", {
      type: "container",
      role: "worker",
      image: "ghcr.io/paulocsanz/pedradb-linux-gate:p185a",
      region: "brasil",
      cpus: 4,
      memory_mb: 4096,
      disk_mb: 8192,
    }),
  ],
}));
