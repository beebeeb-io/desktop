-- state-v2.db; all IDs are random 32-byte BLOBs, never path/content hashes.
CREATE TABLE v2_store (
  singleton INTEGER PRIMARY KEY CHECK(singleton=1),
  schema_version INTEGER NOT NULL CHECK(schema_version=1),
  store_id BLOB NOT NULL UNIQUE CHECK(length(store_id)=32),
  account_binding BLOB NOT NULL, root_token BLOB NOT NULL CHECK(length(root_token)=32),
  bootstrap_receipt BLOB, migration_receipt BLOB,
  mode TEXT NOT NULL CHECK(mode IN ('Fixture','BootstrapPending','RecoveryOnly','Active')),
  quota_bytes INTEGER NOT NULL CHECK(quota_bytes>0)
) STRICT;
CREATE TABLE v2_owners (
  owner_id BLOB NOT NULL PRIMARY KEY CHECK(length(owner_id)=32),
  kind TEXT NOT NULL CHECK(kind IN ('FileControl','Snapshot','Upload','Finalization',
    'Resolution','RemoteCatchUp','InstallIntent','RecoveryCase','Migration','RootRetirement')),
  revision INTEGER NOT NULL CHECK(revision>=0),
  body_version INTEGER NOT NULL CHECK(body_version=1), body BLOB NOT NULL CHECK(length(body)<=65536),
  terminal_disposition BLOB REFERENCES v2_dispositions(disposition_id)
) STRICT;
CREATE TABLE v2_artifacts (
  artifact_id BLOB NOT NULL PRIMARY KEY CHECK(length(artifact_id)=32),
  allocation_owner BLOB NOT NULL REFERENCES v2_owners(owner_id),
  purpose TEXT NOT NULL CHECK(purpose IN ('Capture','Preimage','Replacement','LegacyRecovery','ExportCache')),
  phase TEXT NOT NULL CHECK(phase IN ('Allocated','Ready','Referenced','RecoveryOwned','RetirePending','Removed')),
  format TEXT NOT NULL CHECK(format IN ('Full','LegacyPartial','Unknown')),
  expected_bytes INTEGER CHECK(expected_bytes>=0),
  reservation_id BLOB NOT NULL REFERENCES v2_reservations(reservation_id)
) STRICT;
CREATE TABLE v2_chunks (
  artifact_id BLOB NOT NULL REFERENCES v2_artifacts(artifact_id),
  chunk_index INTEGER NOT NULL CHECK(chunk_index>=0),
  byte_length INTEGER NOT NULL CHECK(byte_length BETWEEN 1 AND 4194304),
  digest BLOB NOT NULL CHECK(length(digest)=32),
  payload BLOB NOT NULL CHECK(length(payload)=byte_length),
  PRIMARY KEY(artifact_id,chunk_index)
) STRICT, WITHOUT ROWID;
CREATE TABLE v2_manifests (
  artifact_id BLOB NOT NULL PRIMARY KEY REFERENCES v2_artifacts(artifact_id),
  format_version INTEGER NOT NULL CHECK(format_version=1),
  logical_eof INTEGER NOT NULL CHECK(logical_eof>=0),
  packed_bytes INTEGER NOT NULL CHECK(packed_bytes>=0),
  chunk_count INTEGER NOT NULL CHECK(chunk_count>=0),
  extent_count INTEGER NOT NULL CHECK(extent_count>=0),
  content_digest BLOB NOT NULL CHECK(length(content_digest)=32),
  manifest_digest BLOB NOT NULL CHECK(length(manifest_digest)=32),
  base_object BLOB, source_witness BLOB NOT NULL
) STRICT;
CREATE TABLE v2_extents (
  artifact_id BLOB NOT NULL REFERENCES v2_artifacts(artifact_id),
  ordinal INTEGER NOT NULL CHECK(ordinal>=0),
  logical_offset INTEGER NOT NULL CHECK(logical_offset>=0),
  packed_offset INTEGER NOT NULL CHECK(packed_offset>=0),
  byte_length INTEGER NOT NULL CHECK(byte_length>0),
  PRIMARY KEY(artifact_id,ordinal)
) STRICT, WITHOUT ROWID;
CREATE TABLE v2_refs (
  owner_id BLOB NOT NULL REFERENCES v2_owners(owner_id),
  artifact_id BLOB NOT NULL REFERENCES v2_artifacts(artifact_id),
  role TEXT NOT NULL CHECK(role IN ('Snapshot','Preimage','Replacement','Recovery','Export')),
  PRIMARY KEY(owner_id,artifact_id,role)
) STRICT, WITHOUT ROWID;
CREATE TABLE v2_dispositions (
  disposition_id BLOB NOT NULL PRIMARY KEY CHECK(length(disposition_id)=32),
  owner_id BLOB NOT NULL REFERENCES v2_owners(owner_id),
  kind TEXT NOT NULL CHECK(kind IN ('ServerReceipt','UserHandoff','UserDiscard','OwnershipTransfer')),
  proof_version INTEGER NOT NULL CHECK(proof_version=1), proof BLOB NOT NULL CHECK(length(proof)<=65536)
) STRICT;
CREATE TABLE v2_transfers (
  transfer_id BLOB NOT NULL PRIMARY KEY CHECK(length(transfer_id)=32),
  owner_id BLOB NOT NULL REFERENCES v2_owners(owner_id),
  deny_token BLOB NOT NULL UNIQUE CHECK(length(deny_token)=32),
  phase TEXT NOT NULL CHECK(phase IN ('Intent','LedgerCommitted','Acknowledged','Detached')),
  ledger_record_id BLOB CHECK(ledger_record_id IS NULL OR length(ledger_record_id)=32), ledger_receipt BLOB,
  CHECK(phase='Intent' OR (ledger_record_id IS NOT NULL AND ledger_receipt IS NOT NULL))
) STRICT;
CREATE TABLE v2_reservations (
  reservation_id BLOB NOT NULL PRIMARY KEY CHECK(length(reservation_id)=32),
  owner_id BLOB NOT NULL REFERENCES v2_owners(owner_id),
  volume_binding BLOB NOT NULL,
  payload_bytes INTEGER NOT NULL CHECK(payload_bytes>=0),
  duplicate_bytes INTEGER NOT NULL CHECK(duplicate_bytes>=0),
  wal_bytes INTEGER NOT NULL CHECK(wal_bytes>=0),
  terminal_bytes INTEGER NOT NULL CHECK(terminal_bytes>=0),
  consumed_bytes INTEGER NOT NULL CHECK(consumed_bytes>=0),
  phase TEXT NOT NULL CHECK(phase IN ('Reserved','Consuming','TerminalOnly','Released'))
) STRICT;
CREATE TABLE v2_gc (
  artifact_id BLOB NOT NULL PRIMARY KEY REFERENCES v2_artifacts(artifact_id),
  disposition_id BLOB NOT NULL REFERENCES v2_dispositions(disposition_id),
  next_chunk INTEGER NOT NULL CHECK(next_chunk>=0),
  phase TEXT NOT NULL CHECK(phase IN ('Pending','Deleting','Removed')),
  last_error_code INTEGER
) STRICT;
CREATE INDEX v2_refs_by_artifact ON v2_refs(artifact_id);
CREATE INDEX v2_owners_by_kind ON v2_owners(kind,owner_id);
CREATE INDEX v2_gc_by_phase ON v2_gc(phase,artifact_id);

