// The controller's only privileged boundary. No shell or Docker CLI is involved:
// each operation addresses one labeled sandbox container through Docker's socket.
import http from 'node:http';

const LABEL = 'sh.hoi.sandbox.member';
const MEMBER = /^[a-z0-9][a-z0-9-]{0,47}$/;
const MEMBER_MEMORY = 512 * 1024 * 1024;
const MEMBER_NANO_CPUS = 2_000_000_000;
const VERBS = new Set(['init', 'pair', 'create', 'join', 'invite', 'whoami', 'sync', 'feed', 'context', 'thread', 'ledger', 'post', 'post-link', 'comment', 'vote', 'vouch', 'review', 'standing', 'community', 'propose-limits', 'propose-rules', 'forge', 'link-approve']);

export class DockerError extends Error {
  constructor(message, status = 0) { super(message); this.name = 'DockerError'; this.status = status; }
}

// Non-TTY exec/log responses carry Docker's 8-byte stdout/stderr frame headers.
export function decodeFrames(buffer) {
  const stdout = [], stderr = [];
  let offset = 0;
  while (offset + 8 <= buffer.length && [0, 1, 2].includes(buffer[offset]) && buffer[offset + 1] === 0 && buffer[offset + 2] === 0 && buffer[offset + 3] === 0) {
    const size = buffer.readUInt32BE(offset + 4);
    if (offset + 8 + size > buffer.length) throw new DockerError('Docker returned a truncated output frame.');
    (buffer[offset] === 2 ? stderr : stdout).push(buffer.subarray(offset + 8, offset + 8 + size));
    offset += 8 + size;
  }
  if (offset === 0) return { stdout: buffer.toString('utf8'), stderr: '' };
  if (offset !== buffer.length) throw new DockerError('Docker returned an incomplete output frame.');
  return { stdout: Buffer.concat(stdout).toString('utf8'), stderr: Buffer.concat(stderr).toString('utf8') };
}

function aborted() { return new DOMException('Agent action stopped.', 'AbortError'); }
function checkSignal(signal) { if (signal?.aborted) throw signal.reason instanceof Error ? signal.reason : aborted(); }

export class Docker {
  constructor({ socketPath = process.env.DOCKER_SOCKET || '/var/run/docker.sock', project = process.env.COMPOSE_PROJECT_NAME || 'hoi-sandbox', network = process.env.HOI_AGENT_NETWORK || `${project}-agents`, image = process.env.HOI_AGENT_IMAGE || 'hoi-sandbox:latest', transport } = {}) {
    if (!MEMBER.test(project)) throw new Error('Invalid sandbox project name.');
    this.socketPath = socketPath;
    this.project = project;
    this.network = network;
    this.image = image;
    this.transport = transport;
    this.locks = new Map();
    this.creating = new Map();
  }

  request(method, path, body, { raw = false, timeout = 240_000 } = {}) {
    if (this.transport) return this.transport(method, path, body, { raw, timeout });
    return new Promise((resolve, reject) => {
      const payload = body === undefined ? undefined : Buffer.from(JSON.stringify(body));
      const req = http.request({ socketPath: this.socketPath, path, method,
        headers: payload ? { 'Content-Type': 'application/json', 'Content-Length': payload.length } : {} }, (res) => {
        const chunks = [];
        let size = 0;
        res.on('data', (chunk) => {
          size += chunk.length;
          if (size > 8 * 1024 * 1024) { req.destroy(new DockerError('Docker response exceeded 8 MiB.')); return; }
          chunks.push(chunk);
        });
        res.on('error', reject);
        res.on('end', () => {
          const bytes = Buffer.concat(chunks);
          if (res.statusCode >= 400) {
            let message = bytes.toString('utf8');
            try { message = JSON.parse(message).message || message; } catch { /* plain daemon error */ }
            reject(new DockerError(message.slice(0, 1500), res.statusCode));
            return;
          }
          if (raw) { resolve(bytes); return; }
          try { resolve(bytes.length ? JSON.parse(bytes.toString('utf8')) : null); }
          catch { reject(new DockerError('Docker returned invalid JSON.')); }
        });
      });
      req.setTimeout(timeout, () => req.destroy(new DockerError('Docker operation timed out.')));
      req.on('error', reject);
      req.end(payload);
    });
  }

  name(id) {
    if (!MEMBER.test(id)) throw new Error('Member IDs use lowercase letters, digits and hyphens (maximum 48 characters).');
    return `${this.project}-agent-${id}`;
  }

  labels(id) {
    return { 'com.docker.compose.project': this.project, 'com.docker.compose.service': `agent-${id}`, 'com.docker.compose.oneoff': 'False', [LABEL]: id };
  }

  async inspectMember(id) {
    const info = await this.request('GET', `/containers/${encodeURIComponent(this.name(id))}/json`);
    if (info.Config?.Labels?.['com.docker.compose.project'] !== this.project || info.Config?.Labels?.[LABEL] !== id) {
      throw new Error('Refusing to control a container outside this sandbox.');
    }
    return info;
  }

  async ensureMember(id) {
    this.name(id);
    if (this.creating.has(id)) return this.creating.get(id);
    const pending = this.createMember(id);
    this.creating.set(id, pending);
    try { return await pending; } finally { this.creating.delete(id); }
  }

  async createMember(id) {
    let info;
    try { info = await this.inspectMember(id); }
    catch (error) { if (error.status !== 404) throw error; }
    const name = this.name(id);
    if (info?.Image) {
      // A rebuilt tag does not update an existing container. Check its immutable
      // image ID before reuse so the next action runs the code just built. Do not
      // cache the tag: Compose may keep an unchanged controller image running.
      const current = await this.request('GET', `/images/${encodeURIComponent(this.image)}/json`);
      if (info.Image !== current.Id) {
        await this.removeMember(id);
        info = undefined;
      }
    }
    if (!info) {
      const network = await this.request('GET', `/networks/${encodeURIComponent(this.network)}`);
      if (!network.Internal) throw new Error('Agent network must be internal; refusing an internet-connected agent.');
      const volume = `${name}-data`;
      await this.request('POST', '/volumes/create', { Name: volume, Labels: this.labels(id) });
      await this.request('POST', `/containers/create?name=${encodeURIComponent(name)}`, {
        Image: this.image, Cmd: ['sleep', 'infinity'], Labels: this.labels(id),
        // Identity mining sizes its worker pool from the cgroup CPU quota. Without
        // a quota, hosts with many cores can exceed the bounded memory budget.
        HostConfig: { NetworkMode: this.network, Binds: [`${volume}:/data`], CapDrop: ['ALL'], SecurityOpt: ['no-new-privileges:true'], ReadonlyRootfs: true, Tmpfs: { '/tmp': 'rw,noexec,nosuid,size=16m' }, Memory: MEMBER_MEMORY, NanoCpus: MEMBER_NANO_CPUS, PidsLimit: 256, RestartPolicy: { Name: 'unless-stopped' } },
        NetworkingConfig: { EndpointsConfig: { [this.network]: {} } },
      });
    } else if (info.HostConfig && info.HostConfig.NanoCpus !== MEMBER_NANO_CPUS) {
      // Adopt the quota on already-created agents without replacing their identity.
      await this.request('POST', `/containers/${encodeURIComponent(name)}/update`, { NanoCpus: MEMBER_NANO_CPUS, Memory: MEMBER_MEMORY });
    }
    if (!info?.State?.Running) await this.request('POST', `/containers/${encodeURIComponent(name)}/start`);
    return { id, name };
  }

  // SQLite members may only perform one protocol action at a time.
  async serial(id, fn) {
    const previous = this.locks.get(id) || Promise.resolve();
    let release;
    const next = new Promise((resolve) => { release = resolve; });
    this.locks.set(id, next);
    await previous;
    try { return await fn(); } finally { release(); if (this.locks.get(id) === next) this.locks.delete(id); }
  }

  async exec(id, command, { signal, scorecard = false, forgery = false } = {}) {
    return this.serial(id, async () => {
      checkSignal(signal);
      await this.ensureMember(id);
      checkSignal(signal);
      const created = await this.request('POST', `/containers/${encodeURIComponent(this.name(id))}/exec`, { AttachStdout: true, AttachStderr: true, Tty: false, Cmd: command });
      checkSignal(signal);
      let stop;
      let stopError;
      let completed = false;
      let rejectAbort;
      const interrupted = new Promise((_, reject) => { rejectAbort = reject; });
      const onAbort = () => {
        // Killing an HTTP request does not kill Docker exec. Stop the actual member,
        // then permit another action to reopen it with the same persisted identity.
        stop ??= this.stopMember(id).catch(error => {
          stopError = new Error(`Could not stop the agent container: ${error.message}`);
        }).finally(() => rejectAbort(stopError || aborted()));
      };
      signal?.addEventListener('abort', onAbort, { once: true });
      let timer;
      try {
        if (signal?.aborted) onAbort();
        timer = setTimeout(onAbort, 235_000);
        const bytes = await Promise.race([this.request('POST', `/exec/${created.Id}/start`, { Detach: false, Tty: false }, { raw: true }), interrupted]);
        if (stop) { await stop; throw stopError || aborted(); }
        checkSignal(signal);
        const result = await this.request('GET', `/exec/${created.Id}/json`);
        completed = !result.Running;
        if (result.ExitCode === 137) {
          const member = await this.inspectMember(id).catch(() => null);
          throw new Error(member?.State?.OOMKilled
            ? 'Agent exceeded its 512 MiB memory budget while running the action. Its identity is preserved; retry after checking Docker resources.'
            : 'Agent was killed (exit 137). Check its 512 MiB memory budget and Docker status before retrying.');
        }
        const output = decodeFrames(bytes);
        let parsed;
        try { parsed = JSON.parse(output.stdout.trim()); }
        catch { throw new Error(`Agent returned no usable result: ${(output.stderr || output.stdout || `exit ${result.ExitCode}`).slice(0, 1000)}`); }
        if (parsed?.ok === false) throw new Error(parsed.error || 'Agent action failed.');
        // battle intentionally exits 1 when its evidence says an invariant broke.
        if (result.ExitCode !== 0 && !(scorecard && Array.isArray(parsed.checks)) && !(forgery && parsed.ok === true && typeof parsed.all_held_locally === 'boolean')) {
          throw new Error(`Agent exited ${result.ExitCode}: ${(output.stderr || output.stdout).slice(0, 1000)}`);
        }
        return parsed;
      } catch (error) {
        if (!completed && !stop) {
          await this.stopMember(id).catch(stopFailure => {
            throw new Error(`${error.message} The agent container could not be stopped: ${stopFailure.message}`);
          });
        }
        throw error;
      } finally {
        clearTimeout(timer);
        signal?.removeEventListener('abort', onAbort);
        if (stop) await stop;
      }
    });
  }

  action(id, args, options = {}) {
    if (!Array.isArray(args) || !VERBS.has(args[0]) || args.length > 16 || args.some((v) => typeof v !== 'string' || v.length > 16_384 || v.includes('\0'))) {
      throw new Error('Invalid agent action.');
    }
    return this.exec(id, ['hoi-sim', 'agent', '--data', '/data', '--hub', 'ws://hub:7842', ...args], { ...options, forgery: args[0] === 'forge' });
  }

  referee(id = 'referee', options = {}) {
    return this.exec(id, ['hoi-sim', 'referee', '--data', '/data', '--hub', 'ws://hub:7842', '--json'], options);
  }

  battle({ honest = 4, attackers = 3 } = {}, options = {}) {
    for (const value of [honest, attackers]) if (!Number.isInteger(value) || value < 1 || value > 12) throw new Error('Battle actors must be from 1 to 12 per team.');
    return this.exec('battle', ['hoi-sim', 'battle', '--honest', String(honest), '--attackers', String(attackers), '--pow-bits', '8', '--json'], { ...options, scorecard: true });
  }

  async stopMember(id) {
    let info;
    try { info = await this.inspectMember(id); } catch (error) { if (error.status === 404) return; throw error; }
    if (info.State?.Running) await this.request('POST', `/containers/${encodeURIComponent(this.name(id))}/stop?t=1`);
  }

  async removeMember(id) {
    try { await this.inspectMember(id); } catch (error) { if (error.status === 404) return; throw error; }
    await this.stopMember(id);
    // Never remove its volume: identities survive stop/restart and agent reconfiguration.
    await this.request('DELETE', `/containers/${encodeURIComponent(this.name(id))}`);
  }

  async list() {
    const filters = encodeURIComponent(JSON.stringify({ label: [`com.docker.compose.project=${this.project}`] }));
    const rows = await this.request('GET', `/containers/json?all=true&filters=${filters}`);
    return rows.map((c) => ({ id: c.Labels?.[LABEL] || null, name: c.Names?.[0]?.replace(/^\//, '') || c.Id.slice(0, 12), service: c.Labels?.['com.docker.compose.service'] || '', state: c.State, status: c.Status }));
  }

  async health() {
    const containers = await this.list();
    const hub = containers.find((c) => c.service === 'hub');
    return { ok: hub?.state === 'running', hub: hub?.state || 'missing', containers };
  }

  async hubPairing() {
    const name = `${this.project}-hub`;
    const info = await this.request('GET', `/containers/${encodeURIComponent(name)}/json`);
    if (info.Config?.Labels?.['com.docker.compose.project'] !== this.project || info.Config?.Labels?.['com.docker.compose.service'] !== 'hub') {
      throw new Error('The hub does not belong to this sandbox.');
    }
    const bytes = await this.request('GET', `/containers/${encodeURIComponent(name)}/logs?stdout=true&stderr=true&tail=200`, undefined, { raw: true });
    const logs = decodeFrames(bytes);
    const token = `${logs.stdout}\n${logs.stderr}`.match(/[?&]t=([^&\s\x1b]+)/)?.[1];
    if (!token) throw new Error('The hub is still starting; its pairing token is not ready.');
    return { internal: `hoi://pair?u=ws://hub:7842&t=${token}&n=hub`, host: `hoi://pair?u=ws://127.0.0.1:7842&t=${token}&n=hub` };
  }
}
