CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS tsm_system_rows;

CREATE TYPE userkind AS ENUM ('super', 'normal');

CREATE TABLE users (
  id UUID PRIMARY KEY NOT NULL DEFAULT gen_random_uuid(),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  kind USERKIND NOT NULL DEFAULT 'normal',
  points INTEGER NOT NULL DEFAULT 0,
  email TEXT UNIQUE NOT NULL,
  password TEXT NOT NULL,
  nickname TEXT NOT NULL
);

CREATE TABLE cards (
  id UUID PRIMARY KEY NOT NULL DEFAULT gen_random_uuid(),
  rating DOUBLE PRECISION NOT NULL DEFAULT 1000.0,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  gacha_queue_index FLOAT NOT NULL DEFAULT random(),
  owned_at TIMESTAMPTZ,
  owner_id UUID REFERENCES users (id),
  creator_id UUID REFERENCES users (id),
  -- match_queued_at TIMESTAMPTZ,
  -- is_battle BOOLEAN,
  image_path TEXT NOT NULL,
  name TEXT NOT NULL
);

CREATE OR REPLACE FUNCTION randomize_gacha_queue_index()
RETURNS TRIGGER AS $$
BEGIN
  NEW.gacha_queue_index = random();
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER randomize_cards_gacha_queue_index
    BEFORE UPDATE ON cards
    FOR EACH ROW
    EXECUTE PROCEDURE randomize_gacha_queue_index();

CREATE INDEX cards_gacha_queue_index_idx ON cards (gacha_queue_index) WHERE owner_id IS NULL;

CREATE TYPE battlestate AS ENUM ('matching', 'fighting', 'finished');

CREATE TABLE duel_battles (
  id UUID PRIMARY KEY NOT NULL DEFAULT gen_random_uuid(),

  voteup1 INT NOT NULL DEFAULT 0,
  voteup2 INT NOT NULL DEFAULT 0,

  state BATTLESTATE NOT NULL DEFAULT 'matching',

  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

  rating DOUBLE PRECISION NOT NULL,

  card_id1 UUID NOT NULL,
  user_id1 UUID NOT NULL,

  card_id2 UUID,
  user_id2 UUID,

  expired_at TIMESTAMPTZ
);

CREATE INDEX duel_battles_rating_idx ON duel_battles (rating) WHERE state != 'finished';
CREATE UNIQUE INDEX duel_battles_card_id1_idx ON duel_battles (card_id1) WHERE state != 'finished';
CREATE UNIQUE INDEX duel_battles_card_id2_idx ON duel_battles (card_id2) WHERE state != 'finished';

CREATE INDEX duel_battles_expired_at_idx ON duel_battles (expired_at DESC) WHERE state = 'fighting';

CREATE INDEX duel_battles_user_id1_idx ON duel_battles (user_id1);
CREATE INDEX duel_battles_user_id2_idx ON duel_battles (user_id2);
