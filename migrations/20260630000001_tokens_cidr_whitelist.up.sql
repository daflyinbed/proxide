ALTER TABLE tokens ADD COLUMN cidr_whitelist TEXT DEFAULT NULL AFTER allowed_scopes;
