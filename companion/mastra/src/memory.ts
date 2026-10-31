// Restoration memory: every repair attempt the engine has recorded, across all inheritances, stored in Postgres with
// pgvector embeddings (Tiger Data's timescaledb-ha image, with the pgvectorscale DiskANN index) and searched with hybrid
// keyword + vector ranking. When the Lead is asked about a failure, it can say "this was fixed in project X by …, and
// this other attempt was reverted because …", with the repair ids to check.
//
// Local-first: the database and the embedding model must be loopback, because rationales and error text can quote the
// inherited source. Embeddings come from the local LM Studio server (nomic-embed-text), never a hosted API.
import pg from 'pg';
import { isLoopback } from './config.ts';
import { truncate } from './untrusted.ts';

export const EMBED_DIM = 768;

export interface MemoryConfig {
  databaseUrl: string;
  embedEndpoint: string;
  embedModel: string;
}

export interface MemoryDoc {
  id: string;
  inheritanceId: string;
  repository: string;
  status: string;
  title: string;
  body: string;
}

export interface MemoryHit {
  id: string;
  inheritance_id: string;
  repository: string;
  status: string;
  title: string;
  excerpt: string;
  score: number;
  matched_by: ('keyword' | 'vector')[];
}

export function memoryConfig(env: NodeJS.ProcessEnv = process.env): MemoryConfig | null {
  const databaseUrl = env.IE_MEMORY_DATABASE_URL?.trim();
  if (!databaseUrl) return null;
  const embedEndpoint = (env.IE_EMBED_ENDPOINT ?? env.IE_MODEL_ENDPOINT ?? 'http://127.0.0.1:1234/v1').replace(/\/$/, '');
  const dbHost = new URL(databaseUrl.replace(/^postgres(ql)?:/, 'http:')).hostname;
  if (!isLoopback(`http://${dbHost.includes(':') ? `[${dbHost}]` : dbHost}`)) {
    throw new Error(`IE_MEMORY_DATABASE_URL must point at a loopback database (repair text can quote inherited source), got host ${dbHost}`);
  }
  if (!isLoopback(embedEndpoint)) throw new Error(`IE_EMBED_ENDPOINT must be loopback, got ${embedEndpoint}`);
  return { databaseUrl, embedEndpoint, embedModel: env.IE_EMBED_MODEL ?? 'text-embedding-nomic-embed-text-v1.5' };
}

/** One searchable document per recorded repair attempt. Replays are left out: they repeat a recorded run. Pure. */
export function repairDocuments(detail: any): MemoryDoc[] {
  if (detail?.replay || String(detail?.successor_name ?? '').includes('(replay)')) return [];
  const issues = new Map<string, any>((detail?.issues ?? []).map((i: any) => [i.id, i]));
  return (detail?.repairs ?? []).map((r: any) => {
    const issue = issues.get(r.issue_id) ?? {};
    const review = r.review ? `review: ${r.review.verdict}${r.review.concerns?.length ? ` (${r.review.concerns.slice(0, 2).join('; ')})` : ''}` : '';
    const probe = r.probe ? `behaviour probe: ${r.probe.verdict} (${r.probe.note ?? ''})` : '';
    const body = [
      `repository: ${detail.full_name}`,
      `outcome: ${r.status}${r.commit ? ` as commit ${String(r.commit).slice(0, 10)}` : ''}`,
      // The engine's own reason comes first: it is the authoritative account of why the attempt ended as it did.
      r.note ? `why (engine): ${truncate(r.note, 400)}` : '',
      issue.diagnosis ? `diagnosis: ${truncate(issue.diagnosis, 500)}` : '',
      r.rationale ? `fix rationale: ${truncate(r.rationale, 500)}` : '',
      r.files?.length ? `files: ${r.files.join(', ')}` : '',
      review,
      probe,
    ].filter(Boolean).join('\n');
    return { id: r.id, inheritanceId: detail.id, repository: detail.full_name, status: r.status, title: issue.title ?? r.issue_id, body };
  });
}

/** Reciprocal-rank fusion of the keyword and vector rankings. Pure, so the ranking rule is unit-tested. */
export function fuse(keyword: string[], vector: string[], k = 60): { id: string; score: number; matched_by: ('keyword' | 'vector')[] }[] {
  const out = new Map<string, { id: string; score: number; matched_by: ('keyword' | 'vector')[] }>();
  const add = (ids: string[], by: 'keyword' | 'vector') =>
    ids.forEach((id, i) => {
      const e = out.get(id) ?? { id, score: 0, matched_by: [] };
      e.score += 1 / (k + i + 1);
      e.matched_by.push(by);
      out.set(id, e);
    });
  add(keyword, 'keyword');
  add(vector, 'vector');
  return [...out.values()].sort((a, b) => b.score - a.score);
}

const SCHEMA = `
CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS vectorscale;
CREATE TABLE IF NOT EXISTS repair_memory (
  id text PRIMARY KEY,
  inheritance_id text NOT NULL,
  repository text NOT NULL,
  status text NOT NULL,
  title text NOT NULL,
  body text NOT NULL,
  embedding vector(${EMBED_DIM}) NOT NULL,
  tsv tsvector GENERATED ALWAYS AS (to_tsvector('english', title || ' ' || body)) STORED,
  indexed_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS repair_memory_tsv ON repair_memory USING gin (tsv);
CREATE INDEX IF NOT EXISTS repair_memory_embedding ON repair_memory USING diskann (embedding vector_cosine_ops);
`;

export function createMemory(cfg: MemoryConfig, fetchImpl: typeof fetch = fetch) {
  const pool = new pg.Pool({ connectionString: cfg.databaseUrl, max: 4 });

  // nomic-embed-text expects task prefixes on documents and queries.
  async function embed(texts: string[], kind: 'document' | 'query'): Promise<number[][]> {
    const r = await fetchImpl(`${cfg.embedEndpoint}/embeddings`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ model: cfg.embedModel, input: texts.map((t) => `search_${kind}: ${t}`) }),
    });
    if (!r.ok) throw new Error(`embeddings -> ${r.status}`);
    const vectors = ((await r.json()) as any).data.map((d: any) => d.embedding as number[]);
    if (vectors.some((v: number[]) => v.length !== EMBED_DIM)) throw new Error(`expected ${EMBED_DIM}-dimensional embeddings`);
    return vectors;
  }
  const literal = (v: number[]) => `[${v.join(',')}]`;

  return {
    async migrate() {
      await pool.query(SCHEMA);
    },

    /** Upsert documents; only those whose text changed are re-embedded. Returns how many were (re)embedded. */
    async index(docs: MemoryDoc[]): Promise<number> {
      if (!docs.length) return 0;
      const existing = await pool.query('SELECT id, title, body, status FROM repair_memory WHERE id = ANY($1)', [docs.map((d) => d.id)]);
      const same = new Set(existing.rows.filter((r) => docs.some((d) => d.id === r.id && d.title === r.title && d.body === r.body && d.status === r.status)).map((r) => r.id));
      const changed = docs.filter((d) => !same.has(d.id));
      for (let i = 0; i < changed.length; i += 16) {
        const batch = changed.slice(i, i + 16);
        const vectors = await embed(batch.map((d) => `${d.title}\n${d.body}`), 'document');
        for (const [j, d] of batch.entries()) {
          await pool.query(
            `INSERT INTO repair_memory (id, inheritance_id, repository, status, title, body, embedding)
             VALUES ($1,$2,$3,$4,$5,$6,$7::vector)
             ON CONFLICT (id) DO UPDATE SET status = EXCLUDED.status, title = EXCLUDED.title, body = EXCLUDED.body,
               embedding = EXCLUDED.embedding, indexed_at = now()`,
            [d.id, d.inheritanceId, d.repository, d.status, d.title, d.body, literal(vectors[j])],
          );
        }
      }
      return changed.length;
    },

    /** Hybrid search: full-text rank and cosine distance, fused. `exclude` leaves out one inheritance (the one asking). */
    async search(query: string, limit = 5, exclude?: string): Promise<MemoryHit[]> {
      const [qv] = await embed([query], 'query');
      const where = exclude ? 'WHERE inheritance_id <> $3' : '';
      const params = (extra: unknown[]) => (exclude ? [...extra, exclude] : extra);
      const kw = await pool.query(
        `SELECT id FROM repair_memory ${where ? `${where} AND` : 'WHERE'} tsv @@ websearch_to_tsquery('english', $1)
         ORDER BY ts_rank_cd(tsv, websearch_to_tsquery('english', $1)) DESC LIMIT $2`,
        params([query, limit * 4]),
      );
      const vec = await pool.query(`SELECT id FROM repair_memory ${where} ORDER BY embedding <=> $1::vector LIMIT $2`, params([literal(qv), limit * 4]));
      const ranked = fuse(kw.rows.map((r) => r.id), vec.rows.map((r) => r.id)).slice(0, limit);
      if (!ranked.length) return [];
      const rows = await pool.query('SELECT id, inheritance_id, repository, status, title, body FROM repair_memory WHERE id = ANY($1)', [ranked.map((r) => r.id)]);
      const byId = new Map(rows.rows.map((r) => [r.id, r]));
      return ranked.map((r) => {
        const row: any = byId.get(r.id);
        return { id: r.id, inheritance_id: row.inheritance_id, repository: row.repository, status: row.status, title: row.title, excerpt: truncate(row.body, 600), score: Number(r.score.toFixed(5)), matched_by: r.matched_by };
      });
    },

    async count(): Promise<number> {
      return Number((await pool.query('SELECT count(*) FROM repair_memory')).rows[0].count);
    },

    close: () => pool.end(),
  };
}

export type Memory = ReturnType<typeof createMemory>;
