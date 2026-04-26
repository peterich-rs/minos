CREATE TABLE accounts (
    account_id     TEXT PRIMARY KEY,
    email          TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash  TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    last_login_at  INTEGER
) STRICT;

CREATE UNIQUE INDEX idx_accounts_email ON accounts(email);
