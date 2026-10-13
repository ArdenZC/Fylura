"""Independent small-set authority oracle. No model imports or SQL visibility query.

ASCII fixtures intentionally avoid pretending to qualify production trigram/Unicode.
BM25 implements SQLite FTS5's single indexed column formula independently.
"""
import math
import re
from collections import Counter
from dataclasses import dataclass, field


@dataclass
class Reference:
    sources: dict = field(default_factory=dict)
    roots: dict = field(default_factory=dict)
    members: dict = field(default_factory=dict)
    revision: int = 0

    def source(self, uid, path, physical, text, revision=1):
        for previous, source in self.sources.items():
            if previous != uid and source['path'] == path:
                source['active'] = False
        self.sources[uid] = dict(uid=uid, path=path, physical=physical, active=True,
                                 text=text, observation=revision)

    def root(self, rid, path):
        self.roots[rid] = dict(path=path, enabled=True, epoch=0,
                               revision=0, watcher=0)
        self.members[rid] = {}

    def publish(self, rid, epoch, uids):
        self.members[rid] = {uid: self.sources[uid]['observation'] for uid in uids}
        self.roots[rid]['epoch'] = epoch
        self.roots[rid]['revision'] += 1
        self.revision += 1

    def visible(self, scope=None):
        permitted = set(self.roots) if scope is None else set(scope)
        found = set()
        for rid, root in self.roots.items():
            if rid not in permitted or not root['enabled']:
                continue
            for uid, observed in self.members[rid].items():
                if self.sources[uid]['active'] and self.sources[uid]['observation'] == observed:
                    found.add(uid)
        return sorted(found)

    def view(self, scope=None):
        return dict(ids=self.visible(scope), count=len(self.visible(scope)),
                    revision=self.revision,
                    roots={rid: [r['epoch'], r['revision'], r['watcher'], r['enabled']]
                           for rid, r in sorted(self.roots.items())
                           if scope is None or rid in scope})

    def rank(self, query, scope=None, limit=None):
        corpus = self.visible()  # Root scope filters results, not global corpus statistics.
        tokens = {uid: Counter(re.findall(r'[a-z0-9]+', self.sources[uid]['text'].lower()))
                  for uid in corpus}
        term = query.lower()
        n = len(corpus)
        if not n:
            return []
        df = sum(bool(t[term]) for t in tokens.values())
        avg = sum(sum(t.values()) for t in tokens.values()) / n
        idf = max(1e-6, math.log((n - df + 0.5) / (df + 0.5)))
        allowed = set(self.visible(scope))
        rows = []
        for uid, t in tokens.items():
            freq = t[term]
            if freq and uid in allowed:
                score = -idf * (freq * 2.2) / (freq + 1.2 * (0.25 + 0.75 * sum(t.values()) / avg))
                rows.append([uid, score])
        rows.sort(key=lambda r: (r[1], r[0]))
        return rows if limit is None else rows[:limit]
