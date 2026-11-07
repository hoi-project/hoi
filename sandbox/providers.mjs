import { randomUUID } from 'node:crypto';

export const CLI_DRIVERS = ['codex', 'claude'];
export const MAX_DECISION_BYTES = 16384;
const OFFLINE_MS = 15000;
const abortError = () => Object.assign(new Error('Model decision cancelled.'), { name: 'AbortError' });

export function decisionContent(value) {
  if (typeof value !== 'string' || Buffer.byteLength(value) > MAX_DECISION_BYTES) throw new Error('The CLI returned no bounded JSON decision.');
  const content = value.trim().replace(/^```(?:json)?\s*/, '').replace(/\s*```$/, '');
  let parsed;
  try { parsed = JSON.parse(content); } catch { throw new Error('The CLI did not return a JSON action.'); }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed) || typeof parsed.action !== 'string') throw new Error('The CLI returned no JSON action object.');
  return content;
}

// The controller never executes a command supplied by an actor. A local host
// worker polls this small mailbox, runs a fixed CLI invocation and returns text.
export class ProviderBroker {
  constructor({ now = () => Date.now(), timeoutMs = 120000 } = {}) {
    this.now = now;
    this.timeoutMs = timeoutMs;
    this.jobs = new Map();
    this.workers = new Map();
  }
  status() {
    const workers = [...this.workers.values()].filter(worker => this.now() - worker.lastSeen < OFFLINE_MS);
    const providers = Object.fromEntries(CLI_DRIVERS.map(driver => {
      const available = workers.find(worker => worker.providers[driver]?.available);
      const report = available?.providers[driver] || workers.find(worker => worker.providers[driver])?.providers[driver];
      return [driver, report || { available: false, error: 'Local CLI worker is offline. Run pnpm sandbox again.' }];
    }));
    return { online: workers.length > 0, providers, pending: [...this.jobs.values()].filter(job => !job.workerId).length, running: [...this.jobs.values()].filter(job => job.workerId).length, lastSeen: workers.length ? Math.max(...workers.map(worker => worker.lastSeen)) : null };
  }
  request({ driver, model = '', messages }, signal) {
    if (!CLI_DRIVERS.includes(driver)) return Promise.reject(new Error('Unknown CLI provider.'));
    if (signal?.aborted) return Promise.reject(abortError());
    const status = this.status();
    if (!status.providers[driver].available) return Promise.reject(new Error(status.providers[driver].error || `${driver} is unavailable. Install it and sign in, then restart pnpm sandbox.`));
    if (typeof model !== 'string' || model.length > 100 || /[\x00-\x1f]/.test(model)) return Promise.reject(new Error('Invalid CLI model name.'));
    if (!Array.isArray(messages) || messages.length < 1 || messages.length > 8 || messages.some(message => !['system', 'user', 'assistant'].includes(message.role) || typeof message.content !== 'string') || Buffer.byteLength(JSON.stringify(messages)) > 100000) return Promise.reject(new Error('Model context exceeded the sandbox limit.'));
    if (this.jobs.size >= 24) return Promise.reject(new Error('The model queue is full. Reduce active agents or increase the turn interval.'));
    return new Promise((resolve, reject) => {
      const id = randomUUID();
      const settle = (error, content) => {
        if (!this.jobs.has(id)) return;
        this.jobs.delete(id);
        clearTimeout(timer);
        signal?.removeEventListener('abort', abort);
        error ? reject(error) : resolve(content);
      };
      const abort = () => settle(abortError());
      const timer = setTimeout(() => settle(new Error(`${driver} decision timed out after ${Math.round(this.timeoutMs / 1000)} seconds.`)), this.timeoutMs);
      this.jobs.set(id, { id, driver, model, messages: structuredClone(messages), settle, createdAt: this.now(), workerId: null });
      signal?.addEventListener('abort', abort, { once: true });
    });
  }
  take({ workerId, providers = {}, capacity = 0 } = {}) {
    if (typeof workerId !== 'string' || !/^[a-f0-9-]{36}$/.test(workerId)) throw new Error('Invalid worker identity.');
    if (!Number.isInteger(capacity) || capacity < 0 || capacity > 2) throw new Error('Invalid worker capacity.');
    const reports = {};
    for (const driver of CLI_DRIVERS) {
      const report = providers[driver];
      reports[driver] = { available: report?.available === true, ...(typeof report?.version === 'string' ? { version: report.version.slice(0, 100) } : {}), ...(typeof report?.error === 'string' ? { error: report.error.slice(0, 300) } : {}) };
    }
    for (const [id, worker] of this.workers) if (this.now() - worker.lastSeen >= OFFLINE_MS) {
      this.workers.delete(id);
      for (const job of this.jobs.values()) if (job.workerId === id) job.settle(new Error('The local CLI worker disconnected. No fallback action was posted.'));
    }
    this.workers.set(workerId, { lastSeen: this.now(), providers: reports });
    const running = [...this.jobs.values()].filter(job => job.workerId);
    const job = capacity > 0 && running.length < 2 ? [...this.jobs.values()].find(candidate => !candidate.workerId && reports[candidate.driver]?.available) : undefined;
    if (job) job.workerId = workerId;
    return { job: job ? { id: job.id, driver: job.driver, model: job.model, messages: job.messages } : null, activeIds: [...this.jobs.values()].filter(candidate => candidate.workerId === workerId).map(candidate => candidate.id) };
  }
  complete({ workerId, id, content, error } = {}) {
    const job = this.jobs.get(id);
    if (!job) return { ok: true, discarded: true };
    if (job.workerId !== workerId) throw new Error('This decision belongs to a different worker.');
    if (typeof error === 'string' && error) job.settle(new Error(error.slice(0, 500)));
    else {
      try { job.settle(null, decisionContent(content)); }
      catch (failure) { job.settle(failure); }
    }
    return { ok: true };
  }
  close() {
    for (const job of this.jobs.values()) job.settle(abortError());
    this.workers.clear();
  }
}
