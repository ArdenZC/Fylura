"""Portable deterministic datasets; no random draws and no SQLite dependency."""
SEED=38131020261013
ROOTS=[{'rid':'A','enabled':True,'health':'healthy'},
       {'rid':'B','enabled':True,'health':'reconciliation_required'},
       {'rid':'D','enabled':False,'health':'disabled'}]


def document(i,uid,name,path,roots,active=True):
    return dict(rowid=i,uid=uid,name=name,path=path,roots=roots,active=active,
                physical='synthetic-volume:inode-'+uid,observation=1,mtime=(i%3)*100)


def small():
    docs=[document(1,'alpha','Needle Needle common','/Projects/资料/Design/build/needle.txt',['A','B','D']),
          document(2,'beta','common needle','/Projects/资料/设计文档/needle.ai',['B']),
          document(3,'gamma','common C++ 图标.ai','/Projects/Windows/用户资料/alpha_[x].txt',['A']),
          document(4,'unicode','设计文档 𠮷野家 Café é','/资料/设计文档/中文/𠮷野家.ai',['A','B']),
          document(5,'disabled','common needle','/elsewhere/disabled/needle',['D']),
          document(6,'quoted','report "quote" 100%_[x]','/资料/报告.v2/c++.ai',['B']),
          document(7,'short','AI x','/资料/图/ab.txt',['B']),
          document(8,'rare','common raretoken','/selected/raretoken-report.txt',['A'])]
    for i in range(9,25):
        docs.append(document(i,f'hidden-{i}','common needle raretoken '*30,'/private/cache/common/'+'needle/'*40+str(i),[],False))
    return dict(seed=SEED,roots=ROOTS,docs=docs,queries=['needle','common','common raretoken','Needle common','设计文档','𠮷野家','Café','é','C++','[x]','"quote"','100%','设计','AI','x'])


def scale(size):
    docs=[]
    for i in range(size):
        hidden=i%5==0
        name='common needle raretoken '*12 if hidden else f'Common asset {i:06} needle'+(' raretoken' if i%997==1 else '')
        path=f'/Work/设计文档/build/Common-needle-{i:06}.dat'
        roots=[] if hidden else ['A','B'] if i%11==0 else ['A' if i%2 else 'B']
        docs.append(document(i+1,f'source-{i:06}',name,path,roots,not hidden))
    return dict(seed=SEED,roots=ROOTS,docs=docs,queries=['common','common raretoken'])
