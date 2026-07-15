-- Revoke every privilege the up migration granted. The roles themselves are
-- created outside migrations (init/01-roles.sh) and are left alone.
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM buildlens_gateway;
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM buildlens_analytics;
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM buildlens_ai;
REVOKE ALL ON ALL TABLES IN SCHEMA public FROM buildlens_services;
