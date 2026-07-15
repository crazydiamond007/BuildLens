-- Extensions and shared helpers.
--
-- Requires Postgres 18, which is what docker-compose pins. The dependency is
-- uuidv7(), added to core in 18. On 16 or 17 it has to be hand-rolled in
-- plpgsql, and a hand-rolled primary key generator is not a thing worth owning
-- when the database will do it for you.

CREATE EXTENSION IF NOT EXISTS pgcrypto;   -- gen_random_bytes, digest
CREATE EXTENSION IF NOT EXISTS citext;     -- case-insensitive email / slug

-- Every primary key in this schema is a uuidv7.
--
-- Why not v4: the primary key is the leading column of the table's main index.
-- Random v4 keys scatter inserts across the whole B-tree, dirtying pages
-- everywhere and blowing out the cache. v7 keys embed a millisecond timestamp in
-- their high bits, so they sort chronologically. Inserts land at the right edge
-- of the index and behave like a sequence. On workflow_runs and test_results,
-- the two tables that actually get big, that is the difference between an index
-- that stays hot and one that thrashes.
--
-- Why not bigserial, which has the same locality: three services in three
-- languages generate these IDs, and none of them can know a sequence value
-- without a round trip to the database first. A uuidv7 can be generated in Rust,
-- Java or Python before the row exists. This is what makes the transactional
-- outbox possible, since the event and the row it describes need the same ID.

-- Keeps updated_at honest. Attached by trigger to every table that has one, so
-- that no service in any of the three languages can forget to set it, including
-- the ones we have not written yet.
CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;
