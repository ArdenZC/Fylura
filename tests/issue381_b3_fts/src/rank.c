/* Test-only FTS5 auxiliary. Native trigram phrase positions, effective statistics.
 * This is not a production extension or a replacement tokenizer. */
#include "sqlite3.h"
#include <math.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
  unsigned char *visible;
  sqlite3_int64 maxid, n, tokens;
} Corpus;
typedef struct { Corpus *corpus; sqlite3_int64 count; } Counter;
typedef struct { int phrases; double *idf; sqlite3_int64 *df; } Cache;
static void free_corpus(void *p) {
  Corpus *c=(Corpus*)p; free(c->visible); free(c);
}
static void free_cache(void *p) {
  Cache *c=(Cache*)p; free(c->idf); free(c->df); free(c);
}
static int visible(Corpus *c, sqlite3_int64 id) {
  return id>=0 && id<=c->maxid && c->visible[id];
}
static int hit(const Fts5ExtensionApi *api,Fts5Context *fts,void *p) {
  Counter *counter=(Counter*)p;
  if(visible(counter->corpus,api->xRowid(fts))) counter->count++;
  return SQLITE_OK;
}
static Cache *cache(const Fts5ExtensionApi *api,Fts5Context *fts,sqlite3_context *ctx) {
  Cache *c=(Cache*)api->xGetAuxdata(fts,0);
  Corpus *corpus=(Corpus*)api->xUserData(fts);
  if(c) return c;
  c=(Cache*)calloc(1,sizeof(Cache));
  if(!c) { sqlite3_result_error_nomem(ctx); return NULL; }
  c->phrases=api->xPhraseCount(fts);
  c->idf=(double*)calloc(c->phrases,sizeof(double));
  c->df=(sqlite3_int64*)calloc(c->phrases,sizeof(sqlite3_int64));
  if(!c->idf || !c->df) { free_cache(c); sqlite3_result_error_nomem(ctx); return NULL; }
  for(int i=0;i<c->phrases;i++) {
    Counter counter={corpus,0};
    int rc=api->xQueryPhrase(fts,i,&counter,hit);
    if(rc!=SQLITE_OK) { free_cache(c); sqlite3_result_error_code(ctx,rc); return NULL; }
    c->df[i]=counter.count;
    c->idf[i]=log(((double)corpus->n-counter.count+0.5)/(counter.count+0.5));
    if(c->idf[i]<=0.0) c->idf[i]=1e-6;
  }
  if(api->xSetAuxdata(fts,c,free_cache)!=SQLITE_OK) { sqlite3_result_error_nomem(ctx); return NULL; }
  return c;
}
static void effective_rank(const Fts5ExtensionApi *api,Fts5Context *fts,sqlite3_context *ctx,int argc,sqlite3_value **argv) {
  (void)argc; (void)argv;
  Corpus *corpus=(Corpus*)api->xUserData(fts);
  if(!visible(corpus,api->xRowid(fts)) || corpus->n==0) { sqlite3_result_null(ctx); return; }
  Cache *c=cache(api,fts,ctx); if(!c) return;
  double *freq=(double*)calloc(c->phrases,sizeof(double));
  if(!freq) { sqlite3_result_error_nomem(ctx); return; }
  int count=0, dl=0,rc=api->xInstCount(fts,&count);
  for(int i=0;rc==SQLITE_OK && i<count;i++) {
    int phrase,col,offset;
    rc=api->xInst(fts,i,&phrase,&col,&offset);
    if(rc==SQLITE_OK) freq[phrase] += col==0 ? 6.0 : col==1 ? 1.5 : 0.0;
  }
  if(rc==SQLITE_OK) rc=api->xColumnSize(fts,-1,&dl);
  if(rc!=SQLITE_OK) { free(freq); sqlite3_result_error_code(ctx,rc); return; }
  const double k1=1.2,b=0.75;
  double avg=(double)corpus->tokens/(double)corpus->n,score=0.0;
  for(int i=0;i<c->phrases;i++) {
    score += c->idf[i]*((freq[i]*(k1+1.0))/(freq[i]+k1*(1-b+b*dl/avg)));
  }
  free(freq); sqlite3_result_double(ctx,-score);
}
static void metrics(const Fts5ExtensionApi *api,Fts5Context *fts,sqlite3_context *ctx,int argc,sqlite3_value **argv) {
  (void)argc; (void)argv;
  Cache *c=cache(api,fts,ctx); if(!c) return;
  int dl=0,count=0;
  if(api->xColumnSize(fts,-1,&dl)!=SQLITE_OK || api->xInstCount(fts,&count)!=SQLITE_OK) {sqlite3_result_error(ctx,"metrics unavailable",-1);return;}
  int *freq=(int*)calloc(c->phrases*2,sizeof(int));
  if(!freq) {sqlite3_result_error_nomem(ctx);return;}
  for(int i=0;i<count;i++) {int p,col,offset; if(api->xInst(fts,i,&p,&col,&offset)==SQLITE_OK && col<2) freq[p*2+col]++;}
  sqlite3_str *s=sqlite3_str_new(NULL);
  sqlite3_str_appendf(s,"{\"dl\":%d,\"phrases\":[",dl);
  for(int i=0;i<c->phrases;i++) sqlite3_str_appendf(s,"%s{\"df\":%lld,\"tokens\":%d,\"name_frequency\":%d,\"path_frequency\":%d}",i?",":"",c->df[i],api->xPhraseSize(fts,i),freq[i*2],freq[i*2+1]);
  sqlite3_str_appendall(s,"]}"); free(freq);
  char *result=sqlite3_str_finish(s); sqlite3_result_text(ctx,result,-1,sqlite3_free);
}
int b3f_register(sqlite3 *db,sqlite3_int64 n,sqlite3_int64 tokens,const sqlite3_int64 *ids,int length) {
  fts5_api *api=NULL; sqlite3_stmt *stmt=NULL;
  int rc=sqlite3_prepare_v2(db,"SELECT fts5(?1)",-1,&stmt,NULL);
  if(rc==SQLITE_OK) {sqlite3_bind_pointer(stmt,1,&api,"fts5_api_ptr",NULL);sqlite3_step(stmt);}
  sqlite3_finalize(stmt);
  if(rc!=SQLITE_OK || !api) return SQLITE_ERROR;
  for(int f=0;f<2;f++) {
    Corpus *c=(Corpus*)calloc(1,sizeof(Corpus)); if(!c) return SQLITE_NOMEM;
    c->n=n;c->tokens=tokens;
    for(int i=0;i<length;i++) if(ids[i]>c->maxid) c->maxid=ids[i];
    c->visible=(unsigned char*)calloc(c->maxid+1,1);
    if(!c->visible) {free(c);return SQLITE_NOMEM;}
    for(int i=0;i<length;i++) c->visible[ids[i]]=1;
    rc=api->xCreateFunction(api,f?"effective_metrics":"effective_bm25",c,f?metrics:effective_rank,free_corpus);
    if(rc!=SQLITE_OK) {free_corpus(c);return rc;}
  }
  return SQLITE_OK;
}
