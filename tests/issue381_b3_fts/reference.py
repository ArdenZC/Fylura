"""Independent trigram/phrase BM25 arithmetic over fixture strings.

ASCII case plus case-stable CJK is sufficient for the large fixtures. Native
visible-only trigram separately checks the Unicode matrix; this reference never
imports model SQL or reads F2's df/docsize/phrase-position results.
"""
import math
import struct


def visible(fixture,scope=None):
    enabled={r['rid'] for r in fixture['roots'] if r['enabled']}
    return [d for d in fixture['docs'] if d['active'] and enabled.intersection(d['roots'])
            and (scope is None or scope in d['roots'] and scope in enabled)]


def frequency(text,phrase):
    text=text.lower();phrase=phrase.lower()
    return sum(text.startswith(phrase,start) for start in range(len(text)))


def rank(fixture,query,scope=None):
    phrases=query.split()[:12]
    if not phrases or any(len(p)<3 for p in phrases): return []
    corpus=visible(fixture)
    n=len(corpus)
    if not n: return []
    average=sum(max(len(d['name'])-2,0)+max(len(d['path'])-2,0) for d in corpus)/n
    dfs=[sum(bool(frequency(d['name'],p)+frequency(d['path'],p)) for d in corpus) for p in phrases]
    idfs=[math.log((n-df+0.5)/(df+0.5)) for df in dfs]
    idfs=[value if value>0 else 1e-6 for value in idfs]
    rows=[]
    for d in visible(fixture,scope):
        freqs=[(frequency(d['name'],p),frequency(d['path'],p)) for p in phrases]
        if any(a+b==0 for a,b in freqs): continue
        dl=max(len(d['name'])-2,0)+max(len(d['path'])-2,0)
        score=0.0
        for (a,b),idf in zip(freqs,idfs):
            f=a*6.0+b*1.5
            score+=idf*((f*(1.2+1.0))/(f+1.2*(1-0.75+0.75*dl/average)))
        score=-score
        rows.append(dict(rowid=d['rowid'],uid=d['uid'],score=score,
                         bits=struct.pack('>d',score).hex()))
    docs={d['uid']:d for d in corpus}
    def ascii_nocase(text):return ''.join(chr(ord(c)+32) if 'A'<=c<='Z' else c for c in text)
    return sorted(rows,key=lambda r:(r['score'],-docs[r['uid']]['mtime'],ascii_nocase(docs[r['uid']]['name']),r['uid']))
