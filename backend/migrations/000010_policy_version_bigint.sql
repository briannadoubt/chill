ALTER TABLE control.sampling_policies
  ALTER COLUMN version TYPE bigint;

ALTER TABLE control.privacy_policies
  ALTER COLUMN version TYPE bigint;
