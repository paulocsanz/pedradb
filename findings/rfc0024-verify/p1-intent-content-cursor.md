# P1 Intent cursor is content, not wall clock

**Date:** 2026-08-15

## Hole

1. Incremental `IntentSync` stamps `intent_version = timestamp_millis`.
   `IntentCache::apply_sync` stored that on every tick. Reconnect sent the
   timestamp as `last_intent_version`. Federation compares it to
   `intent_content_version(list_*)` → mismatch → full desired dump.
2. A disk that still holds a timestamp / generation in
   `observed_fold.json` did the same on first reopen after deploy.
3. `skip_intent_deletion` used `version <= cursor`. Content hashes are not
   ordered.

## Close

- Agent `intent_version()` **recomputes** the live-row content hash
  (tombstones excluded — federation `list_*` is `deleted_at IS NULL`).
  Old timestamp on disk self-heals: restore intents, register the hash.
- Incremental apply does **not** store the wire timestamp.
- Incremental still must not `save()` the fold (partial delta is not SoR).
- `skip_intent_deletion` is equality of content cursors.
- Golden `intent_content_version([vm-1, vm, true, 1, running]) ==
  2794727690938848288` in both `seq_sync` and `observed_fold`.

## Not closed

- Incremental senders still put wall-clock on the wire (debug / uniqueness).
  The skip key is no longer that field.
- Crash after incremental, before the next full_sync persist, still dumps
  (last on-disk snapshot is the previous full picture). That is the
  persist-only-on-full_sync contract.
