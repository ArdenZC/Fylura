-- RESEARCH ONLY. Not Zen Canvas Schema 37, not a proposed migration.
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
CREATE TABLE control(source TEXT PRIMARY KEY, revision INTEGER NOT NULL, owner TEXT, route TEXT);
CREATE TABLE health(source TEXT PRIMARY KEY, status TEXT, cause TEXT, checkpoint INTEGER, full_time INTEGER, incremental_time INTEGER, count INTEGER, reconciliation INTEGER DEFAULT 0, metadata TEXT DEFAULT 'original', provider TEXT DEFAULT 'old-provider');
CREATE TABLE intent(source TEXT PRIMARY KEY, key TEXT UNIQUE, state TEXT, attempts INTEGER DEFAULT 0, generation INTEGER NOT NULL);
CREATE TABLE operations(request TEXT PRIMARY KEY, source TEXT, revision INTEGER, token TEXT, route TEXT, state TEXT, effects INTEGER DEFAULT 0, instance TEXT, command TEXT, intent_generation INTEGER, actor_pid INTEGER NOT NULL);
CREATE TABLE offers(request TEXT PRIMARY KEY, instance TEXT, token TEXT, source TEXT, revision INTEGER, expires INTEGER, granted INTEGER DEFAULT 0);
CREATE TABLE stage(request TEXT, entry TEXT, value TEXT, PRIMARY KEY(request,entry));
CREATE TABLE global_rows(source TEXT, entry TEXT, value TEXT, stale INTEGER, PRIMARY KEY(source,entry));
CREATE TABLE managed(source TEXT, entry TEXT, enabled INTEGER, correction TEXT, PRIMARY KEY(source,entry));
CREATE TABLE ai_jobs(source TEXT, entry TEXT, state TEXT, PRIMARY KEY(source,entry));
CREATE TABLE ai_items(source TEXT, entry TEXT, state TEXT, PRIMARY KEY(source,entry));
CREATE TABLE ai_analysis(source TEXT, entry TEXT, state TEXT, PRIMARY KEY(source,entry));
CREATE TABLE service_slot(singleton INTEGER PRIMARY KEY CHECK(singleton=1), instance TEXT, reserved TEXT);
CREATE TABLE lookup_registry(lookup TEXT PRIMARY KEY, request TEXT);
