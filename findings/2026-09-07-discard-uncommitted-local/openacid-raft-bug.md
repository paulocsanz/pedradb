# The Pitfalls of Raft Membership Change (OpenACID)

Source: https://blog.openacid.com/distributed/raft-bug/
Fetched: 2026-09-07

Primary claim used this turn: “A committed change stays visible. If one
change is already committed, every uncommitted change must be
recognizable as uncommitted. Otherwise a new leader cannot tell which of
them to keep.”

Single-server membership change (Ongaro gist) walks M(abc) → M(abcd) →
M(bcd). The four-node middle state is a single point of failure under
the split `ad | bc`. Joint consensus’s middle quorum set is
M(abc)×M(bcd) = M(abcd) ∪ {bc}, which survives that split.

A second bug (Ongaro 2015): a leader may append a new configuration
before committing an entry of its current term; a later election under
an uncommitted config can overwrite a committed membership entry. The
fix: a leader may not append a new configuration until it has committed
an entry from its current term. That extra commit turns a correct
single-server change into two log commits — the same cost as joint
consensus.

Not claimed here: Pedra switches algorithms. The plant is only that a
local replica still discards its uncommitted suffix when it is no
longer in `ids`.
