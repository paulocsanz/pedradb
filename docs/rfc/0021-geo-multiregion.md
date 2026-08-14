# RFC-0021 P2.6 — Geo / multi-region (lab v0)

**Status:** lab done  
**Parent:** [0021](0021-montanha-fdb-tikv-parity-gaps.md)

## Lab capability (shipped)

- `StoreCluster::set_node_region` / `node_region` / `dial_order_prefer_region`
- `TcpClusterClient::set_region` / `set_prefer_region` / `first_dial_id`
- Regions appear in `cluster_status_json` under `"regions"`
- Tests: `region_prefer_dial_order`, `tcp_region_prefer_first_dial`

## Not claimed

- Production geo-HA, WAN RTT, multi-region majority, or async DR.  
- Field parity with FDB multi-region configs.
