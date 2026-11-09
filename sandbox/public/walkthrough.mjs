// Local observation only: no keys or cryptographic operations are implemented in this UI.
import { createPlayer } from './walkthrough-player.mjs';
const $ = selector => document.querySelector(selector);
const names = { alice: 'Alice', bob: 'Bob' };
const stepNames = { draft: 'Write', seal: 'Protect', store: 'Store', download: 'Download', open: 'Open' };
const stageTitles = { draft: 'Alice prepares the message', seal: 'Protect the message on Alice’s device', store: 'Keep encrypted copies on the relays', download: 'Bring the same encrypted bytes to Bob', open: 'Bob opens and checks the message' };
const defaults = {
  private: { title: 'Our private meeting', text: 'Meet by the blue door at seven. Bring the notebook.' },
  public: { title: 'A picnic for everyone', text: 'The community picnic is in the garden on Saturday.' },
};
let state = null, connected = false, busy = false, scenario = 'private', selectedStage = null;
let selectedService = null, pollTimer, pollCount = 0, requestSerial = 0, screenshotSerial = 0;
let selectedInspection = null, cryptoSelection = { sender: null, receiver: null };
let player;
const stageOrder = { setup: -1, draft: 0, seal: 1, store: 2, download: 3, open: 4 };
const evidenceNames = { observed: 'Observed', explained: 'Core explanation', inferred: 'Inferred route', configured: 'Configured', pending: 'Not reached yet', unknown: 'Not verified' };
function evidenceBadge(value) { const badge = el('span', 'journey-evidence-badge', evidenceNames[value] || value); badge.dataset.evidence = value; return badge; }
function reached(stage) { return (stageOrder[selectedStage] ?? 0) >= (stageOrder[stage] ?? -1); }
function visibleEvidence(item) {
  const publicReview = item.stage && state?.journey?.visibility === 'public' && state.journey.complete && selectedStage !== 'draft';
  return publicReview || reached(item.availableAt || item.stage) ? (typeof item.evidence === 'string' ? item.evidence : item.status) || 'explained' : 'pending';
}
function el(tag, className, value) {
  const item = document.createElement(tag);
  if (className) item.className = className;
  if (value !== undefined && value !== null) item.textContent = String(value);
  return item;
}
function text(selector, value) { $(selector).textContent = value ?? ''; }
function details(label, ...content) {
  const node = el('details'); node.append(el('summary', '', label), ...content); return node;
}
function tag(label, kind) { const node = el('span', 'journey-tag', label); node.dataset.kind = kind; return node; }
function artifactTitle(title, alias) {
  const node = el('div', 'journey-artifact-title'); node.append(el('h3', '', title));
  if (alias) node.append(el('span', 'journey-alias', alias));
  return node;
}
function messageView(value) {
  const node = el('div', 'journey-message');
  node.append(el('h4', '', value?.title ?? ''), el('p', '', value?.text ?? ''));
  return node;
}
function fingerprint(value, label = 'Fingerprint') {
  if (!value) return el('p', 'journey-muted', 'No fingerprint was captured.');
  return details(label, el('code', 'journey-fingerprint', value),
    el('p', 'journey-definition', 'A fingerprint identifies these exact bytes. It does not reveal a key or decrypt the content.'));
}
function proof(label, ok, description) {
  const node = el('p', 'journey-proof'); node.dataset.ok = String(ok === true);
  node.append(el('strong', '', ok === true ? '✓' : 'Not confirmed'), el('span', '', label + (description ? ' · ' + description : '')));
  return node;
}
function plainArtifact(message, title = 'Your message', alias = 'M1') {
  const node = el('div'); node.append(artifactTitle(title, alias), tag('Readable text', 'readable'), messageView(message)); return node;
}
function byteDetails(object) {
  const sections = [];
  const meanings = {
    Nonce: 'A public per-object value used by the encryption operation. This is not the decryption key.',
    'Encrypted bytes': 'The protected content, including padding. The original title and text cannot be read from this display.',
    'Authentication tag': 'Used by the receiving core to detect altered bytes. This is not a key or a signature to decode.',
  };
  for (const segment of object.segments ?? []) {
    const section = el('section', 'journey-byte-section');
    section.append(el('h4', '', segment.name + ' · ' + segment.bytes + ' bytes'),
      el('p', '', 'Starts at byte ' + segment.offset + '. ' + (meanings[segment.name] ?? 'A measured part of the stored object.')),
      el('pre', 'journey-bytes', segment.hex ?? 'No bytes captured.'));
    sections.push(section);
  }
  const output = details('Inspect the encrypted bytes',
    el('p', 'journey-definition', 'These are the actual object bytes, written in hexadecimal: two characters represent one byte. The outer layout below does not expose the secret group state.'),
    ...sections,
    details('All ' + object.size + ' object bytes', el('pre', 'journey-bytes journey-bytes-full', object.hex ?? 'No full bytes captured.')));
  return output;
}
function cipherArtifact(object, { compact = false, label = 'Encrypted package' } = {}) {
  const node = el('div');
  if (!object) { node.append(el('p', 'journey-muted', 'No encrypted package has been created yet.')); return node; }
  node.append(artifactTitle(label, object.alias), tag('Encrypted · not readable text', 'ciphertext'));
  node.append(el('p', '', object.size + ' bytes · a whole encrypted object'));
  node.append(el('pre', 'journey-bytes', (object.hex ?? '').slice(0, 128) || 'No bytes captured.'));
  node.append(el('p', 'journey-muted', 'First ' + Math.min(64, (object.hex?.length ?? 0) / 2) + ' bytes shown.'));
  node.append(fingerprint(object.id, 'How this package is linked across steps'));
  if (!compact) node.append(byteDetails(object));
  return node;
}
function signedArtifact(post) {
  const node = el('div');
  if (!post) { node.append(el('p', 'journey-muted', 'The core has not created a signed post yet.')); return node; }
  node.append(artifactTitle('Signed post', post.alias || 'P1'), tag('Readable on the member’s device', 'readable'), messageView(post));
  node.append(el('p', 'journey-definition', 'A signed post is your message plus authorship and integrity information. Signing lets a receiver check it; signing alone does not hide the text.'));
  node.append(fingerprint(post.id, 'Signed-post identity'));
  if (post.checks) node.append(details('Checks reported by the core', el('pre', 'journey-bytes', JSON.stringify(post.checks, null, 2))));
  return node;
}
function copiesView(copies, objects, downloaded = false) {
  const node = el('div');
  node.append(artifactTitle(downloaded ? 'Downloaded package' : 'Stored copies', objects[0]?.alias));
  node.append(tag('Still encrypted', 'ciphertext'));
  if (!copies.length) node.append(el('p', 'journey-muted', 'No matching bytes have been observed yet.'));
  for (const copy of copies) {
    const row = el('section', 'journey-copy');
    row.append(el('h4', '', (downloaded ? 'From ' : '') + copy.label),
      el('p', 'journey-muted', (copy.objectAlias || 'Package') + ' · ' + copy.size + ' bytes'),
      proof('Identical to Alice’s encrypted package', copy.identical === true, 'complete bytes compared'),
      fingerprint(copy.objectId, 'Matching package fingerprint'));
    if (downloaded && copy.hex) row.append(details('Inspect the downloaded bytes', el('pre', 'journey-bytes journey-bytes-full', copy.hex)));
    node.append(row);
  }
  node.append(el('p', 'journey-definition', downloaded
    ? 'Bob has the same protected package. Downloading it has not yet asked his core to open the content.'
    : 'These are complete copies of the same package, not separate pieces that must be stitched together.'));
  return node;
}
function showError(message) { text('#connection-error', message); $('#connection-error').hidden = !message; }
function setBusy(value) {
  busy = value;
  const unavailable = !connected || busy || state?.busy || state?.status !== 'ready';
  const active = state?.journey && !state.journey.complete && state.journey.stage !== 'draft';
  for (const input of $('#example-form').elements) input.disabled = unavailable || active || state?.journey?.failed;
  $('#scenario-private').disabled = unavailable || active || state?.journey?.failed; $('#scenario-public').disabled = unavailable || active || state?.journey?.failed;
  // Recovery must not depend on a successful observation or a ready network. The server
  // still rejects a reset while an operation is running; stale evidence cannot lock us out.
  $('#reset').disabled = busy; $('#recovery-reset').disabled = busy; $('#refresh').disabled = false;
  $('#next').disabled = unavailable || state?.journey?.failed || !state?.journey?.nextAction || selectedStage !== state?.journey?.stage;
  $('#return-latest').disabled = busy;
  for (const button of $('#journey-steps').querySelectorAll('button')) button.disabled = busy || !state?.journey || button.dataset.complete !== 'true';
  player?.update();
}
function renderScenario() {
  for (const kind of ['private', 'public']) $('#scenario-' + kind).setAttribute('aria-pressed', String(scenario === kind));
  text('#scenario-description', scenario === 'private'
    ? 'Alice wants Bob to know where to meet, without the storage services reading the plan.'
    : 'Alice wants everyone to see a picnic announcement. Compare what changes when the post is public.');
  text('#prepare-note', state?.journey?.failed ? 'Reset the demonstration to start a fresh run.' : state?.journey?.recoverable ? 'Change the title or message, then choose Prepare this message again.' : state?.journey && !state.journey.complete && state.journey.stage !== 'draft'
    ? 'Finish this example or reset the demonstration before preparing another message.' : scenario === 'private'
    ? 'This first step only prepares the input. You decide when each next operation runs.'
    : 'Public delivery runs automatically over the real network. You will review its captured results afterward.');
  const setup = $('#setup-explanation'); setup.replaceChildren();
  if (scenario === 'private') setup.append(
    el('p', '', 'Alice and Bob have already joined the private group. Their devices hold the group state needed to protect and open messages. Secret values stay in the Rust core and are never displayed here.'),
    el('p', '', 'Relays A and B are storage services. They have no member’s decryption keys. We will keep two complete copies of the encrypted package.'));
  else setup.append(
    el('p', '', 'Alice and Bob have joined the public community. The registries have listed its name and the carriers that serve it; that discovery happened before this message.'),
    el('p', '', 'Public carriers can read the public signed post. Transport encryption protects the connection, not the post from the carrier.'));
}
function stageList(journey) {
  return journey?.stages?.length ? journey.stages : ['draft', 'seal', 'store', 'download', 'open'].map(id => ({ id, status: 'pending' }));
}
function renderSteps(journey) {
  const list = $('#journey-steps'), stages = stageList(journey);
  list.dataset.count = stages.length; list.replaceChildren();
  for (const [index, stage] of stages.entries()) {
    const item = el('li'), button = el('button');
    button.type = 'button'; button.dataset.complete = String(stage.status !== 'pending');
    const label = journey?.visibility === 'public' && stage.id === 'seal' ? 'Sign' : stepNames[stage.id] || stage.title;
    button.setAttribute('aria-label', (index + 1) + ' ' + label);
    if (selectedStage === stage.id) button.setAttribute('aria-current', 'step');
    button.disabled = !journey || stage.status === 'pending' || busy;
    button.append(el('span', 'journey-step-number', index + 1), el('span', '', label));
    button.addEventListener('click', () => { selectedStage = stage.id; renderJourney(); });
    item.append(button); list.append(item);
  }
}
function renderReadability(journey, stage) {
  const list = $('#readability'); list.replaceChildren();
  const permissions = (journey?.failed && selectedStage === journey.stage ? journey.permissions : stage?.permissions) || journey?.permissions || [
    { label: 'Alice', state: 'readable', detail: 'Will type the original plan.' },
    { label: 'Relays A and B', state: 'none', detail: 'No message has been sent.' },
    { label: 'Bob', state: 'none', detail: 'No message has been sent.' },
  ];
  for (const permission of permissions) {
    const item = el('li');
    const labels = { readable: 'Can read the text', ciphertext: 'Encrypted bytes only', none: 'No message yet', unknown: 'Not verified' };
    item.append(el('h4', '', permission.label), tag(labels[permission.state] || permission.state, permission.state), el('p', '', permission.detail));
    list.append(item);
  }
}
function renderComparison(journey) {
  const holder = $('#message-comparison'); holder.replaceChildren();
  holder.hidden = selectedStage !== 'open' || !journey?.recovered || journey.failed;
  if (holder.hidden) return;
  holder.append(el('h3', '', 'Compare the beginning and the end'));
  const grid = el('div', 'journey-comparison-grid');
  for (const [label, value] of [['Original message', journey.message], ['Recovered on ' + (names[journey.receiver] || 'Bob'), journey.recovered]]) {
    const column = el('div'); column.append(el('h4', '', label), messageView(value)); grid.append(column);
  }
  holder.append(grid);
}
function renderEvidence(stage, journey) {
  const container = $('#evidence-content'); container.replaceChildren();
  $('#evidence-details').hidden = !journey;
  if (journey?.failed) container.append(el('p', 'journey-definition', 'These observations were captured before the failed operation. They do not verify the current state.'));
  const groups = [['Measured in this run', stage?.observations], ['What the core does internally', stage?.internals]];
  for (const [title, values] of groups) if (values?.length) {
    container.append(el('h4', '', title));
    const list = el(title === 'What the core does internally' ? 'ol' : 'ul'); for (const value of values) list.append(el('li', '', value)); container.append(list);
  }
  if (selectedStage === 'seal') container.append(el('p', 'journey-definition', 'Signing, inner serialization and encryption happen inside the Rust core. Their internal byte-by-byte execution is explained here, not separately captured. The resulting signed-post checks and outer encrypted bytes are measured.'));
  if (journey?.mode === 'recorded') container.append(el('p', 'journey-definition', 'Public networking ran automatically. These are observations from that completed run; the buttons review them and do not pause individual network packets.'));
}
function flowModel() {
  const journey = state?.journey;
  const supplied = journey?.flow || state?.flows?.[scenario] || state?.flow;
  if (supplied && (journey || (scenario === 'private') === (supplied.mode === 'private-replicas'))) return supplied;
  const privatePath = (journey?.visibility || scenario) === 'private';
  return { mode: privatePath ? 'private-replicas' : 'public-chain', nodes: [
    { id: 'alice', label: 'Alice', kind: 'person', detail: 'The sending device.' },
    ...['a', 'b'].map(id => ({ id: (privatePath ? 'relay-' : 'node-') + id, label: (privatePath ? 'Private relay ' : 'Public node ') + id.toUpperCase(), kind: privatePath ? 'relay' : 'node', detail: 'Prepare a message to capture this service’s evidence.' })),
    { id: 'bob', label: 'Bob', kind: 'person', detail: 'The receiving device.' },
  ], edges: [], discovery: [] };
}
function selectInspection(kind, id, side) {
  selectedInspection = { kind, id, side };
  renderVisuals(); renderInspection();
  $('#selection-title').focus({ preventScroll: true }); $('#selection-title').scrollIntoView({ block: 'start', behavior: 'auto' });
}
function renderNetworkMap() {
  const flow = flowModel(), journey = state?.journey, privatePath = flow.mode === 'private-replicas';
  const map = $('#network-map'); map.replaceChildren(); map.dataset.layout = privatePath ? 'replicas' : 'chain';
  text('#map-caption', journey?.visibility === 'public' && journey.complete && selectedStage !== 'draft' ? 'Completed public run · individual packet timing not captured' : journey ? 'Evidence through the ' + (stepNames[selectedStage] || selectedStage) + ' step' : 'Configured network · no message sent');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); svg.setAttribute('aria-hidden', 'true'); map.append(svg);
  const mobile = matchMedia('(max-width: 540px)').matches;
  const nodes = (flow.nodes || []).filter(node => node.kind !== 'registry' && (node.kind === 'person' || node.kind === (privatePath ? 'relay' : 'node')));
  const author = journey?.author || 'alice', receiver = journey?.receiver || 'bob';
  const intermediate = nodes.filter(node => node.kind !== 'person');
  const firstHop = (flow.edges || []).find(edge => edge.from === author && intermediate.some(node => node.id === edge.to));
  if (!privatePath && firstHop) intermediate.sort((a, b) => Number(b.id === firstHop.to) - Number(a.id === firstHop.to));
  const order = [author, ...intermediate.map(node => node.id), receiver];
  const positions = new Map();
  for (const node of nodes) {
    const button = el('button', 'journey-network-node'); button.type = 'button'; button.dataset.node = node.id;
    button.setAttribute('aria-label', 'Inspect ' + node.label); button.setAttribute('aria-pressed', String(selectedInspection?.kind === 'node' && selectedInspection.id === node.id));
    const permissions = journey?.failed && selectedStage === journey.stage ? journey.permissions : journey?.stages?.find(stage => stage.id === selectedStage)?.permissions;
    const permission = permissions?.find(item => item.id === node.id);
    const readable = { readable: 'Text readable here', ciphertext: 'Encrypted bytes only', none: 'No message here yet', unknown: 'Not verified' };
    button.append(el('strong', '', node.label), el('small', '', readable[permission?.state] || (node.kind === 'person' ? 'Member’s device' : 'Storage service')));
    if (permission) button.append(tag(permission.state === 'readable' ? 'Readable' : permission.state === 'ciphertext' ? 'Encrypted' : permission.state === 'unknown' ? 'Not verified' : 'Waiting', permission.state));
    let column, row;
    if (privatePath) {
      const position = node.id === author ? 'sender' : node.id === receiver ? 'receiver' : intermediate.indexOf(node) === 0 ? 'first' : 'second';
      const desktop = { sender: ['1', '1 / 4'], first: ['3', '1'], second: ['3', '3'], receiver: ['5', '1 / 4'] };
      const phone = { sender: ['1 / 3', '1'], first: ['1', '3'], second: ['2', '3'], receiver: ['1 / 3', '5'] };
      [column, row] = (mobile ? phone : desktop)[position];
    } else { const index = order.indexOf(node.id); column = mobile ? '1 / 3' : String(index * 2 + 1); row = mobile ? String(index * 2 + 1) : '1'; }
    button.style.gridColumn = column; button.style.gridRow = row;
    button.addEventListener('click', () => selectInspection('node', node.id)); map.append(button); positions.set(node.id, button);
  }
  const allEdges = (flow.edges || []).filter(edge => positions.has(edge.from) && positions.has(edge.to) && edge.kind !== 'discovery');
  const edges = allEdges.filter(edge => edge.kind !== 'connection' || !allEdges.some(other => other.kind === 'delivery' && other.from === edge.from && other.to === edge.to));
  for (const edge of edges) {
    const button = el('button', 'journey-network-edge'); button.type = 'button'; button.dataset.edge = edge.id;
    button.setAttribute('aria-label', 'Inspect connection: ' + edge.label); button.setAttribute('aria-pressed', String(selectedInspection?.kind === 'edge' && selectedInspection.id === edge.id));
    const badge = evidenceBadge(visibleEvidence(edge));
    if (edge.historical && selectedStage === journey?.stage && visibleEvidence(edge) !== 'pending') badge.textContent = 'Captured before failure';
    button.append(el('strong', '', edge.label), badge);
    const side = edge.from === author ? 'send' : 'receive';
    const branch = intermediate.findIndex(node => node.id === (side === 'send' ? edge.to : edge.from));
    if (privatePath) {
      button.style.gridColumn = mobile ? String(Math.max(0, branch) + 1) : side === 'send' ? '2' : '4';
      button.style.gridRow = mobile ? side === 'send' ? '2' : '4' : branch === 1 ? '3' : '1';
    } else {
      const index = Math.min(order.indexOf(edge.from), order.indexOf(edge.to));
      button.style.gridColumn = mobile ? '1 / 3' : String(index * 2 + 2); button.style.gridRow = mobile ? String(index * 2 + 2) : '1';
    }
    button.addEventListener('click', () => selectInspection('edge', edge.id)); map.append(button);
  }
  requestAnimationFrame(() => {
    if (!svg.isConnected) return;
    const bounds = map.getBoundingClientRect(); svg.setAttribute('viewBox', `0 0 ${bounds.width} ${bounds.height}`);
    const markerId = 'flow-arrow';
    const defs = document.createElementNS(svg.namespaceURI, 'defs'), marker = document.createElementNS(svg.namespaceURI, 'marker');
    marker.id = markerId; marker.setAttribute('viewBox', '0 0 10 10'); marker.setAttribute('refX', '9'); marker.setAttribute('refY', '5'); marker.setAttribute('markerWidth', '6'); marker.setAttribute('markerHeight', '6'); marker.setAttribute('orient', 'auto-start-reverse');
    const tip = document.createElementNS(svg.namespaceURI, 'path'); tip.setAttribute('d', 'M 0 0 L 10 5 L 0 10 z'); tip.setAttribute('fill', 'currentColor'); marker.append(tip); defs.append(marker); svg.append(defs);
    for (const edge of edges) {
      const from = positions.get(edge.from).getBoundingClientRect(), to = positions.get(edge.to).getBoundingClientRect();
      const start = mobile ? [from.x + from.width / 2 - bounds.x, from.bottom - bounds.y] : [from.right - bounds.x, from.y + from.height / 2 - bounds.y];
      const end = mobile ? [to.x + to.width / 2 - bounds.x, to.y - bounds.y] : [to.x - bounds.x, to.y + to.height / 2 - bounds.y];
      const middle = mobile ? (start[1] + end[1]) / 2 : (start[0] + end[0]) / 2;
      const path = document.createElementNS(svg.namespaceURI, 'path'); path.setAttribute('class', 'journey-network-path'); path.dataset.status = visibleEvidence(edge); path.dataset.edge = edge.id;
      path.setAttribute('d', mobile ? `M ${start} C ${start[0]} ${middle}, ${end[0]} ${middle}, ${end}` : `M ${start} C ${middle} ${start[1]}, ${middle} ${end[1]}, ${end}`);
      path.setAttribute('marker-end', 'url(#' + markerId + ')'); svg.append(path);
    }
  });
  const discovery = $('#discovery-map'); discovery.replaceChildren();
  const introduction = el('div'); introduction.append(el('h4', '', 'Setup · separate from message delivery'), el('p', 'journey-muted', privatePath
    ? 'Alice and Bob already belong to the group. Public registries are not a stop on this private message’s route.'
    : 'Directory results help the clients find carriers. The post itself follows the delivery arrows above.')); discovery.append(introduction);
  for (const entry of flow.discovery || []) {
    const discoveryEdge = (flow.edges || []).find(edge => edge.id === entry.id && edge.kind === 'discovery');
    const services = discoveryEdge ? [discoveryEdge.to, ...(entry.selected || [])] : entry.selected || [];
    const route = [(names[entry.actor] || entry.actor || 'Client'), ...services.map(id => flow.nodes?.find(node => node.id === id)?.label || id)].join(' → ');
    const button = el('button', '', route); button.type = 'button'; button.dataset.discovery = entry.id;
    button.append(evidenceBadge(entry.status));
    button.setAttribute('aria-pressed', String(selectedInspection?.kind === 'discovery' && selectedInspection.id === entry.id));
    button.addEventListener('click', () => selectInspection('discovery', entry.id)); discovery.append(button);
  }
  if (!(flow.discovery || []).length) discovery.append(el('p', 'journey-muted', privatePath ? 'Private relays are configured for this local example.' : 'Prepare a message to inspect discovery evidence.'));
}
function fallbackCrypto() {
  const sender = [['message', 'Your message'], ['signed-post', 'Signed post'], ['mls', 'Group encryption · MLS'], ['padding', 'Hide the exact length'], ['outer', 'Outer encryption']];
  const receiver = [['package', 'Downloaded package'], ['outer-open', 'Open outer layer'], ['unpad', 'Remove padding'], ['mls-open', 'Open MLS layer'], ['validate', 'Check signed post'], ['readable', 'Read message']];
  return Object.fromEntries([['sender', sender], ['receiver', receiver]].map(([side, items]) => [side, items.map(([id, title]) => ({ id, title, input: 'Prepare a message to capture this step’s input.', action: title, output: 'No output has been captured yet.', evidence: 'pending', readable: null, availableAt: side === 'sender' ? id === 'message' ? 'draft' : 'seal' : id === 'package' ? 'download' : 'open', layers: [{ id: id + '-preview', label: title, evidence: 'pending', readable: null }] }))]));
}
function layerBytes(layer) {
  if (!reached(layer.availableAt)) return null;
  if (layer.hex) return layer.hex;
  const range = layer.byteRange;
  if (!range) return null;
  const sources = range.source === 'receiver' ? state?.journey?.downloads : state?.journey?.objects;
  const artifact = sources?.find(item => (item.alias || item.objectAlias) === range.artifactAlias);
  return artifact?.hex?.slice(range.offset * 2, (range.offset + range.length) * 2) || null;
}
function renderLayer(layer, side, step, parentAvailableAt) {
  const item = { ...layer, availableAt: layer.availableAt || parentAvailableAt }, available = reached(item.availableAt), evidence = visibleEvidence(item);
  const box = el('div', 'journey-layer'); box.dataset.readable = available ? String(layer.readable) : 'null'; box.dataset.layer = layer.id;
  const button = el('button', 'journey-layer-button'); button.type = 'button';
  button.setAttribute('aria-label', 'Inspect layer: ' + layer.label); button.setAttribute('aria-pressed', String(selectedInspection?.kind === 'layer' && selectedInspection.side === side && selectedInspection.id === layer.id));
  button.append(el('strong', '', layer.label + (available && layer.bytes !== undefined ? ' · ' + layer.bytes + ' bytes' : '')), evidenceBadge(evidence));
    button.addEventListener('click', () => { selectedInspection = { kind: 'layer', id: layer.id, side, step: step.id }; renderVisuals(); renderInspection(); $('#selection-title').focus({ preventScroll: true }); $('#selection-title').scrollIntoView({ block: 'start', behavior: 'auto' }); }); box.append(button);
  if (available && layer.value !== undefined) box.append(el('p', 'journey-layer-value', layer.value));
  if (layer.children?.length) { const children = el('div', 'journey-layer-children'); for (const child of layer.children) children.append(renderLayer(child, side, step, item.availableAt)); box.append(children); }
  return box;
}
function renderCrypto() {
  const journey = state?.journey, privatePath = (journey?.visibility || scenario) === 'private';
  $('#crypto-visual').hidden = !privatePath;
  if (!privatePath) return;
  const model = journey?.crypto || fallbackCrypto();
  text('#crypto-copy-model', model.copyModel || 'The receiver opens one complete encrypted copy. The relay copies are not pieces to join together.');
  for (const side of ['sender', 'receiver']) {
    const holder = $('#crypto-' + side), steps = model[side] || []; holder.replaceChildren();
    holder.append(el('h4', '', (side === 'sender' ? 'Protect on ' + (names[journey?.author] || 'Alice') : 'Open on ' + (names[journey?.receiver] || 'Bob')) + '’s device'),
      el('p', 'journey-device-note', 'Private group state stays inside this device. Select an operation, then inspect its nested package layers.'));
    const pipeline = el('ol', 'journey-crypto-pipeline'); pipeline.style.setProperty('--steps', String(steps.length));
    if (!steps.some(step => step.id === cryptoSelection[side])) cryptoSelection[side] = side === 'sender' || selectedStage === 'open' && !journey?.failed ? steps.at(-1)?.id : steps[0]?.id;
    for (const step of steps) {
      const item = el('li'), button = el('button', 'journey-crypto-step'); button.type = 'button'; button.dataset.cryptoStep = step.id; button.dataset.side = side;
      button.setAttribute('aria-label', (side === 'sender' ? 'Protect: ' : 'Open: ') + step.title); button.setAttribute('aria-pressed', String(cryptoSelection[side] === step.id));
      button.append(el('strong', '', step.title), evidenceBadge(visibleEvidence(step)));
      button.addEventListener('click', () => { cryptoSelection[side] = step.id; selectInspection('crypto', step.id, side); }); item.append(button); pipeline.append(item);
    }
    holder.append(pipeline);
    const selected = steps.find(step => step.id === cryptoSelection[side]);
    if (!selected) continue;
    const packet = el('div', 'journey-packet-view'), heading = el('div', 'journey-packet-heading');
    heading.append(el('h4', '', selected.title + ' · output structure'), el('p', '', visibleEvidence(selected) === 'observed' ? 'Captured output; select a layer to see its evidence.' : 'Structure explained from the core; uncaptured bytes are not invented.')); packet.append(heading);
    for (const layer of selected.layers || []) packet.append(renderLayer(layer, side, selected, selected.availableAt));
    holder.append(packet);
  }
}
function renderVisuals() { renderNetworkMap(); renderCrypto(); }
function listValues(values, empty) {
  const node = el('div');
  for (const value of Array.isArray(values) ? values : values ? [values] : []) node.append(el('p', 'journey-definition', value));
  if (!node.childNodes.length) node.append(el('p', 'journey-muted', empty || 'No observation has been captured.'));
  return node;
}
function renderInspection() {
  const selection = selectedInspection;
  text('#selection-title', selection ? 'Selected part' : 'This whole step'); $('#inspect-whole-step').hidden = !selection; $('#selection-source').hidden = !selection; $('#selection-action').hidden = !selection;
  if (!selection) return;
  const input = $('#step-input'), output = $('#step-result'), journey = state?.journey, flow = flowModel(); input.replaceChildren(); output.replaceChildren(); $('#message-comparison').hidden = true;
  let title = '', action = '', before = [], after = [], evidence = 'explained', source = '', artifact;
  if (selection.kind === 'edge') {
    const edge = flow.edges?.find(item => item.id === selection.id); if (!edge) return;
    title = edge.label; action = edge.action; evidence = visibleEvidence(edge); before = edge.input; after = evidence === 'pending' ? ['No transfer receipt is available at the selected step.'] : edge.output;
    source = evidence === 'pending' ? 'This is a planned connection. No receipt is available at the selected step.' : (edge.evidence || []).join(' '); artifact = evidence === 'observed' ? journey?.objects?.find(item => item.id === edge.objectId || edge.artifactIds?.includes(item.alias)) : null;
    if (edge.historical && selectedStage === journey?.stage && evidence !== 'pending') source = 'Captured before failure. ' + source;
    if (artifact) input.append(cipherArtifact(artifact, { compact: true }));
    if (edge.sameBytes !== undefined && evidence === 'observed') output.append(proof('The transferred bytes match', edge.sameBytes));
  } else if (selection.kind === 'node') {
    const node = flow.nodes?.find(item => item.id === selection.id); if (!node) return;
    const incoming = (flow.edges || []).filter(edge => edge.to === node.id), outgoing = (flow.edges || []).filter(edge => edge.from === node.id);
    title = node.label; action = node.kind === 'person' ? 'Keep local member state; create or open the message on this device.' : 'Store and serve the received object.';
    before = incoming.map(edge => (flow.nodes.find(item => item.id === edge.from)?.label || edge.from) + ' → ' + edge.label + ' · ' + evidenceNames[visibleEvidence(edge)]);
    after = outgoing.map(edge => edge.label + ' → ' + (flow.nodes.find(item => item.id === edge.to)?.label || edge.to) + ' · ' + evidenceNames[visibleEvidence(edge)]);
    source = node.detail || 'Select a connecting arrow for its captured transport evidence.'; evidence = 'configured';
    const permission = (journey?.failed && selectedStage === journey.stage ? journey.permissions : journey?.stages?.find(stage => stage.id === selectedStage)?.permissions)?.find(item => item.id === node.id);
    if (permission) {
      output.append(tag(permission.state === 'readable' ? 'Text readable here' : permission.state === 'ciphertext' ? 'Encrypted bytes only' : permission.state === 'unknown' ? 'Not verified' : 'No message here yet', permission.state), el('p', 'journey-definition', permission.detail));
      if (node.id === journey.author) input.append(plainArtifact(journey.message));
      if (node.id === journey.receiver && permission.state === 'readable' && journey.recovered) output.append(plainArtifact(journey.recovered, 'Recovered message', 'P1'));
      if (permission.state === 'ciphertext') for (const object of journey.objects || []) output.append(cipherArtifact(object, { compact: true }));
    }
  } else if (selection.kind === 'discovery') {
    const item = flow.discovery?.find(entry => entry.id === selection.id); if (!item) return;
    title = 'Finding services'; before = [item.source]; action = (names[item.actor] || item.actor) + ' selects available services before sending this message.'; after = item.selected?.map(id => flow.nodes?.find(node => node.id === id)?.label || id); evidence = item.status; source = (item.evidence || []).join(' ');
  } else {
    const model = journey?.crypto || fallbackCrypto(), step = model[selection.side]?.find(item => item.id === (selection.step || selection.id)); if (!step) return;
    title = step.title; before = [step.input]; action = step.action; after = [step.output]; evidence = visibleEvidence(step);
    if (selection.kind === 'layer') {
      function findLayer(layers) { for (const layer of layers || []) { if (layer.id === selection.id) return layer; const child = findLayer(layer.children); if (child) return child; } }
      const layer = findLayer(step.layers); if (!layer) return;
      title = layer.label; evidence = visibleEvidence({ ...layer, availableAt: layer.availableAt || step.availableAt });
      after = [layer.label];
      const bytes = evidence === 'observed' ? layerBytes({ ...layer, availableAt: layer.availableAt || step.availableAt }) : null;
      if (bytes) output.append(el('pre', 'journey-bytes journey-bytes-full', bytes), el('p', 'journey-definition', 'Actual captured bytes in hexadecimal. Two characters represent one byte.'));
      if (evidence === 'observed' && layer.value !== undefined) output.append(el('p', 'journey-layer-value', layer.value));
      if (evidence === 'observed' && layer.fingerprint) output.append(fingerprint(layer.fingerprint));
      if (evidence === 'explained' && layer.sizeBasis) after.push(layer.sizeBasis);
    } else if (evidence === 'observed' && step.artifactAlias) {
      if (step.artifactAlias === 'M1') output.append(plainArtifact(journey.message));
      else if (step.artifactAlias === 'P1') output.append(signedArtifact(selection.side === 'receiver' ? journey.recovered : journey.post));
      else {
        const object = journey.objects.find(item => item.alias === step.artifactAlias), downloaded = journey.downloads?.find(item => item.objectAlias === step.artifactAlias);
        if (object) output.append(cipherArtifact(selection.side === 'receiver' && downloaded ? { ...object, hex: downloaded.hex } : object));
      }
    }
    if (step.readable !== null && evidence !== 'pending' && evidence !== 'unknown') output.prepend(tag(step.readable ? 'Message text readable' : 'Message text encrypted', step.readable ? 'readable' : 'ciphertext'));
    if (evidence === 'pending') after = ['No output has been captured at this step. The diagram shows the planned structure.'];
    source = evidence === 'observed' ? 'Observed at a client or storage boundary in this local run.' : evidence === 'pending' ? 'This output is not available at the selected step.' : evidence === 'unknown' ? 'The operation did not finish; this output has not been verified.' : 'Explained from the production Rust core. Intermediate bytes are not exported by this sandbox.';
  }
  text('#selection-title', title); text('#selection-action', action); text('#selection-source', source); input.append(listValues(before)); output.prepend(evidenceBadge(evidence)); output.append(listValues(after));
}
function renderJourney() {
  const journey = state?.journey;
  const stages = stageList(journey);
  if (!selectedStage || !stages.some(stage => stage.id === selectedStage)) selectedStage = journey?.stage || 'draft';
  const stage = stages.find(item => item.id === selectedStage);
  const reviewing = !!journey && selectedStage !== journey.stage;
  const author = names[journey?.author] || 'Alice', receiver = names[journey?.receiver] || 'Bob';
  const isPrivate = (journey?.visibility || 'private') === 'private';
  $('#journey-panel').dataset.stage = selectedStage;
  renderSteps(journey);
  text('#step-location', journey ? (isPrivate ? 'Private plan' : 'Public announcement') + ' · ' + author + ' → ' + receiver : 'Alice → encrypted storage → Bob');
  let title = stageTitles[selectedStage];
  if (!isPrivate && selectedStage === 'seal') title = 'Alice’s core creates a signed public post';
  if (!isPrivate && selectedStage === 'store') title = 'Public carriers store the readable post';
  if (!isPrivate && selectedStage === 'open') title = 'Bob checks and reads the public post';
  text('#step-title', journey?.failed && !reviewing ? 'This operation could not be verified' : journey ? title.replaceAll('Alice', author).replaceAll('Bob', receiver) : 'Start with a private plan');
  text('#view-mode', !journey ? 'Ready to begin' : reviewing ? 'Reviewing an earlier step · no operation is repeated'
    : journey.failed ? 'Stopped · reset required' : journey.recoverable ? 'Edit the message to continue' : journey.mode === 'recorded' ? 'Recorded public delivery' : journey.complete ? 'Finished · review any step' : 'Live simulation · waiting for your next action');
  const descriptions = {
    draft: 'The title and text are the input. No post or encrypted package has been created for this example yet.',
    seal: 'The core turns the message into a signed post, then protects it for the private group. The signed post stays readable on the member’s device; the encrypted package is what leaves it.',
    store: 'The relays receive and keep complete encrypted copies. Compare their package fingerprints and the measured byte equality.',
    download: receiver + ' downloads the same encrypted package. At this pause it is held for his client to open; the recovered message has not been released yet.',
    open: receiver + '’s core uses his device’s private group state to open the package and check the enclosed post. It returns the readable title and text below.',
  };
  const publicDescriptions = {
    seal: 'Signing adds information a receiver can check about the author and integrity. It does not hide a public message.',
    store: 'Both public carriers kept the signed public post. Its title and body are readable in their stored record.',
    open: receiver + ' received and checked the signed public post. No private-message decryption was needed.',
  };
  text('#step-explanation', journey?.failed && !reviewing ? 'The operation may have changed a client or store before stopping. Review the earlier evidence or reset to start again.' : journey ? (!isPrivate && publicDescriptions[selectedStage] || descriptions[selectedStage]) : 'Prepare the message above. Then advance one real operation at a time.');
  text('#journey-error', journey?.error); $('#journey-error').hidden = !journey?.error;
  $('#journey-recovery').hidden = !journey?.failed;
  const input = $('#step-input'), result = $('#step-result'); input.replaceChildren(); result.replaceChildren();
  const objects = journey?.objects || [];
  if (!journey) {
    input.append(plainArtifact({ title: defaults.private.title, text: defaults.private.text }));
    result.append(el('p', '', 'A readable message for Bob, with encrypted storage in between.'), el('p', 'journey-definition', 'The next screen will show the actual outputs from this local run.'));
  } else if (journey.failed && !reviewing) {
    input.append(plainArtifact(journey.message));
    result.append(artifactTitle('Result not verified'), tag('Operation stopped', 'unknown'), el('p', '', 'The operation may have partially completed. Reset before starting another run.'));
  } else if (selectedStage === 'draft') {
    input.append(plainArtifact(journey.message));
    result.append(artifactTitle('Prepared input', 'M1'), tag('Ready on ' + author + '’s device', 'readable'), el('p', '', 'The message has not been sent.'), el('p', 'journey-muted', 'Next, ask the real core to create the signed post' + (isPrivate ? ' and encrypted package.' : '.')));
  } else if (selectedStage === 'seal') {
    input.append(plainArtifact(journey.message));
    if (isPrivate) {
      for (const object of objects) result.append(cipherArtifact(object));
      result.append(details('Where is the signed post?', signedArtifact(journey.post)));
    } else result.append(signedArtifact(journey.post));
  } else if (selectedStage === 'store') {
    if (isPrivate) {
      for (const object of objects) input.append(cipherArtifact(object, { compact: true }));
      result.append(copiesView(journey.stored || [], objects));
      result.append(el('p', 'journey-definition', 'The relays can see object size, its fingerprint, the opaque room identifier and expiry. They cannot read the title and text or derive the group’s decryption state.'));
    } else {
      input.append(signedArtifact(journey.post));
      result.append(artifactTitle('Readable carrier copies', 'P1'), tag('Public text is readable here', 'readable'));
      for (const copy of journey.stored || []) {
        const row = el('section', 'journey-copy'); row.append(el('h4', '', copy.label), proof('Same signed-post ID stored', copy.matchingPost === true), fingerprint(copy.objectId || copy.postId || journey.post?.id)); result.append(row);
      }
      result.append(messageView(journey.post));
    }
  } else if (selectedStage === 'download') {
    input.append(copiesView(journey.stored || [], objects));
    result.append(copiesView(journey.downloads || [], objects, true));
  } else if (selectedStage === 'open') {
    if (isPrivate) {
      const firstDownload = journey.downloads?.[0];
      for (const object of objects) input.append(cipherArtifact({ ...object, ...(firstDownload?.objectId === object.id ? { hex: firstDownload.hex } : {}) }, { compact: true }));
      input.append(el('p', 'journey-definition', 'Plus ' + receiver + '’s existing private group state, held inside his core. The relay does not supply a decryption key.'));
    } else input.append(signedArtifact(journey.post));
    result.append(artifactTitle('Original text recovered', 'P1'), tag('Readable on ' + receiver + '’s device', 'readable'), messageView(journey.recovered));
    result.append(proof('Same title', journey.recovered?.sameTitle ?? (journey.recovered?.title === journey.message?.title && !!journey.recovered)),
      proof('Same message text', journey.recovered?.sameText === true), proof('Same signed post', journey.recovered?.samePost === true));
    if (journey.recovered?.checks) result.append(details('Validation returned by the receiving core', el('pre', 'journey-bytes', JSON.stringify(journey.recovered.checks, null, 2))));
    result.append(el('p', 'journey-definition', isPrivate ? 'Nothing was reconstructed by combining the two relay copies. One complete encrypted object was opened with the receiver’s group state.' : 'Public carriers and the receiver can read the same public content.'));
  }
  renderComparison(journey); renderReadability(journey, stage); renderEvidence(stage, journey);
  $('#return-latest').hidden = !reviewing;
  $('#next').hidden = reviewing || !!journey?.complete || !!journey?.failed || !!journey?.recoverable;
  text('#next', journey?.nextAction || 'Prepare a message first');
  text('#next-explanation', !journey ? 'Nothing has been sent yet.' : reviewing ? 'This is captured evidence from an earlier step. The live run stays at its latest step.' : journey.failed ? 'Use Reset and start again above to recover.' : journey.recoverable ? 'Edit the title or message above, then prepare it again. You can also reset to repeat the original example.' : journey.complete ? 'The experiment is complete. Review the earlier inputs and outputs, or prepare a new message above.' : journey.mode === 'recorded' ? 'The next action runs public delivery, then captures its results for review.' : 'The next operation runs only when you press the button.');
  $('#app-preview').hidden = selectedStage !== 'open' || !journey?.recovered || journey.failed;
  $('#app-preview > summary').textContent = 'See ' + receiver + '’s real app screen';
  renderVisuals(); renderInspection();
  setBusy(busy);
}
function renderAdvanced() {
  const services = state?.services || [], picker = $('#service-picker');
  if (!services.some(service => service.id === selectedService)) selectedService = services[0]?.id || null;
  picker.replaceChildren(...services.map(service => { const option = el('option', '', service.label); option.value = service.id; return option; }));
  picker.value = selectedService || '';
  const holder = $('#service-inspector'), service = services.find(item => item.id === selectedService); holder.replaceChildren();
  if (service) {
    holder.append(el('p', 'journey-muted', service.kind === 'registry' ? 'Public discovery data. This service is not in the private message path.' : service.kind === 'relay' ? 'Actual encrypted object storage and visible delivery metadata.' : 'Actual public signed-record storage.'));
    if (service.containsCanary === true && service.kind === 'relay') {
      const warning = el('p', 'journey-error', 'The selected message bytes occur in relay storage. Investigate this match; a byte search alone does not prove plaintext disclosure.'); warning.setAttribute('role', 'alert'); holder.append(warning);
    }
    if (service.canaryNote) holder.append(el('p', 'journey-muted', service.canaryNote));
    holder.append(details('Raw observed fields', el('pre', 'journey-bytes journey-bytes-full', JSON.stringify(service, null, 2))));
  }
  $('#limitations').replaceChildren(...(state?.limitations || []).map(value => el('li', '', value)));
}
function render() {
  text('#network-status', state?.status === 'ready' ? 'Network ready' : state?.status === 'error' ? 'Needs attention' : 'Starting');
  $('#network-status').dataset.state = state?.status || 'starting';
  text('#phase', state?.phase || 'Preparing the local network…');
  text('#session', state?.session || '—');
  if (state?.error) showError(state.error);
  renderScenario(); renderJourney(); renderAdvanced();
  player?.update();
}
async function request(path, body) {
  const response = await fetch(path, { method: body === undefined ? 'GET' : 'POST', cache: 'no-store', credentials: 'same-origin', redirect: 'error',
    ...(body === undefined ? {} : { headers: { 'content-type': 'application/json', 'x-sandbox-control': '1' }, body: JSON.stringify(body) }),
    signal: AbortSignal.timeout(body === undefined ? 30000 : 600000) });
  const value = await response.json();
  if (!response.ok) throw new Error(value.error || 'The local operation did not finish.');
  return value;
}
function accept(next, follow = false) {
  const changed = next.journey?.id !== state?.journey?.id;
  if (follow || changed) { selectedStage = next.journey?.stage || 'draft'; selectedInspection = null; cryptoSelection = { sender: null, receiver: null }; }
  if (changed) {
    $('#example-composer').open = !next.journey;
    $('#journey-details').open = false;
    ++screenshotSerial;
    $('#app-preview').open = false;
    const image = $('#app-screen'); image.onload = null; image.onerror = null; image.removeAttribute('src'); image.hidden = true;
    text('#screenshot-status', '');
    if (next.journey) {
      scenario = next.journey.visibility;
      $('#post-title').value = next.journey.message.title;
      $('#post-text').value = next.journey.message.text;
    }
  }
  state = next; connected = true; showError(''); render();
  if (next.journey?.recoverable) $('#example-composer').open = true;
}
function scheduleRefresh() {
  clearTimeout(pollTimer);
  if (pollCount < 300 && (busy || state?.busy || !state || state.status === 'starting')) pollTimer = setTimeout(() => { pollCount++; void refresh(); }, 2000);
}
async function refresh() {
  const serial = ++requestSerial;
  try { const next = await request('/api/state'); if (serial === requestSerial) accept(next); }
  catch (error) { if (serial === requestSerial) { connected = false; showError(error.message); text('#network-status', 'Disconnected'); setBusy(busy); } }
  finally { if (serial === requestSerial) scheduleRefresh(); }
}
async function mutate(path, payload, pending, finished, { fromPlayer = false } = {}) {
  if (busy || path !== '/api/reset' && (!connected || state?.status !== 'ready')) return false;
  if (!fromPlayer) player?.stop();
  setBusy(true); showError(''); text('#action-status', pending); pollCount = 0; scheduleRefresh();
  try {
    const result = await request(path, payload); ++requestSerial; accept(result, true); text('#action-status', finished);
    if (path === '/api/journey/start' || path === '/api/journey/next') {
      if (path === '/api/journey/start') player?.focus();
    }
    return true;
  }
  catch (error) {
    showError(error.message); text('#action-status', 'The step did not finish. Refresh the evidence or reset the demonstration.');
    // A failed POST has finished. Do not hold the local action lock while its follow-up GET
    // waits on a slow or disconnected client; Refresh and Reset are the recovery controls.
    connected = false; setBusy(false);
    player?.stop();
    void refresh();
    return false;
  }
  finally { setBusy(false); scheduleRefresh(); }
}
function refreshScreenshot() {
  const journey = state?.journey;
  if (!$('#app-preview').open || !journey?.recovered || journey.failed || state?.status !== 'ready' || busy) return;
  const image = $('#app-screen'), serial = ++screenshotSerial;
  image.hidden = true; image.alt = (names[journey.receiver] || 'Bob') + '’s recovered message in the running hoi app';
  text('#screenshot-status', 'Capturing the receiving client…');
  image.onload = () => { if (serial === screenshotSerial) { image.hidden = false; text('#screenshot-status', 'Captured from the real receiving client.'); } };
  image.onerror = () => { if (serial === screenshotSerial) text('#screenshot-status', 'The app screen is unavailable. Try refreshing it.'); };
  image.src = '/api/screenshot/' + journey.receiver + '?visibility=' + journey.visibility + '&capture=' + Date.now();
}
for (const kind of ['private', 'public']) $('#scenario-' + kind).addEventListener('click', () => {
  player?.stop(); $('#example-composer').open = true;
  scenario = kind; $('#post-title').value = defaults[kind].title; $('#post-text').value = defaults[kind].text; renderScenario();
  player?.update();
});
$('#example-form').addEventListener('submit', event => {
  event.preventDefault();
  void mutate('/api/journey/start', { author: 'alice', visibility: scenario, title: $('#post-title').value.trim(), text: $('#post-text').value.trim() }, 'Preparing the input…', 'Input prepared. Follow the next operation below.');
});
$('#next').addEventListener('click', () => {
  const journey = state?.journey;
  if (!journey || selectedStage !== journey.stage || !journey.nextAction) return;
  void mutate('/api/journey/next', { journeyId: journey.id, stage: journey.stage }, 'Running: ' + journey.nextAction + '…', 'Operation finished. Inspect its input and result.');
});
$('#return-latest').addEventListener('click', () => { selectedStage = state?.journey?.stage; renderJourney(); });
$('#inspect-whole-step').addEventListener('click', () => { selectedInspection = null; renderJourney(); });
window.addEventListener('resize', () => { renderVisuals(); });
$('#refresh').addEventListener('click', () => { pollCount = 0; void refresh(); });
$('#service-picker').addEventListener('change', () => { selectedService = $('#service-picker').value; renderAdvanced(); });
$('#app-preview').addEventListener('toggle', refreshScreenshot); $('#refresh-screen').addEventListener('click', refreshScreenshot);
for (const selector of ['#reset', '#recovery-reset']) $(selector).addEventListener('click', () => { player?.stop(); $('#reset-dialog').showModal(); });
$('#reset-dialog').addEventListener('close', () => {
  if ($('#reset-dialog').returnValue === 'reset') void mutate('/api/reset', {}, 'Preparing a fresh local network…', 'Fresh demonstration ready.');
});
player = createPlayer({ host: $('#story-player'), getState: () => state, getScenario: () => scenario,
  canRun: () => connected && state?.status === 'ready' && !busy && !state?.busy,
  onAdvance: () => {
    const journey = state?.journey;
    if (!journey?.nextAction) return Promise.resolve(false);
    return mutate('/api/journey/next', { journeyId: journey.id, stage: journey.stage }, 'Running: ' + journey.nextAction + '…', 'Captured. Continue through the explanation in the player.', { fromPlayer: true });
  },
});
renderScenario(); renderJourney(); void refresh();
