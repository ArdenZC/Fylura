import os
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

from prototype import Fenced, Prototype, TABLES, connect, rows
from schema37 import columns


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='issue-380-generation-')
        self.addCleanup(self.temp.cleanup)
        self.serial = 0

    def fixture(self, strategy, n=10):
        self.serial += 1
        p = Prototype(Path(self.temp.name)/f'{self.serial}.db', strategy, n)
        self.addCleanup(p.c.close)
        return p

    def assert_preserved(self, p):
        self.assertEqual(p.history_rows(), p.history)
        self.assertEqual(p.c.execute('PRAGMA foreign_key_check').fetchall(), [])
        self.assertEqual(p.c.execute('PRAGMA integrity_check').fetchone()[0], 'ok')
        for table in ('global_entries_fts', 'files_fts'):
            p.c.execute(f"INSERT INTO {table}({table},rank) VALUES('integrity-check',1)")

    def test_0_1_1k_10k_full_projection_and_search(self):
        for n in (0, 1, 1000, 10000):
            for strategy in ('C1', 'C2', 'C3'):
                with self.subTest(n=n, strategy=strategy):
                    print(f'correctness fixture {strategy}/{n}', flush=True)
                    p = self.fixture(strategy, n)
                    for t in TABLES:
                        self.assertEqual(columns(p.c, 'rp_'+t)[1:], columns(p.c, t))
                    p.stage()
                    self.assertEqual(p.read(), p.old)
                    self.assertEqual(p.search('newtoken'), [])
                    p.publish()
                    self.assertEqual(p.read(), p.expected)
                    expected_hits = [r['id'] for r in sorted(p.expected['global_entries'],
                                     key=lambda r: (-r['modified_at_fs'], r['id'])) if not r['is_stale']]
                    self.assertEqual(p.search('newtoken'), expected_hits)
                    self.assert_preserved(p)
                    p.close()
                    # Free disk incrementally; addCleanup tolerates repeated close.
                    for suffix in ('', '-wal', '-shm'):
                        Path(p.path+suffix).unlink(missing_ok=True)

    def test_cancel_revision_disabled_identity_metadata_fences(self):
        changes = ("UPDATE rp_control SET revision=2", "UPDATE global_volumes SET enabled=0",
                   "UPDATE global_volumes SET stable_volume_id='replaced'",
                   "UPDATE global_volumes SET mount_path='D:/fixture'",
                   "UPDATE global_volumes SET provider='other'",
                   "UPDATE rp_control SET metadata_revision=metadata_revision+1")
        for strategy in ('C1', 'C2', 'C3'):
            for change in changes:
                p = self.fixture(strategy)
                p.stage()
                p.c.execute(change)
                before = p.read()
                with self.assertRaises(Fenced):
                    p.publish()
                self.assertEqual(p.read(), before)
            p = self.fixture(strategy)
            p.stage()
            p.cancel()
            with self.assertRaises(Fenced):
                p.publish()
            with self.assertRaises(Fenced):
                p.fence(p.c)
            self.assertEqual(p.read(), p.old)

    def test_sql_failure_each_active_table_and_pointer_rolls_back(self):
        for strategy in ('C1', 'C2', 'C3'):
            points = (*TABLES, 'pointer') if strategy == 'C1' else ('pointer',)
            for point in points:
                p = self.fixture(strategy)
                p.stage()
                with self.assertRaises(sqlite3.OperationalError):
                    p.publish(fail_after=point)
                self.assertEqual(p.read(), p.old)
                self.assertEqual(p.search('newtoken'), [])
                self.assert_preserved(p)

    def test_staging_sql_failure_incomplete_manifest_is_invisible(self):
        for strategy in ('C1', 'C2', 'C3'):
            p = self.fixture(strategy, 1000)
            p.c.execute('''CREATE TRIGGER rp_fail BEFORE INSERT ON rp_files
                           WHEN NEW.id='stable-file-500' BEGIN SELECT RAISE(ABORT,'stage failure'); END''')
            with self.assertRaises(sqlite3.IntegrityError):
                p.stage()
            self.assertEqual(p.read(), p.old)
            with self.assertRaises(Fenced):
                p.publish()
            p.cancel()
            self.assertEqual(p.read(), p.old)

    def test_process_exit_wal_before_and_after_commit(self):
        for strategy in ('C1', 'C2', 'C3'):
            for phase in ('stage', 'publish', 'committed'):
                p = self.fixture(strategy, 1000)
                if phase != 'stage':
                    p.stage()
                script = '''
import os,sys
from prototype import Prototype
p=Prototype(sys.argv[1],sys.argv[2])
if sys.argv[3]=='stage':
    p.stage(hook=lambda t,s: os._exit(73))
elif sys.argv[3]=='publish':
    p.publish(step_hook=lambda step: os._exit(73) if step in ('ai_job_items','pointer') else None)
else:
    p.publish()
    os._exit(73)
'''
                child = subprocess.run([sys.executable, '-c', script, p.path, strategy, phase],
                                       cwd=Path(__file__).parent, timeout=30)
                self.assertEqual(child.returncode, 73)
                recovered = connect(p.path)
                try:
                    self.assertEqual(p.read(recovered), p.expected if phase == 'committed' else p.old)
                    self.assertEqual(recovered.execute('PRAGMA integrity_check').fetchone()[0], 'ok')
                    if phase != 'committed':
                        # Recovery fences unfinished work; never silently resumes it.
                        recovered.execute("UPDATE rp_control SET revision=revision+1,phase='unknown'")
                        with self.assertRaises(Fenced):
                            p.publish()
                finally:
                    recovered.close()

    def test_cross_table_read_snapshot_and_generation_lease_gc(self):
        for strategy in ('C1', 'C2', 'C3'):
            p = self.fixture(strategy)
            p.stage()
            reader = connect(p.path)
            self.addCleanup(reader.close)
            reader.execute('BEGIN')
            self.assertEqual(p.read(reader), p.old)
            p.c.execute("INSERT INTO rp_leases VALUES('reader',1)")
            p.publish()
            self.assertEqual(p.read(reader), p.old)
            self.assertEqual(p.search('newtoken', reader), [])
            self.assertFalse(p.gc())
            reader.execute('COMMIT')
            self.assertEqual(p.read(reader), p.expected)
            if strategy != 'C1':
                self.assertEqual(p.read(generation=1), p.old)
            p.c.execute("DELETE FROM rp_leases WHERE token='reader'")
            self.assertTrue(p.gc())
            self.assertEqual(p.read(), p.expected)
            self.assert_preserved(p)

    def test_identity_and_correction_retention(self):
        for strategy in ('C1', 'C2', 'C3'):
            p = self.fixture(strategy)
            p.stage()
            p.publish()
            new = p.read()
            self.assertEqual([r['id'] for r in new['files']], [r['id'] for r in p.old['files']])
            self.assertNotEqual(new['global_entries'][0]['id'], new['global_entries'][1]['id'])
            self.assertEqual(new['global_entries'][0]['platform_file_id'], new['global_entries'][1]['platform_file_id'])
            self.assertTrue(all(r['user_corrected'] == 1 for r in new['ai_analysis_state']))
            self.assertTrue(all(r['classification_json'] == '{"purpose":"user correction"}' for r in new['ai_analysis_state']))
            self.assertEqual(sum(r['status']=='canceled' for r in new['ai_jobs']), 5)
            self.assert_preserved(p)

    def test_metadata_writer_is_not_lost_by_staged_snapshot(self):
        for strategy in ('C1', 'C2', 'C3'):
            p = self.fixture(strategy)
            p.stage()
            writer = connect(p.path)
            with p.transaction('user-correction', writer) as c:
                c.execute("UPDATE ai_analysis_state SET classification_json='new user correction'")
                c.execute('UPDATE rp_control SET metadata_revision=metadata_revision+1')
            writer.close()
            with self.assertRaises(Fenced):
                p.publish()
            self.assertTrue(all(r['classification_json']=='new user correction'
                                for r in rows(p.c, 'ai_analysis_state')))

    def test_negative_controls_legacy_pointer_read_and_naive_chunk_publish(self):
        for strategy in ('C2', 'C3'):
            p = self.fixture(strategy)
            p.stage()
            p.publish()
            # Required BLOCKED witness: untouched production-shaped readers see old state.
            self.assertNotEqual(rows(p.c, 'global_entries'), p.read()['global_entries'])
            self.assertNotEqual(rows(p.c, 'ai_analysis_state'), p.read()['ai_analysis_state'])
            self.assertNotEqual(rows(p.c, 'files'), p.read()['files'])
            legacy_hits = p.c.execute('''SELECT count(*) FROM global_entries_fts
                                         WHERE global_entries_fts MATCH 'newtoken' ''').fetchone()[0]
            self.assertEqual(legacy_hits, 0)
            self.assertGreater(len(p.search('newtoken')), 0)
        p = self.fixture('C1')
        p.c.execute("UPDATE global_entries SET path='C:/newtoken' WHERE id='entry-0'")
        self.assertNotEqual(p.read(), p.old)
        self.assertNotEqual(p.read(), p.expected)
        self.assertEqual(p.c.execute('SELECT journal_cursor FROM global_volumes').fetchone()[0], 'old')

    def test_negative_control_unfenced_legacy_correction_writer(self):
        for strategy in ('C1', 'C2', 'C3'):
            p = self.fixture(strategy)
            p.stage()
            p.c.execute("UPDATE ai_analysis_state SET classification_json='late legacy correction'")
            p.publish()
            # Legacy writers do not participate in the proposed metadata revision.
            # Even C1 can overwrite late state if implemented as a cached merge.
            self.assertTrue(all(r['classification_json'] != 'late legacy correction'
                                for r in p.read()['ai_analysis_state']))

    def test_cancel_between_batches_and_old_stage_revision(self):
        for strategy in ('C1', 'C2', 'C3'):
            p = self.fixture(strategy, 1000)
            cancelled = False
            def hook(table, offset):
                nonlocal cancelled
                if not cancelled:
                    p.cancel()
                    cancelled = True
            with self.assertRaises(Fenced):
                p.stage(hook=hook)
            self.assertEqual(p.read(), p.old)
            with self.assertRaises(Fenced):
                p.publish()

    def test_negative_control_new_identity_anchor_is_visible_to_legacy_reader(self):
        # Adding a stable FK anchor is not a free solution for new namespace IDs.
        p = self.fixture('C2', 1)
        p.c.execute('''INSERT INTO global_entries
            (id,volume_id,platform_file_id,name,name_normalized,path,path_normalized,source_provider,last_seen_at)
            VALUES('new-namespace','v','new-physical','new.txt','new.txt','C:/new.txt','c:/new.txt','ntfs',2)''')
        self.assertEqual(len(p.read()['global_entries']), 1)
        self.assertEqual(len(rows(p.c,'global_entries')), 2)

    def test_writer_5000ms_success_and_timeout_contract(self):
        # Deliberate lock holds are controls, not workload performance measurements.
        for hold, expected in ((0.05, 'committed'), (5.25, 'sqlite_busy')):
            p = self.fixture('C2', 1)
            result = {}
            start = threading.Event()
            def writer():
                c = connect(p.path)
                start.set()
                began = time.perf_counter()
                try:
                    c.execute('BEGIN IMMEDIATE')
                    c.execute("INSERT INTO app_settings VALUES('writer','ok')")
                    c.execute('COMMIT')
                    result['outcome'] = 'committed'
                except sqlite3.OperationalError as e:
                    result['outcome'] = 'sqlite_busy'
                    result['code'] = e.sqlite_errorcode
                finally:
                    result['wait_ms'] = (time.perf_counter()-began)*1000
                    c.close()
            # Prepare connection before taking lock; start flag proves an attempt.
            thread = threading.Thread(target=writer)
            p.c.execute('BEGIN IMMEDIATE')
            thread.start()
            self.assertTrue(start.wait(1))
            time.sleep(hold)
            p.c.execute('COMMIT')
            thread.join(2)
            self.assertFalse(thread.is_alive())
            self.assertEqual(result['outcome'], expected)
            if expected == 'sqlite_busy':
                self.assertEqual(result['code'], sqlite3.SQLITE_BUSY)
                self.assertGreaterEqual(result['wait_ms'], 4900)


if __name__ == '__main__':
    unittest.main(verbosity=2)
