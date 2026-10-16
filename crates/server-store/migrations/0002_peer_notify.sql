-- Publie chaque ajout ou révocation d'appareil sur le canal « ankou_peers » (ADR 0004).
-- Le déclencheur couvre l'API comme ankou-admin ; la notification part au COMMIT.

CREATE FUNCTION ankou_notify_peer() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'INSERT' AND NEW.revoked_at IS NULL THEN
        PERFORM pg_notify('ankou_peers', json_build_object(
            'op', 'added', 'public_key', NEW.public_key, 'ipv4', host(NEW.ipv4))::text);
    ELSIF TG_OP = 'UPDATE' AND OLD.revoked_at IS NULL AND NEW.revoked_at IS NOT NULL THEN
        PERFORM pg_notify('ankou_peers', json_build_object(
            'op', 'removed', 'public_key', NEW.public_key)::text);
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER devices_notify_peer
    AFTER INSERT OR UPDATE OF revoked_at ON devices
    FOR EACH ROW EXECUTE FUNCTION ankou_notify_peer();
