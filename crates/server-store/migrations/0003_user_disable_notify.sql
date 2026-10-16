-- Désactiver un compte retire ses appareils des nodes ; le réactiver les y remet (ADR 0004).

CREATE FUNCTION ankou_notify_user_peers() RETURNS trigger AS $$
DECLARE
    d record;
BEGIN
    IF (OLD.disabled_at IS NULL) = (NEW.disabled_at IS NULL) THEN
        RETURN NEW;
    END IF;
    FOR d IN SELECT public_key, ipv4 FROM devices WHERE user_id = NEW.id AND revoked_at IS NULL LOOP
        IF NEW.disabled_at IS NOT NULL THEN
            PERFORM pg_notify('ankou_peers', json_build_object(
                'op', 'removed', 'public_key', d.public_key)::text);
        ELSE
            PERFORM pg_notify('ankou_peers', json_build_object(
                'op', 'added', 'public_key', d.public_key, 'ipv4', host(d.ipv4))::text);
        END IF;
    END LOOP;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER users_notify_peers
    AFTER UPDATE OF disabled_at ON users
    FOR EACH ROW EXECUTE FUNCTION ankou_notify_user_peers();
