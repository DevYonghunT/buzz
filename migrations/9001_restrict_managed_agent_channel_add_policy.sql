-- Managed agents must not be attachable to arbitrary channels without their
-- owner's consent. Preserve the stricter `nobody` policy while backfilling
-- persisted NIP-OA mappings created before owner assignment tightened the
-- default atomically.
UPDATE users
SET channel_add_policy = 'owner_only'::channel_add_policy
WHERE agent_owner_pubkey IS NOT NULL
  AND channel_add_policy = 'anyone'::channel_add_policy;
