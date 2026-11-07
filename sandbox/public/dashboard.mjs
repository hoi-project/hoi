const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
const roles = {
  honest: { name: 'Neighbour', goal: 'Keep a useful conversation going.' },
  defender: { name: 'Defender', goal: 'Recognise the challenge and protect the community.' },
  challenger: { name: 'Organiser', goal: 'Build influence and persuade the community to accept rules that concentrate power.' },
  forger: { name: 'Forger', goal: 'Try to make an invalid signed record pass the checks.' },
  persuader: { name: 'Persuader', goal: 'Persuade a participant to open the harmless local canary link.' },
};
const roleOptions = '<option value="honest">Neighbour</option><option value="defender">Defender</option><option value="challenger">Organiser · attempts a takeover</option><option value="forger">Forger</option><option value="persuader">Persuader</option>';
const driverLabels = { scripted: 'Scripted demo', llm: 'Local AI model', codex: 'Codex CLI', claude: 'Claude Code CLI' };
const driverOptions = Object.entries(driverLabels).map(([value, label]) => `<option value="${value}">${label}</option>`).join('');
const communityPresets = ['neighbours', 'takeover', 'townhall'];
const exerciseScenarios = [
  { id: 'balanced', name: 'A bit of everything', population: 4, activeMembers: 4, brief: 'A neighbour, defender, forger, and persuader explore conversation, protocol validation, and one intercepted local link.' },
  { id: 'protocol', name: 'The protocol', population: 2, activeMembers: 2, brief: 'A neighbour and a forger accompany bounded probes of the real protocol validator.' },
  { id: 'social', name: 'The link exercise', population: 3, activeMembers: 3, brief: 'A neighbour, defender, and persuader explore interaction with a harmless intercepted local link. This does not validate external links.' },
];
const agentNodes = new Map();
const signatures = new Map();
let state = null;
let connected = false;
let busy = false;
let configDirty = false;
let loadedConfig = false;
let toastTimer;
let requestSerial = 0;
let agentSerial = 0;

function el(tag, className, value) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (value !== undefined) node.textContent = String(value);
  return node;
}

function text(selector, value, root = document) {
  const node = $(selector, root);
  const next = String(value ?? '—');
  if (node.textContent !== next) node.textContent = next;
}

function format(value) {
  if (value == null) return '';
  return typeof value === 'string' ? value : JSON.stringify(value, null, 2);
}

function tone(status) {
  if (['passed', 'pass', 'ready', 'healthy', 'completed'].includes(status)) return 'pass';
  if (['failed', 'fail', 'error', 'unhealthy', 'exited', 'dead'].includes(status)) return 'fail';
  if (['running', 'starting', 'active', 'pending'].includes(status)) return 'active';
  return 'waiting';
}

function badge(node, label, status) {
  node.textContent = label;
  node.dataset.tone = tone(status);
}

function readable(value) {
  return String(value ?? 'waiting').replaceAll('_', ' ').replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function date(value) {
  if (!value) return null;
  const result = new Date(value);
  return Number.isNaN(result.getTime()) ? null : result;
}

function toast(message) {
  clearTimeout(toastTimer);
  const node = $('#toast');
  node.textContent = message;
  node.hidden = false;
  toastTimer = setTimeout(() => { node.hidden = true; }, 6000);
}

function safeHref(value) {
  if (typeof value !== 'string' || !value) return null;
  try {
    const url = new URL(value, location.origin);
    return ['http:', 'https:'].includes(url.protocol) ? url.href : null;
  } catch { return null; }
}

function memberInviteHref(invite) {
  const base = safeHref(state.links?.member);
  if (!base || typeof invite !== 'string') return null;
  const value = invite.trim();
  const match = value.match(/^(?:web\+)?hoi:\/\/join\/?\?(.+)$/i) || value.match(/#\/join\?(.+)$/);
  const query = match?.[1] || (/^\??c=/.test(value) ? value.replace(/^\?/, '') : null);
  if (!query || !new URLSearchParams(query).has('c')) return null;
  const url = new URL(base);
  url.hash = `/join?${query}`;
  return url.href;
}

function hasChanged(key, value) {
  const next = JSON.stringify(value);
  if (signatures.get(key) === next) return false;
  signatures.set(key, next);
  return true;
}

async function fetchJson(url, options = {}) {
  const response = await fetch(url, { cache: 'no-store', signal: AbortSignal.timeout(15000), ...options });
  const result = await response.json().catch(() => null);
  if (!response.ok) throw new Error(result?.error || `The sandbox returned HTTP ${response.status}.`);
  if (!result) throw new Error('The sandbox returned an empty response.');
  return result;
}

async function refresh() {
  const serial = ++requestSerial;
  try {
    const result = await fetchJson('/api/state');
    if (serial !== requestSerial) return;
    state = result;
    connected = true;
    render();
  } catch (error) {
    if (serial !== requestSerial) return;
    connected = false;
    text('#connection span:last-child', 'Disconnected');
    $('#connection').dataset.tone = 'fail';
    const notice = $('#connection-error');
    notice.textContent = `Connection lost. Controls are paused; any running agents may still be working. ${error.message} Reconnecting automatically…`;
    notice.hidden = false;
    controls();
  }
}

async function mutate(url, body, method = 'POST') {
  if (busy) return false;
  busy = true;
  controls();
  try {
    await fetchJson(url, {
      method,
      headers: { 'Content-Type': 'application/json', 'X-Sandbox-Control': '1' },
      body: JSON.stringify(body ?? {}),
    });
    await refresh();
    return true;
  } catch (error) {
    toast(error.message);
    return false;
  } finally {
    busy = false;
    controls();
  }
}

function controls() {
  const ready = connected && Boolean(state?.ready) && !busy;
  const running = state?.run?.status === 'running';
  $('#start-run').disabled = !ready || running;
  $('#stop-run').disabled = !connected || busy || !running;
  $$('[data-needs-ready]').forEach((node) => { node.disabled = !ready; });
  $$('#config-form input, #config-form select, #config-form button').forEach((node) => { node.disabled = !ready || running; });
  $$('#add-agent-form input, #add-agent-form select').forEach((node) => { node.disabled = !ready; });
  for (const [id, row] of agentNodes) {
    const agent = state?.agents?.find((item) => String(item.id) === id);
    const active = ['running', 'starting'].includes(agent?.status);
    const containerRunning = Array.isArray(state?.services) && state.services.some((service) => String(service.id) === id && service.state === 'running');
    $('[data-action="start"]', row).disabled = !ready || active;
    $('[data-action="stop"]', row).disabled = !connected || busy || !(active || containerRunning);
    $$('input, select, button[type="submit"]', row).forEach((node) => { node.disabled = !ready || running; });
    if (agent?.steward) $('select[name="role"]', row).disabled = true;
  }
}

function render() {
  const ready = Boolean(state.ready);
  const error = state.error;
  text('#connection span:last-child', error ? 'Needs attention' : ready ? 'Connected' : 'Preparing sandbox');
  $('#connection').dataset.tone = error ? 'fail' : ready ? 'pass' : 'active';
  $('#connection-error').hidden = !error;
  if (error) text('#connection-error', format(error));
  renderRun();
  renderLinks();
  renderAgents();
  renderConfig();
  renderScenario();
  renderProvider();
  renderEvidence();
  controls();
}

function renderRun() {
  const run = state.run;
  const preparing = run?.status === 'running' && run.phase === 'preparing';
  const checks = Array.isArray(run?.checks) ? run.checks : [];
  const agents = Array.isArray(state.agents) ? state.agents : [];
  text('#working-count', `${agents.filter((agent) => ['running', 'starting'].includes(agent.status)).length} / ${agents.length}`);
  text('#checks-count', checks.length ? `${checks.filter((check) => check.status === 'passed').length} / ${checks.length}` : '—');
  const usage = run?.usage;
  text('#calls-count', `${usage?.calls || 0} / ${usage?.limit ?? state.config?.maxCalls ?? 60}`);
  text('#usage-detail', usage ? `${usage.inFlight || 0} decisions in progress · ${usage.completed || 0} completed · ${usage.errors || 0} errors. Calls are counted across the whole run; this is not a price estimate.` : 'AI calls are shared across all residents. Provider usage may cost money; the cap limits requests, not currency.');
  const scenario = run?.scenario;
  const steward = agents.find(agent => agent.steward)?.name || 'Alex';
  text('#run-community', scenario ? `${scenario.name} · ${Number(scenario.population || 0).toLocaleString()} people in the story · ${scenario.activeMembers ?? agents.length} active residents.${communityPresets.includes(scenario.id) ? ` ${steward} acts as the community owner.` : ''}` : 'Choose a community scenario below. Only active identities make real posts and votes.');
  const session = String(typeof state.session === 'object' ? state.session?.name || state.session?.id || 'local' : state.session || 'local');
  text('#session-name', session.length > 20 ? `${session.slice(0, 8)}…` : session);
  $('#session-name').title = session;
  const titles = { running: 'The experiment is running.', completed: 'The experiment is finished.', stopped: 'The experiment was stopped.', error: 'The experiment needs attention.' };
  text('#run-title', !state.ready ? 'Getting the community ready.' : preparing ? 'The residents are getting ready.' : run ? titles[run.status] || 'Ready for another experiment.' : 'Ready when you are.');
  let detail = !state.ready ? 'The first setup includes identity work and can take a few minutes. The controls unlock when the community is ready.' : 'Choose a scenario and AI provider below, then start the community. Open the app to join the discussion.';
  if (run) {
    if (run.verdict === 'failed') detail = 'An adversary reached a tested goal. Inspect the checks and activity below to see the evidence.';
    else if (preparing) detail = 'Preparing identities and waiting for real protocol standing, about one minute after admission. The discussion timer starts when the residents are ready.';
    else if (run.status === 'running') detail = 'Residents are taking turns, with AI calls spread across the discussion window. Join in, stop a resident, or end the run at any time.';
    else if (run.verdict === 'passed') detail = 'The completed checks held for this experiment. Review the evidence for the boundaries tested.';
    else detail = 'This run has unfinished or inconclusive checks. Review the evidence before drawing a conclusion.';
  }
  text('#run-detail', detail);
  const labels = { passed: '✓ Selected checks passed', failed: '× Check failed', running: 'In progress', inconclusive: 'Inconclusive' };
  badge($('#verdict'), preparing && run.verdict !== 'failed' ? 'Preparing residents' : run ? labels[run.verdict] || 'Not evaluated' : 'Not evaluated', run?.verdict);
  renderClock();
}

function renderClock() {
  const end = date(state?.run?.endsAt);
  if (state?.run?.status === 'running' && (state.run.phase === 'preparing' || !end)) {
    text('#time-remaining', state.run.phase === 'preparing' ? 'Preparing' : 'Starting');
    return;
  }
  if (state?.run?.status !== 'running') {
    text('#time-remaining', state?.run ? 'Ended' : `${state?.config?.duration || '—'} min`);
    return;
  }
  const seconds = Math.max(0, Math.ceil((end.getTime() - Date.now()) / 1000));
  text('#time-remaining', `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`);
}

function renderLinks() {
  for (const role of ['watcher', 'member', 'owner']) {
    const node = $(`#${role}-link`);
    const href = safeHref(state.links?.[role]);
    if (href) {
      node.href = href;
      node.removeAttribute('aria-disabled');
    } else {
      node.removeAttribute('href');
      node.setAttribute('aria-disabled', 'true');
    }
  }
  const invites = [];
  for (const community of state.communities || []) {
    if (community.invite) invites.push({ name: community.name || community.id || 'Community invite', current: community.id === state.communities[0]?.id, value: community.invite, href: memberInviteHref(community.invite) });
  }
  const pairing = state.links?.pairing;
  if (typeof pairing === 'string' && pairing) invites.push({ name: 'Browser pairing link', value: pairing });
  else if (pairing && typeof pairing === 'object') {
    for (const [name, value] of Object.entries(pairing)) if (value) invites.push({ name: `${readable(name)} pairing link`, value: format(value) });
  }
  if (hasChanged('invites', invites)) {
    const list = $('#invites');
    list.replaceChildren();
    for (const invite of invites) {
      const row = el('div', 'invite-item');
      const content = el('div');
      content.append(el('h3', '', invite.name), el('code', 'invite-value', invite.value));
      if (invite.current) content.prepend(el('span', 'small-label', 'Current community'));
      const copy = el('button', 'pill pill-sm', 'Copy');
      copy.type = 'button';
      copy.setAttribute('aria-label', `Copy ${invite.name}`);
      copy.addEventListener('click', () => copyText(String(invite.value), copy));
      const actions = el('div', 'invite-actions');
      actions.append(copy);
      if (invite.href) {
        const open = el('a', 'pill pill-sm', 'Open as member ↗');
        open.href = invite.href;
        open.target = '_blank';
        open.rel = 'noopener noreferrer';
        open.setAttribute('aria-label', `Open ${invite.name} invite as member in a new tab`);
        actions.append(open);
      }
      row.append(content, actions);
      list.append(row);
    }
    if (!invites.length) list.append(el('p', 'empty-state', 'Invites will appear when the community is ready.'));
    text('#invite-count', invites.length ? `· ${invites.length} available` : '');
  }
  const services = Array.isArray(state.services) ? state.services : Object.entries(state.services || {}).map(([name, value]) => ({ name, ...(typeof value === 'object' ? value : { status: value }) }));
  if (hasChanged('services', services)) {
    const list = $('#services');
    list.replaceChildren();
    for (const service of services) {
      const item = el('div', 'service-item');
      const status = service.state || service.status || 'unknown';
      const label = el('span', 'tag');
      badge(label, readable(status), status === 'running' ? 'passed' : status);
      item.append(el('strong', '', service.service || service.name || service.id || 'Service'), label);
      if (service.status) item.title = String(service.status);
      list.append(item);
    }
    if (!services.length) list.append(el('p', 'muted', 'Service details are not available yet.'));
    text('#service-summary', services.length ? `${services.length} services` : 'Waiting for services');
  }
}

async function copyText(value, button) {
  try {
    await navigator.clipboard.writeText(value);
    const previous = button.textContent;
    button.textContent = 'Copied ✓';
    setTimeout(() => { button.textContent = previous; }, 1800);
    toast('Invite copied.');
  } catch {
    const code = button.parentElement.querySelector('code');
    if (code) {
      const range = document.createRange();
      range.selectNodeContents(code);
      const selection = window.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
    }
    toast('The browser could not copy automatically. The invite is selected; use your copy shortcut.');
  }
}

function createAgent(agent) {
  const row = el('article', 'agent-row');
  const serial = ++agentSerial;
  // This template contains only static application markup. All received text uses textContent.
  row.innerHTML = `<span class="avatar" aria-hidden="true"></span><div class="agent-body"><div class="agent-title"><h3></h3><span class="tag agent-status"></span></div><p class="agent-description"></p><p class="agent-persona" hidden></p><p class="agent-goal"></p><p class="agent-last"></p><p class="agent-error" hidden></p><details class="agent-memory" hidden><summary>Recent memory</summary><ol></ol></details><details class="agent-edit"><summary>Configure this resident</summary><form class="agent-form"><div class="field"><label for="edit-name-${serial}">Name</label><input id="edit-name-${serial}" name="name" maxlength="32" required autocomplete="off"></div><div class="field"><label for="edit-role-${serial}">Role</label><select id="edit-role-${serial}" name="role">${roleOptions}</select></div><div class="field"><label for="edit-driver-${serial}">Driver</label><select id="edit-driver-${serial}" name="driver">${driverOptions}</select></div><button class="pill pill-sm" type="submit">Save resident</button></form></details></div><div class="agent-actions"><button class="pill pill-sm" data-action="start" type="button">Start</button><button class="pill pill-sm" data-action="stop" type="button">Stop</button></div>`;
  row.dataset.agentId = String(agent.id);
  const form = $('form', row);
  form.addEventListener('input', () => { form.dataset.dirty = 'true'; });
  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    const values = Object.fromEntries(new FormData(form));
    if (await mutate(`/api/agents/${encodeURIComponent(agent.id)}`, values, 'PATCH')) {
      form.dataset.dirty = 'false';
      $('.agent-edit', row).open = false;
      toast('Agent updated.');
    }
  });
  for (const action of ['start', 'stop']) {
    $(`[data-action="${action}"]`, row).addEventListener('click', async () => {
      if (await mutate(`/api/agents/${encodeURIComponent(agent.id)}/${action}`)) toast(action === 'start' ? 'Agent started.' : 'Agent stopped.');
    });
  }
  return row;
}

function renderAgents() {
  const agents = Array.isArray(state.agents) ? state.agents : [];
  const list = $('#agent-list');
  for (const [id, node] of agentNodes) {
    if (!agents.some((agent) => String(agent.id) === id)) { node.remove(); agentNodes.delete(id); }
  }
  if (agents.length) $$('.empty-state', list).forEach((node) => node.remove());
  for (const agent of agents) {
    const id = String(agent.id);
    let row = agentNodes.get(id);
    if (!row) { row = createAgent(agent); agentNodes.set(id, row); list.append(row); }
    const name = agent.name || readable(agent.role);
    text('h3', name, row);
    const avatar = $('.avatar', row);
    avatar.textContent = name.split(/\s+/).slice(0, 2).map((word) => word[0]).join('').toUpperCase();
    avatar.dataset.role = agent.role;
    badge($('.agent-status', row), readable(agent.status || 'idle'), agent.status);
    text('.agent-description', `${agent.steward ? 'Community owner' : roles[agent.role]?.name || readable(agent.role)} · ${driverLabels[agent.driver] || readable(agent.driver)} · ${agent.rounds || 0} turns`, row);
    $('.agent-persona', row).hidden = !agent.persona;
    text('.agent-persona', agent.persona || '', row);
    text('.agent-goal', `Goal: ${agent.goal || roles[agent.role]?.goal || 'Waiting for a goal.'}`, row);
    text('.agent-last', agent.lastAction ? `Last move: ${format(agent.lastAction)}` : 'No moves yet. Ready for a turn.', row);
    const memory = Array.isArray(agent.memory) ? agent.memory.slice(-8) : [];
    $('.agent-memory', row).hidden = !memory.length;
    if (hasChanged(`memory:${id}`, memory)) {
      $('.agent-memory ol', row).replaceChildren(...[...memory].reverse().map((item) => el('li', '', format(item))));
    }
    $('.agent-error', row).hidden = !agent.error;
    if (agent.error) text('.agent-error', format(agent.error), row);
    $('[data-action="start"]', row).setAttribute('aria-label', `Start ${name}`);
    $('[data-action="stop"]', row).setAttribute('aria-label', `Stop ${name}`);
    const form = $('form', row);
    if (form.dataset.dirty !== 'true') {
      form.elements.name.value = name;
      form.elements.role.value = agent.role;
      form.elements.driver.value = agent.driver || 'scripted';
    }
  }
  if (!agents.length && !$('.empty-state', list)) list.append(el('p', 'empty-state', 'No agents yet. Add one below to give the experiment its first participant.'));
}

function renderConfig() {
  if (!state.config || configDirty) return;
  const form = $('#config-form');
  for (const key of ['preset', 'duration', 'rounds', 'pace', 'driver', 'model', 'endpoint', 'codexModel', 'claudeModel', 'maxCalls', 'concurrency']) {
    if (state.config[key] != null) form.elements[key].value = state.config[key];
  }
  if (!communityPresets.includes(form.elements.preset.value)) $('.legacy-presets').open = true;
  text('#config-status', state.run?.status === 'running' ? 'Settings are locked while the community is running.' : loadedConfig ? 'Saved settings. Ready for the next run.' : 'Settings loaded. Ready for the next run.');
  loadedConfig = true;
}

function renderScenario() {
  const form = $('#config-form');
  const preset = form.elements.preset.value;
  const catalog = Array.isArray(state?.scenarios) ? state.scenarios : Object.values(state?.scenarios || {});
  const scenario = [...catalog, ...exerciseScenarios].find((item) => item.id === preset);
  const sameLineup = preset === state?.config?.preset;
  const active = sameLineup ? state.agents?.length : scenario?.activeMembers;
  text('#scenario-name', scenario?.name || readable(preset));
  text('#scenario-brief', scenario?.brief || scenario?.description || 'The scenario brief will appear when the sandbox connects.');
  text('#scenario-population', scenario?.population != null ? Number(scenario.population).toLocaleString() : '—');
  text('#scenario-active', active ?? '—');
  const steward = sameLineup ? state.agents?.find(agent => agent.steward)?.name || 'Alex' : 'Alex';
  text('#scenario-boundary', `${communityPresets.includes(preset) ? `${steward} acts as the community owner in this scenario. ` : ''}Only active identities can post or vote. Background residents add context, not voting weight.${sameLineup && scenario && active !== scenario.activeMembers ? ' This lineup has been customised.' : ''}`);
  const calls = Number(form.elements.maxCalls.value) || 60;
  const concurrency = Number(form.elements.concurrency.value) || 2;
  text('#budget-preview', `Up to ${calls} AI calls shared by the run, with ${concurrency} ${concurrency === 1 ? 'decision' : 'decisions'} at a time. The CLI worker allows at most two Codex/Claude decisions combined. Calls can be spread across the discussion window; scripted turns do not spend AI calls.`);
}

function renderProvider() {
  const driver = $('#config-form').elements.driver.value;
  $('#local-settings').hidden = driver !== 'llm';
  $('#codex-settings').hidden = driver !== 'codex';
  $('#claude-settings').hidden = driver !== 'claude';
  $('#cli-help').hidden = !['codex', 'claude'].includes(driver);
  $('#scripted-help').hidden = driver !== 'scripted';
  const providers = state?.providers;
  const report = Array.isArray(providers?.providers) ? { available: providers.providers.includes(driver) } : providers?.providers?.[driver];
  let status;
  if (driver === 'scripted') status = 'Scripted demonstration · no AI calls.';
  else if (driver === 'llm') status = 'Local API · connection checked when a resident takes a turn.';
  else if (!providers?.online) status = 'Local worker offline. Start the sandbox launcher or the worker command below.';
  else if (!report?.available) status = report?.error || `${driverLabels[driver]} is not available to the worker. Install and sign in to the CLI, then restart the worker.`;
  else status = `${driverLabels[driver]} worker connected · ${providers.running || 0} running · ${providers.pending || 0} queued. Authentication is checked on a request.`;
  text('#provider-status', connected ? status : 'Waiting for the sandbox.');
  $('#provider-status').dataset.tone = ['codex', 'claude'].includes(driver) && (!providers?.online || !report?.available) ? 'waiting' : 'neutral';
}

function renderEvidence() {
  renderOutcomes();
  const checks = Array.isArray(state.run?.checks) ? state.run.checks : [];
  if (hasChanged('checks', checks)) {
    const list = $('#checks');
    list.replaceChildren();
    for (const check of checks) {
      const row = el('div', 'check-row');
      const copy = el('div');
      copy.append(el('h3', '', check.name || check.id || 'Check'), el('p', '', format(check.detail) || 'Waiting for evidence.'));
      const status = el('span', 'tag');
      badge(status, `${check.status === 'passed' ? '✓ ' : check.status === 'failed' ? '× ' : ''}${readable(check.status || 'pending')}`, check.status);
      row.append(copy, status);
      list.append(row);
    }
    if (!checks.length) list.append(el('p', 'empty-state', 'Start a run to see its measured checks here.'));
  }
  const referee = state.referee;
  const currentCommunity = state.communities?.[0];
  if (hasChanged('referee', [referee, currentCommunity?.id])) {
    const counts = $('#referee-counts');
    counts.replaceChildren();
    const metrics = [];
    if (referee && typeof referee === 'object') {
      if (Array.isArray(referee.communities)) {
        const observed = referee.communities.filter(community => community.id === currentCommunity?.id);
        for (const [key, label] of [['members', 'Members'], ['mods', 'Moderators'], ['posts_visible', 'Visible posts'], ['posts_napping', 'Napping posts'], ['banned', 'Banned'], ['proposals', 'Proposals']]) {
          if (observed.some((community) => typeof community[key] === 'number')) metrics.push([label, observed.reduce((total, community) => total + (Number(community[key]) || 0), 0)]);
        }
      } else {
        const values = referee.counts || referee.summary || referee;
        for (const [key, value] of Object.entries(values)) {
          if (typeof value === 'number' && !/time|timestamp|At$/.test(key)) metrics.push([readable(key.replace(/([a-z])([A-Z])/g, '$1 $2')), value]);
        }
      }
    }
    for (const [name, value] of metrics.slice(0, 12)) {
      const item = el('div');
      item.append(el('dt', '', name), el('dd', '', value));
      counts.append(item);
    }
    if (!metrics.length) counts.append(el('p', 'muted', 'Waiting for an observation of the current community. See the raw snapshot for other communities.'));
    const observedAt = date(referee?.updatedAt || referee?.observedAt);
    text('#referee-status', referee?.error ? 'Snapshot unavailable' : `${currentCommunity?.name || 'Current community'}${observedAt ? ` · ${observedAt.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })}` : ''}`);
    text('#referee-raw', referee ? format(referee) : 'No snapshot yet.');
  }
  const events = Array.isArray(state.run?.events) ? state.run.events : [];
  if (hasChanged('events', events)) {
    const list = $('#events');
    list.replaceChildren();
    for (const event of [...events].reverse().slice(0, 100)) {
      const row = el('li', 'event');
      const observedAt = date(event.time || event.at);
      const time = el('time', '', observedAt ? observedAt.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' }) : '—');
      if (observedAt) { time.dateTime = observedAt.toISOString(); time.title = observedAt.toLocaleString(); }
      const agent = state.agents?.find((item) => String(item.id) === String(event.agent));
      row.append(time, el('strong', '', agent?.name || event.agent || 'Referee'), el('p', '', format(event.message || event.detail)));
      list.append(row);
    }
    if (!events.length) list.append(el('li', 'empty-state', 'Agents’ actions and check results will appear here.'));
    text('#event-count', events.length ? `${events.length > 100 ? 'Latest 100 of ' : ''}${events.length} events · newest first` : 'No events yet');
  }
}

function renderOutcomes() {
  const outcomes = state.run?.outcomes;
  const panel = $('#community-outcomes');
  panel.hidden = !outcomes;
  if (!outcomes || !hasChanged('outcomes', [outcomes, state.run?.phase, state.agents?.map(agent => [agent.key, agent.name])])) return;
  const preparing = state.run.phase === 'preparing';
  const proposals = Array.isArray(outcomes.proposals) ? outcomes.proposals : [];
  const blocked = Array.isArray(outcomes.blocked) ? outcomes.blocked : [];
  text('#rules-outcome', preparing ? 'Awaiting observation' : outcomes.rulesChanged ? 'Change observed' : 'No change observed');
  text('#proposal-count', proposals.length);
  text('#blocked-count', blocked.length);
  const list = $('#outcome-proposals');
  list.replaceChildren();
  for (const proposal of proposals) {
    const row = el('li', 'proposal-outcome');
    const copy = el('div');
    copy.append(el('strong', '', `${readable(proposal.kind || 'Community')} proposal`), el('p', '', `${proposal.agreed || 0} agreed · ${proposal.objecting || 0} objecting · ${proposal.waiting || 0} awaiting response`));
    const id = el('code', '', proposal.id || '');
    copy.append(id);
    const status = el('span', 'tag');
    // Proposal status is protocol evidence; a proposal alone never implies a rule change.
    badge(status, readable(proposal.status || 'Unknown'), proposal.status === 'applied' ? 'passed' : 'waiting');
    row.append(copy, status);
    list.append(row);
  }
  if (!proposals.length) list.append(el('li', 'empty-state', preparing ? 'Proposals will appear after the discussion begins.' : 'No proposals in the latest observed state.'));
  const names = blocked.map(key => state.agents?.find(agent => agent.key === key)?.name || String(key));
  text('#blocked-members', names.length ? `Observed blocked during this run: ${names.join(', ')}.` : 'No member has been observed blocked during this run.');
}

$('#start-run').addEventListener('click', async () => {
  if (configDirty) { toast('Save your scenario settings before starting the community.'); $('#config-status').scrollIntoView({ block: 'center' }); return; }
  if (await mutate('/api/run/start')) toast('The experiment has started.');
});
$('#stop-run').addEventListener('click', async () => { if (await mutate('/api/run/stop')) toast('Run stopped. Unfinished checks remain inconclusive.'); });
$('#config-form').addEventListener('input', () => {
  configDirty = true;
  const preset = $('#config-form').elements.preset.value;
  text('#config-status', preset !== state?.config?.preset ? communityPresets.includes(preset) ? 'Saving this scenario creates a fresh community and replaces the resident lineup. Previous communities keep their invites.' : 'Saving this exercise replaces the resident lineup in the current community.' : 'Unsaved changes. Save before starting the next run.');
  renderScenario();
  renderProvider();
});
$('#config-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  const values = Object.fromEntries(new FormData(event.currentTarget));
  for (const key of ['duration', 'rounds', 'pace', 'maxCalls', 'concurrency']) values[key] = Number(values[key]);
  if (await mutate('/api/config', values)) {
    configDirty = false;
    renderConfig();
    renderScenario();
    renderProvider();
    toast('Scenario settings saved.');
  }
});
$('#agent-role').addEventListener('change', (event) => { text('#new-agent-goal', `Goal: ${roles[event.target.value].goal}`); });
$('#add-agent-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  const form = event.currentTarget;
  const values = Object.fromEntries(new FormData(form));
  if (!values.driver) delete values.driver;
  if (await mutate('/api/agents', values)) {
    form.reset();
    text('#new-agent-goal', `Goal: ${roles.honest.goal}`);
    $('#add-agent-details').open = false;
    toast('Agent added. Start it when you are ready.');
  }
});

async function poll() {
  await refresh();
  setTimeout(poll, document.hidden ? 6000 : 2000);
}
document.addEventListener('visibilitychange', () => { if (!document.hidden) refresh(); });
setInterval(renderClock, 1000);
controls();
poll();
