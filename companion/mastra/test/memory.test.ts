import { afterAll, describe, expect, it } from 'vitest';
import { createMemory, EMBED_DIM, fuse, memoryConfig, repairDocuments, type MemoryDoc } from '../src/memory.ts';

const detail = {
  id: 'inh_fn',
  full_name: 'kachayev/fn.py',
  successor_name: 'fn.py-successor',
  replay: null,
  issues: [{ id: 'iss_1', title: "ImportError: cannot import name 'Iterable' from 'collections'", diagnosis: 'collections ABCs moved to collections.abc in Python 3.10' }],
  repairs: [
    { id: 'rep_kept', issue_id: 'iss_1', status: 'kept', commit: 'ed0b4e4aa11', files: ['fn/iters.py'], rationale: 'import from collections.abc', review: { verdict: 'approve', concerns: [] } },
    { id: 'rep_bad', issue_id: 'iss_1', status: 'reverted', files: ['fn/func.py'], probe: { verdict: 'changed', note: '3 changed' }, note: 'tests passed but the behaviour probe found changed behaviour; reverted' },
  ],
};

describe('restoration memory documents', () => {
  it('turns every recorded repair into a searchable document with its outcome and evidence', () => {
    const docs = repairDocuments(detail);
    expect(docs.map((d) => [d.id, d.status, d.title])).toEqual([
      ['rep_kept', 'kept', "ImportError: cannot import name 'Iterable' from 'collections'"],
      ['rep_bad', 'reverted', "ImportError: cannot import name 'Iterable' from 'collections'"],
    ]);
    expect(docs[0].body).toContain('outcome: kept as commit ed0b4e4aa1');
    expect(docs[1].body).toContain('behaviour probe: changed (3 changed)');
    // The engine's reason sits right after the outcome, so it survives truncation into excerpts.
    expect(docs[1].body.split('\n')[2]).toMatch(/^why \(engine\): tests passed but the behaviour probe/);
  });
  it('leaves replays out, since they repeat a recorded run', () => {
    expect(repairDocuments({ ...detail, replay: { fixture: 'fn-py-prflags' } })).toEqual([]);
    expect(repairDocuments({ ...detail, successor_name: 'fn.py-successor (replay)' })).toEqual([]);
  });
});

describe('hybrid ranking', () => {
  it('fuses keyword and vector ranks, rewarding documents both find', () => {
    const r = fuse(['a', 'b', 'c'], ['c', 'd']);
    expect(r[0]).toMatchObject({ id: 'c', matched_by: ['keyword', 'vector'] });
    expect(r.map((x) => x.id).sort()).toEqual(['a', 'b', 'c', 'd']);
  });
});

describe('memory configuration', () => {
  it('is off unless a database is configured', () => {
    expect(memoryConfig({})).toBeNull();
  });
  it('accepts only loopback databases and embedding endpoints', () => {
    expect(memoryConfig({ IE_MEMORY_DATABASE_URL: 'postgres://ie:ie@127.0.0.1:5433/memory' })?.embedModel).toBe('text-embedding-nomic-embed-text-v1.5');
    expect(() => memoryConfig({ IE_MEMORY_DATABASE_URL: 'postgres://u:p@db.tsdb.cloud.timescale.com:5432/tsdb' })).toThrow(/loopback/);
    expect(() => memoryConfig({ IE_MEMORY_DATABASE_URL: 'postgres://ie:ie@localhost/memory', IE_EMBED_ENDPOINT: 'https://api.example.com/v1' })).toThrow(/loopback/);
  });
});

// Against a real Postgres with pgvector and pgvectorscale (Tiger Data's timescaledb-ha image), when one is configured:
//   IE_TEST_MEMORY_DATABASE_URL=postgres://postgres:ie@127.0.0.1:5433/postgres npm test
// Embeddings are a deterministic bag-of-words stand-in so the ranking is checkable without a model.
const testDb = process.env.IE_TEST_MEMORY_DATABASE_URL;
describe.skipIf(!testDb)('restoration memory on Postgres (pgvector + pgvectorscale)', () => {
  const vocab = new Map<string, number>();
  const toVector = (text: string) => {
    const v = new Array(EMBED_DIM).fill(0);
    for (const w of text.toLowerCase().replace(/^search_\w+: /, '').match(/[a-z]+/g) ?? []) {
      if (!vocab.has(w)) vocab.set(w, vocab.size % EMBED_DIM);
      v[vocab.get(w)!] += 1;
    }
    return v;
  };
  let embedCalls = 0;
  const fakeEmbed = (async (_url: string, init?: RequestInit) => {
    embedCalls++;
    const { input } = JSON.parse(String(init!.body));
    return new Response(JSON.stringify({ data: input.map((t: string) => ({ embedding: toVector(t) })) }));
  }) as typeof fetch;
  const memory = createMemory({ databaseUrl: testDb!, embedEndpoint: 'http://127.0.0.1:1234/v1', embedModel: 'test' }, fakeEmbed);
  afterAll(() => memory.close());

  const other: MemoryDoc = { id: 'rep_sig', inheritanceId: 'inh_sig', repository: 'shaunduncan/smokesignal', status: 'rejected', title: 'Missing self when calling bound method', body: 'review: reject (would drop self)' };

  it('creates the schema with a DiskANN index, indexes repairs and skips unchanged ones', async () => {
    await memory.migrate();
    await memory.index([...repairDocuments(detail), other]);
    const calls = embedCalls;
    expect(await memory.index([...repairDocuments(detail), other])).toBe(0);
    expect(embedCalls).toBe(calls);
  });

  it('finds the precedent by keyword and meaning, and can exclude the asking inheritance', async () => {
    const hits = await memory.search('cannot import Iterable from collections', 3);
    expect(hits[0].repository).toBe('kachayev/fn.py');
    expect(hits[0].matched_by).toContain('keyword');
    const elsewhere = await memory.search('bound method self', 3, 'inh_fn');
    expect(elsewhere.map((h) => h.id)).toEqual(['rep_sig']);
  });
});
