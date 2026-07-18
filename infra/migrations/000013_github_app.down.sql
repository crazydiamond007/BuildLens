ALTER TABLE organizations DROP CONSTRAINT IF EXISTS organizations_github_installation_fkey;
DROP TABLE IF EXISTS github_installations;
