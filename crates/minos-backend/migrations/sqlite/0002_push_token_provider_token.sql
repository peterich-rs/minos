-- Persist provider device token alongside hash so APNs/FCM can deliver.
-- Hash remains the primary key for upsert/revoke; provider_token is the send material.
ALTER TABLE push_tokens ADD COLUMN provider_token TEXT;
