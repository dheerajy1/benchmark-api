-- PostgreSQL bootstrap for benchmark-api
--
-- Run ONCE, manually, as a PostgreSQL administrator (a superuser, or a role with
-- CREATEROLE and CREATEDB):
--
--   psql -U postgres -f docs/postgres/bootstrap.sql
--
-- Replace <CHANGE_ME> with a real password before running. Never commit the real password.
-- Tables, constraints, indexes and lookup rows are created by the migrations in
-- ./migrations when the application starts, never by hand.
--
-- The role gets grants only, it does not own the database.
-- CREATE DATABASE cannot run inside a transaction, so run this file outside BEGIN/COMMIT.

CREATE DATABASE benchmark_api_db;

CREATE ROLE benchmark_api_service LOGIN PASSWORD '<CHANGE_ME>';

-- CONNECT, CREATE (the tests create temporary schemas) and TEMP on the database
GRANT ALL PRIVILEGES ON DATABASE benchmark_api_db TO benchmark_api_service;

-- Schema grants apply per database, so switch to the new database first.
-- With a GUI client, remove the next line and connect to benchmark_api_db yourself.
\connect benchmark_api_db

-- PostgreSQL 15+ no longer lets every role create tables in public, the migrations need this
GRANT ALL ON SCHEMA public TO benchmark_api_service;