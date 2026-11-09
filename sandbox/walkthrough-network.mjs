// Sandbox-only host: production browser clients, public nodes, registries and opaque relays.
// Observation reads service stores; it never exports client keys or changes protocol behavior.
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { createSocket } from 'node:dgram';
import { once } from 'node:events';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, readdir, unlink, rmdir } from 'node:fs/promises';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createLocalRegistry, createLocalRelay, inspectPublicRecord } from './walkthrough-services.mjs';
import { cipherArtifact, describeJourney, newJourney } from './walkthrough-journey.mjs';
import { describeFlow } from './walkthrough-flow.mjs';
import { describeConnections } from './walkthrough-connections.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const PEOPLE = [{ id: 'alice', name: 'Alice' }, { id: 'bob', name: 'Bob' }];
const LIMITS = { powBits: 12, newcomerPostsPerDay: 100, ladder: { identityBits: 8, admitVouches: 1, establishedAge: 60 } };
const LIMITATIONS = [
  'This is the current protocol: full public carriers can read public posts; private relays store encrypted objects.',
  'Service observations prove storage and receipt, not the exact packet route or anonymity. Relays still observe timing and network addresses.',
  'Two local public carriers do not demonstrate scattered delivery across independent holders.',
  'Reset discards this session’s browser identities and service stores and seeds a fresh local network.',
];

export function validatePublication(value) {
  if (!value || !PEOPLE.some(person => person.id === value.author) || !['public', 'private'].includes(value.visibility)
    || typeof value.title !== 'string' || !value.title.trim() || value.title.length > 120
    || typeof value.text !== 'string' || !value.text.trim() || value.text.length > 4000) {
    throw new Error('Choose Alice or Bob, a community, a title of 1–120 characters and text of 1–4000 characters.');
  }
  return { author: value.author, visibility: value.visibility, title: value.title.trim(), text: value.text.trim() };
}

export function allowedBrowserRequest(value, origins) {
  try {
    const url = new URL(value);
    return ['http:', 'https:'].includes(url.protocol) && origins.has(url.origin)
      && ['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname) && !url.username && !url.password;
  } catch { return false; }
}

export function relayCanary(text) {
  // Short strings occur in random ciphertext by chance; never present those matches as leaks.
  return typeof text === 'string' && Buffer.byteLength(text) >= 16 ? { plaintextCanary: text } : {};
}

async function listen(server) {
  await new Promise((done, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', done); });
  return `http://127.0.0.1:${server.address().port}`;
}

async function freePort(udp = false) {
  if (udp) {
    const socket = createSocket('udp4');
    await new Promise((done, reject) => { socket.once('error', reject); socket.bind(0, '127.0.0.1', done); });
    const port = socket.address().port;
    await new Promise(done => socket.close(done));
    return port;
  }
  const server = createServer();
  await listen(server);
  const port = server.address().port;
  await new Promise(done => server.close(done));
  return port;
}

async function until(check, message, timeout = 60_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await check()) return;
    await new Promise(done => setTimeout(done, 200));
  }
  throw new Error(message);
}

export function createWalkthroughNetwork({ stateDir = join(ROOT, 'sandbox/.state/walkthrough'), nodeCommand,
  executablePath = process.env.CHROMIUM_PATH, browserChannel, onProgress = () => {} } = {}) {
  let state = { status: 'stopped', phase: 'Not started', session: '', participants: [], communities: {}, services: [], trace: null, limitations: LIMITATIONS };
  let browser, vite, runDir;
  let journeyData = null, journeyRunning = false, journeyNumber = 0;
  let invitationProgress = { requestCreated: false, inviteCreated: false, joined: false };
  let services = [], nodes = [], contexts = [], pages = new Map();
  const phase = value => { state.phase = value; onProgress(value); };
  const call = (page, method, ...args) => page.evaluate(async ({ method, args }) => {
    const { app } = await import('/src/lib/app.svelte.ts');
    return app.core[method](...args);
  }, { method, args });

  async function node(id, registries, entryNode) {
    const directory = join(runDir, id);
    await mkdir(directory, { recursive: true });
    const rtcPort = await freePort(true);
    const command = nodeCommand || join(ROOT, 'core/target/debug', process.platform === 'win32' ? 'hoi-node.exe' : 'hoi-node');
    const child = spawn(command, ['--data', directory, '--public', '--name', id, '--peer-bind', '127.0.0.1:0',
      '--rtc-port', String(rtcPort), '--rtc-bind', '127.0.0.1', '--rtc-advertise', `127.0.0.1:${rtcPort}`, '--no-port-mapping', '--no-lan', '--no-dht',
      '--hold-pieces', '0', '--carry-limit', '8', ...(entryNode ? ['--node', entryNode.publicLink] : []), ...registries.flatMap(registry => ['--index', registry.url])],
    { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    const result = { id, label: id === 'node-a' ? 'Public node A' : 'Public node B', child, directory, output: '', exited: false, error: null };
    child.once('error', error => { result.error = error; });
    child.once('exit', () => { result.exited = true; });
    const read = bytes => { result.output = (result.output + bytes).slice(-24_000); };
    child.stdout.on('data', read); child.stderr.on('data', read);
    nodes.push(result);
    await until(() => {
      if (result.error) throw new Error(`Could not start ${result.label}. Check the hoi-node executable.`);
      if (result.exited) throw new Error(`${result.label} stopped during startup.`);
      return result.output.includes(`@127.0.0.1:${rtcPort}`);
    }, `${result.label} did not advertise its local WebRTC route.`);
    result.route = [...result.output.matchAll(/rtc:[A-Za-z0-9_-]{43}@127\.0\.0\.1:\d+/g)].map(match => match[0]).find(route => route.endsWith(`:${rtcPort}`));
    await until(async () => {
      try { result.publicLink = (await readFile(join(directory, 'node-link'), 'utf8')).trim(); }
      catch { return false; }
      return result.publicLink.length <= 8192 && result.publicLink.includes('#/node?');
    }, 'The local node did not write its public entry link.');
    return result;
  }

  async function person(info, origin, origins) {
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: 'reduce', serviceWorkers: 'block' });
    contexts.push(context);
    await context.route('**/*', route => allowedBrowserRequest(route.request().url(), origins) ? route.continue() : route.abort('blockedbyclient'));
    // Discovery can suggest more nodes. Enforce this demonstration's single client route at
    // the actual WebRTC boundary, so an automatic second link cannot bypass the native hop.
    const allowedNode = nodes[info.id === 'alice' ? 0 : 1];
    const [fingerprint, address] = allowedNode.route.slice(4).split('@');
    const expectedFingerprint = Buffer.from(fingerprint, 'base64url').toString('hex').match(/../g).join(':').toUpperCase();
    await context.addInitScript(({ nodeId, port, fingerprint }) => {
      const Original = window.RTCPeerConnection;
      const connections = new Map();
      let nextConnection = 1, blocked = 0;
      window.__hoiWalkthroughRoutes = () => ({ allowedNode: nodeId, blocked, connections: [...connections.values()].slice(-8) });
      window.__hoiWalkthroughJson = async (response, limit) => {
        if (!response.body || Number(response.headers.get('content-length')) > limit) throw new Error('The local observation response exceeds its limit.');
        const reader = response.body.getReader(), bytes = new Uint8Array(limit);
        let length = 0;
        try {
          for (;;) {
            const chunk = await reader.read();
            if (chunk.done) break;
            if (length + chunk.value.length > limit) { await reader.cancel(); throw new Error('The local observation response exceeds its limit.'); }
            bytes.set(chunk.value, length); length += chunk.value.length;
          }
        } finally { reader.releaseLock(); }
        return JSON.parse(new TextDecoder().decode(bytes.subarray(0, length)));
      };
      const loopback = address => address === '127.0.0.1' || address === '::1';
      const refuse = () => { blocked++; throw new Error('This demonstration allows only the selected public node for this browser.'); };
      const checkCandidate = candidate => {
        const parts = candidate.trim().split(/\s+/);
        if (parts.length >= 6 && (!loopback(parts[4]) || parts[5] !== port)) refuse();
      };
      const remote = Original.prototype.setRemoteDescription;
      Original.prototype.setRemoteDescription = function (description) {
        if (!String(description.sdp || '').split(/\r?\n/).includes(`a=fingerprint:sha-256 ${fingerprint}`)) refuse();
        for (const line of (description.sdp || '').split(/\r?\n/)) if (line.startsWith('a=candidate:')) checkCandidate(line.slice(2));
        const id = nextConnection++;
        connections.set(id, { node: nodeId, state: this.connectionState });
        this.addEventListener('connectionstatechange', () => {
          if (['closed', 'failed'].includes(this.connectionState)) connections.delete(id);
          else connections.set(id, { node: nodeId, state: this.connectionState });
        });
        return remote.call(this, description);
      };
      const add = Original.prototype.addIceCandidate;
      Original.prototype.addIceCandidate = function (candidate) {
        if (candidate?.candidate) checkCandidate(candidate.candidate);
        return add.call(this, candidate);
      };
      window.RTCPeerConnection = class extends Original {
        constructor(configuration = {}) {
          if (configuration.iceServers?.length) throw new Error('External ICE servers are disabled in this walkthrough.');
          super(configuration);
        }
      };
    }, { nodeId: allowedNode.id, port: address.split(':').at(-1), fingerprint: expectedFingerprint });
    if (context.routeWebSocket) await context.routeWebSocket('**/*', socket => socket.close());
    const { consentAccepted } = await import('../scripts/browser-vault.mjs');
    await consentAccepted(context);
    const page = await context.newPage();
    page.setDefaultTimeout(300_000);
    await page.goto(origin);
    await page.waitForFunction(async () => (await import('/src/lib/app.svelte.ts')).app.phase !== 'booting', null, { timeout: 60_000 });
    await page.evaluate(async name => {
      const { app } = await import('/src/lib/app.svelte.ts');
      const deadline = Date.now() + 60_000;
      while (app.phase === 'booting' && Date.now() < deadline) await new Promise(done => setTimeout(done, 50));
      if (app.phase === 'locked') await app.unlock('local walkthrough disposable browser passphrase');
      if (app.phase !== 'ready') throw new Error(`The production browser core did not become ready (${app.phase}): ${app.bootError}`);
      await app.identityChanged(await app.core.createIdentity(name));
      await app.setSettings({ motion: 'reduced', postingNoticeSeen: true, directLinks: false });
    }, info.name);
    pages.set(info.id, page);
    return page;
  }

  async function syncPrivate(page, community, selectedObjects = []) {
    const urls = services.filter(service => service.kind === 'relay').map(service => service.url);
    const result = await page.evaluate(async ({ community, urls, selectedObjects }) => {
      const { app } = await import('/src/lib/app.svelte.ts');
      const { syncPrivateGroup } = await import('/src/lib/private-relay.ts');
      const uploads = [];
      const selected = new Set(selectedObjects);
      const fetcher = async (url, init) => {
        const response = await fetch(url, init);
        const match = /\/v2\/rooms\/([a-f0-9]{64})\/objects\/([a-f0-9]{64})$/.exec(url);
        if (init?.method === 'PUT' && match && selected.has(match[2]) && uploads.length < 16) {
          const bytes = new Uint8Array(init.body);
          const sha256 = [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map(value => value.toString(16).padStart(2, '0')).join('');
          // Capture only fixed receipt fields. Authorization headers and room signatures stay out.
          let receipt;
          try { receipt = await window.__hoiWalkthroughJson(response.clone(), 4096); } catch { /* The production client rejects malformed receipts. */ }
          uploads.push({ endpoint: urls.indexOf(url.slice(0, url.indexOf('/v2/'))), method: 'PUT', status: response.status,
            objectId: match[2], room: match[1], size: bytes.length, sha256,
            receipt: receipt && { id: receipt.id, size: receipt.size, expires: receipt.expires } });
        }
        return response;
      };
      const status = await syncPrivateGroup(app.core, community, urls, fetcher);
      app.privateSync = { ...app.privateSync, [community]: status };
      return { status, uploads };
    }, { community, urls, selectedObjects });
    if (result.status.error || result.status.replicas !== urls.length) throw new Error(result.status.error || 'Both local relays must acknowledge the encrypted objects.');
    return result.uploads.map(item => ({ ...item, service: services.filter(service => service.kind === 'relay')[item.endpoint]?.id }));
  }

  async function refresh() {
    for (const page of pages.values()) await page.evaluate(async () => (await import('/src/lib/app.svelte.ts')).app.refresh());
    if (pages.size === 2) state.topology = await topology();
  }

  async function topology(requireCommunity = false) {
    const clients = [];
    for (const [index, info] of PEOPLE.entries()) {
      const page = pages.get(info.id);
      if (!page) continue;
      const evidence = await page.evaluate(async () => ({
        net: await (await import('/src/lib/app.svelte.ts')).app.core.net(), routes: window.__hoiWalkthroughRoutes(),
      }));
      const connected = evidence.net.connections.filter(connection => connection.connected);
      if (evidence.routes.allowedNode !== nodes[index].id || evidence.routes.connections.some(connection => connection.node !== nodes[index].id)
        || connected.some(connection => connection.transport !== 'webrtc' || connection.direct)) throw new Error('The browser no longer follows the selected node route.');
      const communityConnected = connected.some(connection => connection.keeps && connection.communities.includes(state.communities.public?.id));
      if (requireCommunity && (!communityConnected || !evidence.routes.connections.some(connection => connection.state === 'connected'))) throw new Error('The selected public node link is not connected.');
      clients.push({ id: info.id, node: nodes[index].id, communityConnected, blockedAlternatives: evidence.routes.blocked,
        connections: connected.slice(0, 4).map(connection => ({ transport: connection.transport, encryption: connection.encryption,
          connected: true, keeps: connection.keeps, community: connection.communities.includes(state.communities.public?.id) })) });
    }
    return { enforced: true, capturedAt: Date.now(), nativeEntry: { from: 'node-b', to: 'node-a', configured: Boolean(nodes[0]?.publicLink), source: 'Node B starts with node A’s public node-link through --node.' }, clients };
  }

  async function connectionEvidence() {
    const observations = [];
    for (const info of PEOPLE) {
      const view = await pages.get(info.id).evaluate(async communities => {
        const { app } = await import('/src/lib/app.svelte.ts');
        const status = await app.core.publicJoinStatus(communities.public.id);
        const group = await app.core.privateGroup(communities.private.id);
        // The raw group also includes member credentials and ready invitations. Keep it inside
        // this client; the room is used only for equality and is removed before exposing state.
        return { public: { joined: status.status === 'joined', communityId: status.id },
          private: { active: group.active, communityId: group.community, epoch: group.epoch, memberCount: group.members.length, room: group.room } };
      }, state.communities);
      observations.push({ id: info.id, ...view });
    }
    return describeConnections({ communities: state.communities, observations, nativeConfigured: Boolean(nodes[0]?.publicLink), invitation: invitationProgress });
  }

  async function discover(person, registry, selectedNode) {
    const result = await pages.get(person).evaluate(async ({ registry, community, route, name }) => {
      const { app } = await import('/src/lib/app.svelte.ts');
      const { registryClient } = await import('/src/lib/registry.ts');
      let status;
      const client = registryClient(registry, async (url, init) => {
        const response = await fetch(url, init);
        if (new URL(url).pathname === '/v2/nodes') status = response.status;
        return response;
      });
      const routes = (await client.nodesFor(community)).filter(route => /^rtc:[A-Za-z0-9_-]{43}@127\.0\.0\.1:\d+$/.test(route));
      if (!routes.includes(route)) throw new Error('The selected node was not returned by the local registry.');
      await app.core.publicJoin(community, [route], name);
      return { routes, status };
    }, { registry: registry.url, community: state.communities.public.id, route: selectedNode.route, name: state.communities.public.name });
    return { id: `${person}-registry`, actor: person, service: registry.id, method: 'GET', path: '/v2/nodes', status: result.status,
      available: nodes.filter(node => result.routes.includes(node.route)).map(node => node.id), selected: [selectedNode.id] };
  }

  async function relayAvailability(person) {
    const relays = services.filter(service => service.kind === 'relay');
    const observed = await pages.get(person).evaluate(async urls => {
      const result = [];
      for (const url of urls) {
        const response = await fetch(`${url}/v2/meta`, { credentials: 'omit', referrerPolicy: 'no-referrer', cache: 'no-store', redirect: 'error', signal: AbortSignal.timeout(15000) });
        if (!response.ok) throw new Error('The configured private relay is unavailable.');
        const meta = await window.__hoiWalkthroughJson(response, 4096);
        if (meta.apiVersion !== 2) throw new Error('The configured private relay uses a different protocol.');
        result.push({ method: 'GET', path: '/relay/v2/meta', status: response.status, apiVersion: meta.apiVersion });
      }
      return result;
    }, relays.map(relay => relay.url));
    return observed.map((result, index) => ({ ...result, actor: person, service: relays[index].id }));
  }

  async function start() {
    if (state.status === 'ready') return snapshot();
    if (nodes.length || services.length || browser || vite) await close();
    state = { status: 'starting', phase: 'Starting local services', session: '', participants: [], communities: {}, services: [], trace: null, journey: null, limitations: LIMITATIONS };
    journeyData = null;
    invitationProgress = { requestCreated: false, inviteCreated: false, joined: false };
    try {
      await mkdir(stateDir, { recursive: true });
      runDir = await mkdtemp(join(resolve(stateDir), 'session-'));
      state.session = runDir.slice(runDir.lastIndexOf(process.platform === 'win32' ? '\\' : '/') + 1);
      const port = await freePort();
      const origin = `http://127.0.0.1:${port}`;
      for (const letter of ['a', 'b']) {
        const registry = createLocalRegistry({ filename: join(runDir, `registry-${letter}.db`), origins: [origin], publicNodes: [], appUrl: `${origin}/`, allowPrivateRoutesForTests: true });
        const url = await listen(registry.server);
        services.push({ ...registry, id: `registry-${letter}`, label: `Registry ${letter.toUpperCase()}`, kind: 'registry', url });
        const relay = createLocalRelay({ filename: join(runDir, `relay-${letter}.db`), origins: [origin] });
        services.push({ ...relay, id: `relay-${letter}`, label: `Private relay ${letter.toUpperCase()}`, kind: 'relay', url: `${await listen(relay.server)}/relay` });
      }
      const registries = services.filter(service => service.kind === 'registry');
      phase('Starting two public nodes');
      const entryNode = await node('node-a', registries);
      await node('node-b', registries, entryNode);
      await until(async () => {
        const results = await Promise.all(registries.map(async registry => (await (await fetch(`${registry.url}/v1/meta`)).json()).publicNodes));
        return results.every(routes => nodes.every(item => routes.includes(item.route)));
      }, 'The registries did not receive both public node announcements.');
      phase('Opening the production browser app');
      process.env.VITE_HOI_BACKEND = 'wasm';
      // Both registries are queried explicitly below. Select only their loopback routes;
      // native nodes can also advertise interface routes which this isolated demo must not dial.
      process.env.VITE_HOI_REGISTRY_URL = '';
      // Step buttons control private delivery. The app's background timer must not upload or
      // decrypt a selected message before its step; explicit calls still use the production client.
      process.env.VITE_HOI_RELAY_URLS = '[]';
      process.env.HOI_SANDBOX = '0';
      const { createServer: createViteServer } = await import('../apps/web/node_modules/vite/dist/node/index.js');
      vite = await createViteServer({ root: join(ROOT, 'apps/web'), server: { host: '127.0.0.1', port, strictPort: true, hmr: false }, logLevel: 'error' });
      await vite.listen();
      const { chromium } = await import('playwright');
      browser = await chromium.launch({ executablePath, channel: browserChannel, args: ['--disable-background-networking', '--disable-component-update', '--disable-domain-reliability'] });
      const origins = new Set([origin, ...services.map(service => new URL(service.url).origin)]);
      phase('Creating Alice and Bob with independent browser keys');
      for (const info of PEOPLE) await person(info, origin, origins);
      const alice = pages.get('alice'), bob = pages.get('bob');
      phase('Seeding public and private communities');
      const community = await call(alice, 'createCommunity', { name: 'sandbox-garden', description: 'A public discussion for this local network.', color: 'mint', limits: LIMITS });
      state.communities.public = { id: community.id, name: community.name };
      phase('Publishing the public listing in both registries');
      await alice.evaluate(async ({ community, registries }) => {
        const { app } = await import('/src/lib/app.svelte.ts');
        const { registryClient } = await import('/src/lib/registry.ts');
        const proof = await app.core.communityPublication(community);
        for (const registry of registries) await registryClient(registry).publish(proof);
        await app.refresh();
      }, { community: community.id, registries: registries.map(registry => registry.url) });
      state.discovery = [await discover('alice', registries[0], nodes[0])];
      phase('Connecting Alice to public node A');
      await until(async () => (await call(alice, 'net')).connections.some(connection => connection.keeps && connection.communities.includes(community.id)), 'Alice did not connect to a carrier for the public community.');
      state.discovery.push(await discover('bob', registries[1], nodes[1]));
      phase('Bob is joining through node B and the native node connection');
      await until(async () => (await call(bob, 'publicJoinStatus', community.id)).status === 'joined', 'Bob did not receive and validate the public community.');
      state.topology = await topology(true);
      state.relayAvailability = await relayAvailability('alice');
      const bobMe = await call(bob, 'me');
      await call(alice, 'vouch', community.id, bobMe.rootKey, 1);
      phase('Creating the private group and encrypted invitation');
      const privateCommunity = await call(alice, 'createPrivateCommunity', { name: 'sandbox-circle', description: 'Only Alice and Bob can read these posts.', color: 'mint', limits: LIMITS });
      state.communities.private = { id: privateCommunity.id, name: privateCommunity.name };
      const request = await call(bob, 'privateJoinRequest');
      invitationProgress.requestCreated = true;
      const invitation = await call(alice, 'privateInvite', privateCommunity.id, request);
      invitationProgress.inviteCreated = true;
      const joinedPrivate = await call(bob, 'privateJoin', invitation);
      invitationProgress.joined = joinedPrivate.id === privateCommunity.id;
      await syncPrivate(alice, privateCommunity.id);
      await syncPrivate(bob, privateCommunity.id);
      await call(alice, 'vouch', privateCommunity.id, bobMe.rootKey, 1);
      await syncPrivate(alice, privateCommunity.id);
      await syncPrivate(bob, privateCommunity.id);
      await refresh();
      state.connections = await connectionEvidence();
      phase('Publishing seeded posts through the real protocol');
      await publish({ author: 'alice', visibility: 'public', title: 'Welcome to the garden', text: 'The community picnic is on Saturday. Public carriers can read this post.' }, true);
      await publish({ author: 'alice', visibility: 'private', title: 'Our private meeting', text: 'Bring the blue notebook. Only invited devices should read this message.' }, true);
      state.status = 'ready';
      phase('Ready to publish and inspect');
      return snapshot();
    } catch (error) {
      state.status = 'error';
      state.error = error.message;
      phase('Startup needs attention');
      throw error;
    }
  }

  function sequenceError() {
    return Object.assign(new Error('Finish or reset the current journey before starting another action.'), { code: 'WALKTHROUGH_SEQUENCE' });
  }

  function incompleteJourney() { return state.journey && (state.journey.failed || !state.journey.complete && state.journey.stage !== 'draft'); }

  async function createPost(page, community, value) {
    const result = await page.evaluate(async ({ community, title, text, visibility }) => {
      const { app } = await import('/src/lib/app.svelte.ts');
      const before = visibility === 'private' ? new Set((await app.core.privateOutbox(community)).map(object => object.id)) : new Set();
      let created;
      try { created = await app.core.createPost({ community, title, text }); }
      catch (error) {
        // This specific core rejection occurs before a post is accepted or queued for delivery.
        // Keep it distinct from errors after creation, which may have changed protocol state.
        if (error?.code === 'invalid' && ['The same words again.', 'Already have it.'].includes(error.message)) return { duplicate: true };
        throw error;
      }
      const objects = visibility === 'private' ? (await app.core.privateOutbox(community)).filter(object => !before.has(object.id)) : [];
      return { post: await app.core.post(created.id), objects };
    }, { community, ...value });
    if (result.duplicate) throw Object.assign(new Error('You already sent this message. Change its title or text and prepare it again, or reset the demonstration to repeat it.'), { code: 'WALKTHROUGH_DUPLICATE' });
    return result;
  }

  async function publish(input, seeding = false, fromJourney = false) {
    if (!fromJourney && (journeyRunning || incompleteJourney())) throw sequenceError();
    if (!seeding && !fromJourney) { state.journey = null; journeyData = null; }
    const value = validatePublication(input);
    if (!seeding && state.status !== 'ready') throw new Error('Wait for the local network to finish starting.');
    const author = pages.get(value.author);
    const receiverId = value.author === 'alice' ? 'bob' : 'alice';
    const receiver = pages.get(receiverId);
    const community = state.communities[value.visibility].id;
    phase(`Publishing as ${PEOPLE.find(person => person.id === value.author).name}`);
    if (value.visibility === 'public') {
      state.topology = await topology(true);
      await author.evaluate(async community => (await import('/src/lib/app.svelte.ts')).app.requirePostingConnection(community), community);
    }
    const { post, objects } = await createPost(author, community, value);
    const ciphertextIds = objects.map(object => object.id);
    state.trace = { postId: post.id, author: value.author, visibility: value.visibility, title: value.title, text: value.text, complete: false,
      steps: [{ id: value.author, label: 'Signed on the author’s device', status: 'observed', detail: `The core created post ${post.id}.`, recordIds: [post.id] }] };
    try {
      if (value.visibility === 'private') {
        state.trace.ciphertextIds = ciphertextIds;
        if (!state.trace.ciphertextIds.length) throw new Error('The private core did not produce a new encrypted object for this post.');
        await syncPrivate(author, community);
        await syncPrivate(receiver, community);
        for (const service of services.filter(service => service.kind === 'relay')) {
          const stored = service.snapshot().records.map(record => record.id);
          if (!state.trace.ciphertextIds.every(id => stored.includes(id))) throw new Error('A local relay has not shown the encrypted objects for this post.');
        }
      }
      await until(async () => {
        try { return (await call(receiver, 'post', post.id)).id === post.id; } catch { return false; }
      }, 'The other client has not received this post yet. The signed post remains on its author’s device.');
      state.trace.received = true;
      if (value.visibility === 'public') await until(() => nodes.every(item => inspectPublicRecord({ filename: join(item.directory, 'hoi.db'), postId: post.id }).present), 'Both public nodes have not stored this post yet.');
      if (value.visibility === 'public') state.topology = await topology(true);
      await refresh();
      for (const page of pages.values()) await page.evaluate(id => { location.hash = `#/p/${id}`; }, post.id);
      state.trace.complete = true;
      phase('Ready to publish and inspect');
      return snapshot();
    } catch (error) {
      state.trace.error = error.message;
      phase('Post created; delivery needs attention');
      await snapshot();
      throw error;
    }
  }

  async function startJourney(input) {
    if (state.status !== 'ready' || journeyRunning || incompleteJourney()) throw sequenceError();
    const value = validatePublication(input);
    const community = state.communities[value.visibility];
    const author = pages.get(value.author), receiverId = value.author === 'alice' ? 'bob' : 'alice';
    let setup;
    if (value.visibility === 'private') {
      // Finish older setup/previous-run updates before observing this message's new objects.
      await syncPrivate(author, community.id);
      await syncPrivate(pages.get(receiverId), community.id);
      const group = await call(author, 'privateGroup', community.id);
      if (group.pieces !== 'whole') throw new Error('This walkthrough requires whole private messages rather than scattered pieces.');
      setup = { summary: 'Both people were invited to this private group. Their group keys stay inside their own Rust cores. No registry is used for this delivery.',
        memberCount: group.members.length, epoch: group.epoch, transport: group.transport, pieces: group.pieces, availability: await relayAvailability(value.author) };
    } else {
      setup = { summary: 'During setup, the registries listed the public community and carrier routes. The clients already used those routes to join. The registries do not carry the post.',
        discovery: { registryIds: services.filter(service => service.kind === 'registry').map(service => service.id), nodeIds: nodes.map(node => node.id), communityId: community.id }, topology: await topology(true) };
    }
    for (const page of pages.values()) await page.evaluate(() => { window.__hoiWalkthroughDownloads = []; });
    state.connections = await connectionEvidence();
    setup.connection = state.connections[value.visibility];
    state.journey = newJourney({ id: `${state.session}-${++journeyNumber}`, value, community, setup });
    journeyData = { value, objects: [] };
    state.trace = null;
    phase('Message prepared. Follow each step to see what changes.');
    return snapshot();
  }

  const postEvidence = post => ({ alias: 'P1', id: post.id, title: post.title, text: post.text,
    author: post.author.name, checks: post.checks, createdAt: post.createdAt, expiresAt: post.expiresAt });

  async function receiverHasPost(journey) {
    if (!journey.post) return false;
    try { return (await call(pages.get(journey.receiver), 'post', journey.post.id)).id === journey.post.id; }
    catch { return false; }
  }

  async function nextJourney(expected) {
    const journey = state.journey;
    if (!journey || !journeyData || journey.complete || journey.failed || journeyRunning || state.status !== 'ready'
      || expected?.journeyId !== journey.id || expected?.stage !== journey.stage) throw sequenceError();
    journeyRunning = true;
    delete journey.error;
    delete journey.recoverable;
    const author = pages.get(journey.author), receiver = pages.get(journey.receiver);
    const community = journey.community.id;
    const relays = services.filter(service => service.kind === 'relay');
    try {
      if (journey.visibility === 'public') {
        // Public WebRTC runs automatically. Capture its completed evidence; do not claim that
        // the observer paused individual public hops or measured their exact packet order.
        await publish(journeyData.value, false, true);
        journey.post = postEvidence(await call(author, 'post', state.trace.postId));
        journey.stored = nodes.map(node => {
          const result = inspectPublicRecord({ filename: join(node.directory, 'hoi.db'), postId: journey.post.id, plaintextCanary: journey.message.text });
          return { service: node.id, label: node.label, objectAlias: 'P1', objectId: journey.post.id,
            size: result.record.bytes, matchingPost: result.present, readable: true, record: result.record };
        });
        const recovered = await call(receiver, 'post', journey.post.id);
        journey.recovered = { ...postEvidence(recovered), samePost: recovered.id === journey.post.id,
          sameTitle: recovered.title === journey.message.title, sameText: recovered.text === journey.message.text };
        journey.stage = 'open'; journey.complete = true;
      } else if (journey.stage === 'draft') {
        const { post, objects } = await createPost(author, community, journeyData.value);
        if (!objects.length || objects.length > 8) throw new Error('This text did not produce a bounded set of new encrypted packages.');
        journey.post = postEvidence(post);
        journey.objects = objects.map(cipherArtifact);
        journeyData.objects = objects;
        if (await receiverHasPost(journey)) throw new Error('The receiver already has this new message; staged delivery cannot be claimed.');
        journey.stage = 'seal';
        state.trace = { postId: post.id, author: journey.author, visibility: 'private', title: post.title, text: post.text,
          ciphertextIds: objects.map(object => object.id), complete: false,
          steps: [{ id: journey.author, label: 'Signed on the author’s device', status: 'observed', detail: 'The core created this signed post.', recordIds: [post.id] }] };
      } else if (journey.stage === 'seal') {
        journey.uploads = await syncPrivate(author, community, journey.objects.map(object => object.id));
        const stored = [];
        for (const relay of relays) for (const [index, object] of journeyData.objects.entries()) {
          const result = relay.inspectObject({ id: object.id, room: object.room, expectedBody: Buffer.from(object.body, 'base64url') });
          if (!result.present || !result.record.hashMatchesId || !result.matchesExpected) throw new Error('A relay copy does not match the author’s complete encrypted package.');
          stored.push({ service: relay.id, label: relay.label, objectAlias: journey.objects[index].alias, objectId: object.id,
            size: result.record.size, sha256: result.record.sha256, identical: true, room: object.room, sequence: result.record.sequence, expires: result.record.expires });
        }
        if (await receiverHasPost(journey)) throw new Error('The receiver opened the message before the guided download step.');
        journey.stored = stored;
        journey.stage = 'store';
      } else if (journey.stage === 'store') {
        const downloaded = await receiver.evaluate(async ({ community, objects, endpoint }) => {
          const { app } = await import('/src/lib/app.svelte.ts');
          const { toBase64Url } = await import('/src/lib/core/rpc-core.ts');
          const result = [];
          for (const object of objects) {
            const proof = await app.core.privateRead(community, [`objects/${object.id}`]);
            if (proof.room !== object.room) throw new Error('The group changed rooms before download.');
            const read = proof.reads[0];
            const response = await fetch(`${endpoint}/v2/rooms/${object.room}/objects/${object.id}`, {
              credentials: 'omit', referrerPolicy: 'no-referrer', cache: 'no-store', redirect: 'error', signal: AbortSignal.timeout(15000),
              headers: { 'x-hoi-read-time': String(read.time), 'x-hoi-read-nonce': read.nonce, 'x-hoi-read-signature': read.signature },
            });
            if (!response.ok) throw new Error('The local relay did not return the encrypted package.');
            const declared = response.headers.get('content-length');
            if (declared !== null && Number(declared) !== object.size) {
              await response.body?.cancel();
              throw new Error('The relay declared a different encrypted package size.');
            }
            if (!response.body) throw new Error('The relay returned no encrypted bytes.');
            const bytes = new Uint8Array(object.size), reader = response.body.getReader();
            let length = 0;
            for (;;) {
              const chunk = await reader.read();
              if (chunk.done) break;
              if (length + chunk.value.length > bytes.length) {
                await reader.cancel();
                throw new Error('The downloaded package exceeds the expected size.');
              }
              bytes.set(chunk.value, length); length += chunk.value.length;
            }
            if (length !== object.size) throw new Error('The downloaded size differs from the sender’s package.');
            const hash = [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map(value => value.toString(16).padStart(2, '0')).join('');
            if (hash !== object.id) throw new Error('The downloaded package hash does not match.');
            result.push({ id: object.id, body: toBase64Url(bytes), method: 'GET', status: response.status, readProofUsed: true });
          }
          // This is only ciphertext in the managed browser, not a core import or decrypted post.
          window.__hoiWalkthroughDownloads = result;
          return result;
        }, { community, objects: journeyData.objects.map(({ id, room, size }) => ({ id, room, size })), endpoint: relays[0].url });
        journey.downloads = downloaded.map((item, index) => {
          const bytes = Buffer.from(item.body, 'base64url'), expectedBytes = Buffer.from(journeyData.objects[index].body, 'base64url');
          if (!bytes.equals(expectedBytes)) throw new Error('The received bytes differ from the encrypted package sent.');
          return { service: relays[0].id, label: relays[0].label, objectAlias: journey.objects[index].alias, objectId: item.id,
            size: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex'), identical: true, hex: bytes.toString('hex'), method: item.method, status: item.status, readProofUsed: item.readProofUsed };
        });
        if (await receiverHasPost(journey)) throw new Error('The message was decrypted before the guided opening step.');
        journey.stage = 'download';
      } else if (journey.stage === 'download') {
        await receiver.evaluate(async ({ community, ids }) => {
          const { app } = await import('/src/lib/app.svelte.ts');
          const objects = window.__hoiWalkthroughDownloads;
          if (!Array.isArray(objects) || objects.length !== ids.length || objects.some((object, index) => object.id !== ids[index])) throw new Error('The expected downloaded packages are missing.');
          for (const object of objects) await app.core.privateReceive(community, object.body);
          await app.refresh();
        }, { community, ids: journey.objects.map(object => object.id) });
        const recovered = await call(receiver, 'post', journey.post.id);
        journey.recovered = { ...postEvidence(recovered), samePost: recovered.id === journey.post.id,
          sameTitle: recovered.title === journey.message.title, sameText: recovered.text === journey.message.text };
        if (!journey.recovered.samePost || !journey.recovered.sameText || !journey.recovered.sameTitle) throw new Error('The recovered message does not match the input.');
        journey.stage = 'open'; journey.complete = true;
        state.trace.received = true; state.trace.complete = true;
        for (const page of pages.values()) await page.evaluate(id => { location.hash = `#/p/${id}`; }, journey.post.id);
      } else throw sequenceError();
      await refresh();
      phase(journey.complete ? 'Message recovered and verified. Compare the input and result.' : 'Step complete. Inspect the input and result, then continue.');
      return snapshot();
    } catch (error) {
      if (error.code === 'WALKTHROUGH_DUPLICATE' && journey.stage === 'draft' && !journey.post && !journey.objects.length) {
        journey.failed = false;
        journey.recoverable = true;
        journey.error = error.message;
        phase('This message was already sent. Change its title or text and prepare it again.');
        throw error;
      }
      // A failed create/store/open may already have changed real protocol state. Do not retry
      // it as though nothing happened, or claim that the last completed stage is still current.
      journey.failed = true;
      journey.error = 'This step could not be verified and may have partially completed. Earlier panels show captured evidence. Reset the demonstration before continuing.';
      throw error;
    } finally { journeyRunning = false; }
  }

  async function snapshot() {
    const participants = [];
    for (const info of PEOPLE) {
      const page = pages.get(info.id);
      const feeds = { public: [], private: [] };
      if (page && !page.isClosed()) for (const visibility of ['public', 'private']) {
        const community = state.communities[visibility]?.id;
        if (!community) continue;
        try {
          const items = await call(page, 'feed', { community, sort: 'fresh' });
          feeds[visibility] = await Promise.all(items.slice(0, 8).map(async item => {
            const post = await call(page, 'post', item.id);
            return { id: post.id, title: post.title, text: post.text, author: post.author?.name };
          }));
        } catch { /* Incomplete startup remains inspectable. */ }
      }
      participants.push({ ...info, feeds });
    }
    const inspection = state.trace?.visibility === 'private' ? relayCanary(state.trace.text) : {};
    const observed = services.map(service => ({ ...service.snapshot(inspection), id: service.id, label: service.label, kind: service.kind, status: 'ready',
      ...(service.kind === 'relay' && state.trace?.visibility === 'private' && !inspection.plaintextCanary
        ? { canaryNote: 'This message is too short for a meaningful ciphertext byte search.' } : {}) }));
    for (const item of nodes) {
      const data = state.trace ? inspectPublicRecord({ filename: join(item.directory, 'hoi.db'), postId: state.trace.postId, plaintextCanary: state.trace.text }) : { present: false };
      observed.push({ id: item.id, label: item.label, kind: 'node', status: item.exited ? 'stopped' : 'ready', records: data.present ? [data.record] : [],
        visibleFields: ['Public signed records, post text, author keys, community, expiry'], hiddenFields: ['Private community posts and content keys'], present: data.present, containsCanary: data.containsCanary });
    }
    state.participants = participants;
    state.services = observed;
    if (state.trace) {
      const trace = state.trace;
      const authorStep = trace.steps[0];
      const receiverId = trace.author === 'alice' ? 'bob' : 'alice';
      trace.steps = [authorStep, ...observed.filter(service => service.kind === (trace.visibility === 'private' ? 'relay' : 'node')).map(service => {
        const privatePath = trace.visibility === 'private' && service.kind === 'relay';
        const publicPath = trace.visibility === 'public' && service.kind === 'node';
        const ids = privatePath ? service.records.filter(record => trace.ciphertextIds?.includes(record.id)).map(record => record.id) : publicPath && service.present ? [trace.postId] : [];
        return { id: service.id, label: service.label, status: privatePath || publicPath ? ids.length ? 'observed' : 'pending' : 'not-in-path',
          detail: privatePath ? `${ids.length} matching encrypted object(s) stored.` : publicPath ? service.present ? 'Matching signed public record stored; content is readable here.' : 'This post has not been observed in this store.' : service.kind === 'registry' ? 'Discovery metadata only; posts do not pass through a registry.' : 'This service is not part of this delivery.', recordIds: ids };
      }), { id: receiverId, label: 'Read on the other device', status: trace.received ? 'observed' : 'pending', detail: trace.received ? 'The receiving core returned the same post ID and readable content.' : 'Waiting for verified receipt.', recordIds: trace.received ? [trace.postId] : [] }];
    }
    state.flows = {
      public: describeFlow({ visibility: 'public', topology: state.topology, discovery: state.discovery }),
      private: describeFlow({ visibility: 'private', availability: state.relayAvailability }),
    };
    state.flow = state.flows.private;
    if (state.journey) {
      describeJourney(state.journey);
      state.journey.flow = describeFlow({ visibility: state.journey.visibility, journey: state.journey,
        topology: state.journey.setup.topology || state.topology, discovery: state.discovery, availability: state.journey.setup.availability });
    }
    return structuredClone(state);
  }

  async function screenshot(persona, visibility) {
    if (!PEOPLE.some(person => person.id === persona)) throw new Error('Unknown participant.');
    if (visibility !== undefined && !['public', 'private'].includes(visibility)) throw new Error('Choose a public or private community.');
    const page = pages.get(persona);
    if (!page || page.isClosed()) throw new Error('This participant is not ready yet.');
    if (visibility) {
      const community = state.communities[visibility]?.id;
      if (!community) throw new Error('This community is not ready yet.');
      const posts = await call(page, 'feed', { community, sort: 'fresh' });
      const post = state.trace?.visibility === visibility && posts.some(post => post.id === state.trace.postId) ? state.trace.postId : posts[0]?.id;
      await page.evaluate(hash => { location.hash = hash; }, post ? `#/p/${post}` : `#/c/${community}`);
      if (post) {
        const title = posts.find(item => item.id === post).title;
        await page.getByRole('heading', { name: title, exact: true }).waitFor({ timeout: 10_000 });
      }
    }
    return page.screenshot({ type: 'png', fullPage: false });
  }

  async function close() {
    await browser?.close(); browser = undefined; pages = new Map(); contexts = [];
    await vite?.close(); vite = undefined;
    for (const item of nodes) if (!item.exited && !item.error) {
      const ended = once(item.child, 'exit'); item.child.kill(); await ended;
    }
    nodes = [];
    for (const service of services) await service.close();
    services = [];
    state.status = 'stopped';
  }

  async function reset() {
    await close();
    if (runDir) {
      const owned = resolve(runDir);
      if (!/^session-[a-zA-Z0-9]+$/.test(relative(resolve(stateDir), owned))) throw new Error('Refusing to reset data outside the current walkthrough session.');
      // Only the exact mkdtemp directory owned by this run is traversed. Symlinks are removed,
      // never followed, so reset cannot erase data outside this disposable session.
      const removeOwned = async directory => {
        for (const entry of await readdir(directory, { withFileTypes: true })) {
          const target = join(directory, entry.name);
          if (entry.isDirectory() && !entry.isSymbolicLink()) await removeOwned(target);
          else await unlink(target);
        }
        await rmdir(directory);
      };
      await removeOwned(owned);
    }
    return start();
  }
  return { start, reset, publish, startJourney, nextJourney, snapshot, screenshot, close };
}
