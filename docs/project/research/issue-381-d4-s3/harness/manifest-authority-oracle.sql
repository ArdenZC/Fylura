-- Independent bidirectional Manifest projection Oracle.
-- Views are deliberately separate from effective_file_sources and from the
-- publication triggers so the test compares authority inputs to projection
-- outputs rather than reusing the product-facing view under test.

CREATE VIEW d4s3_oracle_expected_paths AS
SELECT DISTINCT
    mr.manifest_id,
    gm.source_id,
    gm.path_entry_id,
    pe.path_key,
    pe.path_text AS path_text
FROM manifest_root_heads AS mr
JOIN source_generation_members AS gm
  ON gm.generation_id = mr.generation_id
JOIN file_path_entries AS pe
  ON pe.path_entry_id = gm.path_entry_id
 AND pe.source_id = gm.source_id;

CREATE VIEW d4s3_oracle_actual_paths AS
SELECT manifest_id, source_id, path_entry_id, path_key,
       projected_path AS path_text
FROM manifest_file_paths;

CREATE VIEW d4s3_oracle_missing_paths AS
SELECT manifest_id, source_id, path_entry_id, path_key, path_text
FROM d4s3_oracle_expected_paths
EXCEPT
SELECT manifest_id, source_id, path_entry_id, path_key, path_text
FROM d4s3_oracle_actual_paths;

CREATE VIEW d4s3_oracle_extra_paths AS
SELECT manifest_id, source_id, path_entry_id, path_key, path_text
FROM d4s3_oracle_actual_paths
EXCEPT
SELECT manifest_id, source_id, path_entry_id, path_key, path_text
FROM d4s3_oracle_expected_paths;

CREATE VIEW d4s3_oracle_invalid_identity AS
SELECT mr.manifest_id, gm.generation_id, gm.source_id, gm.path_entry_id,
       pe.identity_status, pe.physical_identity_id, pe.evidence_digest,
       gm.identity_digest, fs.status AS source_status,
       fs.identity_confidence, pi.identity_epoch, pi.platform_file_identity
FROM manifest_root_heads AS mr
JOIN source_generation_members AS gm
  ON gm.generation_id = mr.generation_id
LEFT JOIN file_path_entries AS pe
  ON pe.path_entry_id = gm.path_entry_id
 AND pe.source_id = gm.source_id
LEFT JOIN file_sources AS fs ON fs.source_id = gm.source_id
LEFT JOIN file_physical_identity_epochs AS pi
  ON pi.physical_identity_id = pe.physical_identity_id
WHERE pe.path_entry_id IS NULL
   OR pe.identity_status <> 'verified'
   OR pe.physical_identity_id IS NULL
   OR pe.evidence_digest IS NULL OR pe.evidence_digest = ''
   OR gm.identity_digest IS NULL OR gm.identity_digest = ''
   OR gm.identity_digest <> pe.evidence_digest
   OR pi.physical_identity_id IS NULL
   OR pi.identity_epoch IS NULL OR pi.identity_epoch = ''
   OR pi.platform_file_identity IS NULL OR pi.platform_file_identity = ''
   OR fs.status <> 'observed'
   OR fs.identity_confidence NOT IN ('native_verified', 'user_claimed')
   OR (fs.identity_confidence = 'user_claimed' AND NOT EXISTS (
       SELECT 1 FROM source_identity_claims AS sic
       WHERE sic.source_id = gm.source_id
         AND sic.path_entry_id = gm.path_entry_id
         AND sic.claim_kind = 'explicit_legacy_claim'
   ));

CREATE VIEW d4s3_oracle_generation_summary AS
SELECT mr.manifest_id, mr.root_id, mr.generation_id,
       sg.status AS generation_status,
       sg.member_count AS declared_member_count,
       (SELECT count(*) FROM source_generation_members AS gm
        WHERE gm.generation_id = sg.generation_id) AS actual_member_count,
       sr.status AS scan_status,
       sr.coverage_complete
FROM manifest_root_heads AS mr
JOIN source_generations AS sg ON sg.generation_id = mr.generation_id
JOIN scan_runs AS sr ON sr.id = sg.run_id;
