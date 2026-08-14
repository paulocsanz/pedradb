import { defineProject, service } from "@caixote/iac";

/**
 * Montanha / mesh lab on caixote.
 *
 * Modes (env):
 *   MONTANHA_MODE=local3   (default) — 3 processes one VM
 *   MONTANHA_MODE=netprobe — L3 canary: TCP to peer *.internal (or SCAN_MESH)
 *   MONTANHA_MODE=mesh     — 3 Raft singles; PEERS use *.internal DNS names
 *
 * Private DNS: `<service>-<project8>.internal` (platform tag + project suffix).
 * Never bake raw 10.0.0.x into PEERS — IPs reassign on restart.
 *
 * Images:
 *   local3: MONTANHA_TCP_IMAGE (default …:p01-health2)
 *   mesh:   MONTANHA_TCP_IMAGE_BASE + {1,2,3} (default …:m12-n → m12-n1/n2/n3)
 *           bake CLUSTER/NODE_ID/membership (caixote env inject race)
 *
 * Project id prefix for tag DNS (first 8 hex of uuid, no dashes):
 *   MONTANHA_PROJECT_TAG_SUFFIX=c8d3f0c9
 *
 * Deploy:
 *   ./scripts/montanha_tcp_caixote.sh local3
 *   ./scripts/montanha_tcp_caixote.sh mesh
 *   MONTANHA_MESH_REV=m13 ./scripts/montanha_tcp_caixote.sh wire
 */
export default defineProject("pedradb-montanha", () => {
  const image =
    process.env.MONTANHA_TCP_IMAGE ||
    "ghcr.io/paulocsanz/montanha-tcp:p01-health2";
  const mode = process.env.MONTANHA_MODE || "local3";
  const rev = process.env.MONTANHA_REV || "m13";
  // Matches portaria unique_network_tags: first 8 hex chars of project_id.
  const tagSuffix =
    process.env.MONTANHA_PROJECT_TAG_SUFFIX || "c8d3f0c9";
  const internalHost = (serviceName: string) =>
    `${serviceName}-${tagSuffix}.internal`;

  if (mode === "netprobe") {
    // Pure L3 canary — PEERS via stable DNS, not mesh IPs.
    const n = Number(process.env.NETPROBE_COUNT || "3");
    const services = [];
    const names = Array.from(
      { length: n },
      (_, i) => `netprobe-${i + 1}-${rev}`
    );
    for (let i = 1; i <= n; i++) {
      const self = names[i - 1];
      const peers = names
        .filter((n) => n !== self)
        .map((n) => `${internalHost(n)}:9701`)
        .join(" ");
      services.push(
        service(self, {
          type: "container",
          role: "worker",
          image,
          region: "brasil",
          cpus: 1,
          memory_mb: 256,
          disk_mb: 512,
          ports: [{ port: 9701, mode: "tcp" }],
          env: {
            CLUSTER: "netprobe",
            PORT: "9701",
            SELF_NAME: self,
            PEERS: peers,
            // Optional: still scan 10.0.0.100-120 if DNS VIP path is wrong for L3 canary
            SCAN_MESH: process.env.NETPROBE_SCAN_MESH || "0",
            PROBE_INTERVAL_SECS: "5",
          },
        })
      );
    }
    return { services };
  }

  if (mode === "mesh") {
    // Raft PEERS: stable *.internal names (no hard-coded 10.0.0.x).
    // Per-node image tags bake CLUSTER=single + NODE_ID (caixote often boots
    // before service env inject — same workaround as netprobe bake).
    // Image override: MONTANHA_TCP_IMAGE_BASE=ghcr.io/.../montanha-tcp:m10-n
    //   → m10-n1 / m10-n2 / m10-n3
    const names = [1, 2, 3].map((id) => `mtcp-${id}-${rev}`);
    const peersLine = [1, 2, 3]
      .map((id) => `${id}=${internalHost(names[id - 1])}:9701`)
      .join(" ");
    const imageBase =
      process.env.MONTANHA_TCP_IMAGE_BASE ||
      "ghcr.io/paulocsanz/montanha-tcp:m10-n";
    const mkNode = (id: number) =>
      service(names[id - 1], {
        type: "container",
        role: "worker",
        image: `${imageBase}${id}`,
        region: "brasil",
        cpus: 1,
        memory_mb: 512,
        disk_mb: 2048,
        ports: [
          { port: 9701, mode: "tcp" },
          { port: 9780, mode: "tcp" }, // HTTP /leader /ready /follower /status
        ],
        env: {
          CLUSTER: "single",
          NODE_ID: String(id),
          BIND: "0.0.0.0:9701",
          HEALTH_BIND: "0.0.0.0:9780",
          DATA_DIR: "/data",
          RANGES: "1",
          PEERS: peersLine,
        },
      });
    const services = [mkNode(1), mkNode(2), mkNode(3)];
    // Optional RoleAware write proxy as 4th service (MONTANHA_MESH_PROXY=1).
    if (process.env.MONTANHA_MESH_PROXY === "1") {
      const members = names
        .map((n) => `${internalHost(n)}:9701`)
        .join(" ");
      // Bake CLUSTER=proxy — first-boot env inject often leaves CLUSTER empty → local3.
      const img =
        process.env.MONTANHA_PROXY_IMAGE ||
        "ghcr.io/paulocsanz/montanha-tcp:p02-proxy-svc";
      services.push(
        service(`mtcp-proxy-${rev}`, {
          type: "container",
          role: "worker",
          image: img,
          region: "brasil",
          cpus: 1,
          memory_mb: 256,
          disk_mb: 1024,
          ports: [{ port: 9600, mode: "tcp" }],
          env: {
            CLUSTER: "proxy",
            PROXY_LISTEN: "0.0.0.0:9600",
            PROXY_MODE: "write",
            MEMBERS: members,
            // PEERS backup if MEMBERS empty
            PEERS: peersLine,
          },
        })
      );
    }
    return { services };
  }

  // local3 default
  const cluster = service(`mtcp-cluster-${rev}`, {
    type: "container",
    role: "worker",
    image,
    region: "brasil",
    cpus: 1,
    memory_mb: 1024,
    disk_mb: 2048,
    ports: [
      { port: 9701, mode: "tcp" },
      { port: 9702, mode: "tcp" },
      { port: 9703, mode: "tcp" },
    ],
    env: {
      CLUSTER: "local3",
      DATA_DIR: "/data",
      SMOKE_ON_START: "1",
      RANGES: "1",
    },
  });
  return { services: [cluster] };
});
