
CREATE TABLE installation_format (
  singleton INTEGER PRIMARY KEY CHECK(singleton=1),
  schema_version INTEGER NOT NULL CHECK(schema_version=1),
  installation_id BLOB NOT NULL CHECK(length(installation_id)=32),
  bootstrap_receipt BLOB
) STRICT;
CREATE TABLE deny_tokens (
  token BLOB NOT NULL PRIMARY KEY CHECK(length(token)=32),
  kind TEXT NOT NULL CHECK(kind IN ('NeverResubmit','RetiredRoot'))
) STRICT, WITHOUT ROWID;
CREATE TABLE protected_records (
  record_id BLOB NOT NULL PRIMARY KEY CHECK(length(record_id)=32),
  class TEXT NOT NULL CHECK(class IN ('Envelope','Cleanup','Activation')),
  key_slot BLOB NOT NULL UNIQUE REFERENCES purge_jobs(key_slot) CHECK(length(key_slot)=32),
  part_index INTEGER NOT NULL CHECK(part_index>=0),
  expires_at INTEGER CHECK(expires_at IS NULL OR expires_at>=0),
  deadline INTEGER GENERATED ALWAYS AS (coalesce(expires_at,-1)) STORED,
  nonce BLOB NOT NULL CHECK(length(nonce)=12),
  ciphertext BLOB NOT NULL CHECK(length(ciphertext)=65536),
  UNIQUE(key_slot,nonce),
  CHECK(class<>'Envelope' OR (part_index=0 AND expires_at IS NOT NULL)),
  FOREIGN KEY(key_slot,deadline) REFERENCES purge_jobs(key_slot,deadline) DEFERRABLE INITIALLY DEFERRED
) STRICT;
CREATE TABLE purge_jobs (
  key_slot BLOB NOT NULL PRIMARY KEY CHECK(length(key_slot)=32),
  due_at INTEGER CHECK(due_at IS NULL OR due_at>=0),
  deadline INTEGER GENERATED ALWAYS AS (coalesce(due_at,-1)) STORED,
  phase TEXT NOT NULL CHECK(phase IN ('Retained','PurgeIntent','KeyRemoved','RowsRemoved','Purged')),
  last_error_code INTEGER,
  UNIQUE(key_slot,deadline)
) STRICT;
CREATE TABLE activation_receipts (
  root_token BLOB NOT NULL PRIMARY KEY CHECK(length(root_token)=32),
  phase TEXT NOT NULL CHECK(phase IN ('MigrationIntent','LegacyDrained','PayloadsImported',
    'LegacyFenced','V2Provisioned','Active','Retired')),
  qualification_digest BLOB NOT NULL CHECK(length(qualification_digest)=32)
) STRICT;
CREATE INDEX protected_records_expiring ON protected_records(expires_at) WHERE expires_at IS NOT NULL;
CREATE INDEX purge_jobs_pending ON purge_jobs(due_at) WHERE phase<>'Retained';
