CREATE TABLE IF NOT EXISTS lobby_layout (
    id smallint PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    layout jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS player_sessions (
    player_uuid uuid NOT NULL,
    realm text NOT NULL,
    session jsonb NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (player_uuid, realm)
);
