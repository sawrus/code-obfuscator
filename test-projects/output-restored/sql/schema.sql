-- antifraud schema
CREATE TABLE antifraud_events (
    event_id BIGSERIAL PRIMARY KEY,
    freeze_reason TEXT NOT NULL,
    customer_id TEXT NOT NULL,
    score NUMERIC(5,2) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_antifraud_events_customer ON antifraud_events(customer_id);

CREATE OR REPLACE FUNCTION freeze_high_risk_customer(p_customer_id TEXT)
RETURNS VOID AS $$
BEGIN
    INSERT INTO antifraud_events(freeze_reason, customer_id, score)
    VALUES ('AUTO_FREEZE', p_customer_id, 97.50);
END;
$$ LANGUAGE plpgsql;
