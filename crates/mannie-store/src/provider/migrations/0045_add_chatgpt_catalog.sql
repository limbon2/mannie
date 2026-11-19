-- A catalog model keeps its profile ID after it leaves the account's list,
-- so a pinned conversation still names the model it used.
ALTER TABLE provider_profiles ADD COLUMN catalog_account TEXT;
ALTER TABLE provider_profiles ADD COLUMN catalog_position INTEGER
  CHECK (catalog_position IS NULL OR (catalog_position >= 0 AND catalog_account IS NOT NULL));
ALTER TABLE provider_profiles ADD COLUMN catalog_efforts TEXT
  CHECK (catalog_efforts IS NULL OR json_type(catalog_efforts) = 'array');

CREATE UNIQUE INDEX provider_catalog_slug ON provider_profiles (model)
WHERE catalog_account IS NOT NULL;

DROP INDEX provider_profiles_name;
CREATE UNIQUE INDEX provider_profiles_name ON provider_profiles (name COLLATE NOCASE)
WHERE catalog_account IS NULL;

-- An owner-configured plan profile can use the advertised efforts of its
-- model too. An endpoint with the same model name has its own capabilities.
CREATE VIEW provider_profile_choices AS
SELECT p.*, COALESCE(p.catalog_efforts, c.catalog_efforts) AS supported_efforts
FROM provider_profiles AS p
LEFT JOIN provider_profiles AS c
  ON p.catalog_account IS NULL AND json_extract(p.document, '$.access') = 'chatgpt_plan'
  AND c.catalog_account IS NOT NULL AND c.catalog_position IS NOT NULL AND c.model = p.model;
