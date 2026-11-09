// The player animates captured evidence. Real mutations are owned by the walkthrough controller.
import { buildStory } from './walkthrough-story.mjs';

const stages = { draft: 0, seal: 1, store: 2, download: 3, open: 4 };
const labels = { alice: 'Alice', bob: 'Bob', 'node-a': 'Public node A', 'node-b': 'Public node B', 'relay-a': 'Private relay A', 'relay-b': 'Private relay B', 'registry-a': 'Registry A', 'registry-b': 'Registry B' };
const evidenceLabels = { observed: 'Captured result', explained: 'Core explanation', inferred: 'Inferred route', pending: 'Not captured yet', unknown: 'Not verified', configured: 'Configured' };
const svgNS = 'http://www.w3.org/2000/svg';
const node = (tag, className, text) => { const element = document.createElement(tag); if (className) element.className = className; if (text !== undefined) element.textContent = text; return element; };
const svgNode = (tag, attributes) => { const element = document.createElementNS(svgNS, tag); for (const [key, value] of Object.entries(attributes)) element.setAttribute(key, String(value)); return element; };

export function createPlayer({ host, getState, getScenario, canRun, onAdvance }) {
  let index = 0, identity = '', playing = false, running = false, timer, generation = 0, animation = 0, lastFrame = '';
  const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)');
  const narrow = matchMedia('(max-width: 620px)');
  host.innerHTML = `<div class="story-top"><div><p class="journey-kicker" id="story-count"></p><h2 id="story-title" tabindex="-1"></h2></div><span id="story-evidence" class="journey-evidence-badge"></span></div>
    <ol class="story-phases" aria-label="Demonstration phases"><li data-phase="connect">1 Connect</li><li data-phase="protect">2 Protect</li><li data-phase="send">3 Deliver</li><li data-phase="open">4 Read</li></ol>
    <div class="story-community"><strong id="story-community-name"></strong><span id="story-members"></span></div>
    <div id="story-map" class="story-map" aria-label="Message location and connections"></div>
    <div class="story-location"><span class="story-location-dot" aria-hidden="true"></span><strong id="story-location"></strong><span id="story-route-note"></span></div>
    <p id="story-description" class="story-description"></p>
    <div class="story-io"><section class="story-card" aria-labelledby="story-input-label"><h3 id="story-input-label">Input</h3><div id="story-input"></div></section><span class="story-transform" aria-hidden="true">→</span><section class="story-card" aria-labelledby="story-output-label"><h3 id="story-output-label">Output</h3><div id="story-output"></div></section></div>
    <div class="story-source"><span id="story-source"></span></div>
    <div class="story-controls"><div><button id="story-back" class="pill pill-sm" type="button">Back</button><button id="story-play" class="pill pill-sm" type="button">Play</button><button id="story-replay" class="journey-text-button" type="button">Replay</button></div><span id="story-status" role="status" aria-live="polite"></span><button id="story-next" class="pill pill-ink" type="button">Next step →</button></div>`;
  const $ = selector => host.querySelector(selector);
  const put = (selector, text) => { $(selector).textContent = text || ''; };
  const frames = () => buildStory(getState(), getScenario());
  const current = () => frames()[index];

  function stop({ render = true } = {}) {
    playing = false; ++generation; clearTimeout(timer); cancelAnimationFrame(animation);
    if (render) draw();
  }
  function readiness() {
    const state = getState(), journey = state?.journey;
    return !!journey && !journey.failed && !journey.recoverable && canRun();
  }
  function schedule() {
    clearTimeout(timer);
    if (playing) timer = setTimeout(() => { void advance(); }, 4000);
  }
  async function advance() {
    if (running || !readiness()) { stop(); return; }
    const story = frames();
    if (index >= story.length - 1) { stop(); return; }
    const target = index + 1, next = story[target], ticket = generation;
    running = true; draw();
    try {
      // A core encryption/open or public publication is atomic. Its smaller visual steps
      // review the captured result; moving backward never repeats that operation.
      while (next.requiredStage && stages[getState()?.journey?.stage] < stages[next.requiredStage]) {
        if (!readiness()) return;
        const before = getState().journey.stage;
        const ok = await onAdvance();
        if (!ok || getState()?.journey?.stage === before || getState()?.journey?.failed || getState()?.journey?.recoverable) { stop({ render: false }); return; }
        if (ticket !== generation) return;
      }
      if (ticket === generation && readiness()) index = target;
    } finally {
      running = false; draw();
      if (index >= frames().length - 1) stop(); else if (playing && ticket === generation) schedule();
    }
  }
  function appendText(holder, value) {
    if (value?.label && !['Input', 'Output'].includes(value.label)) holder.append(node('strong', 'story-value-label', value.label));
    for (const text of value?.text || []) holder.append(node('p', '', text));
  }
  function layersView(layers) {
    const holder = node('div', 'story-layers');
    for (const layer of layers || []) {
      const box = node('div', 'story-layer'); box.dataset.evidence = layer.evidence;
      box.append(node('strong', '', layer.label + (layer.bytes !== undefined ? ' · ' + layer.bytes + ' B' : '')));
      if (layer.value !== undefined) box.append(node('p', '', layer.value));
      if (layer.children?.length) {
        const chips = node('div', 'story-layer-chips');
        for (const child of layer.children) chips.append(node('span', '', child.label));
        box.append(chips);
        const details = node('details'); details.append(node('summary', '', 'Inside this layer'), layersView(layer.children)); box.append(details);
      }
      holder.append(box);
    }
    return holder;
  }
  function compactCipher(holder, object) {
    if (!object?.hex) return;
    holder.append(node('p', 'story-artifact-id', `${object.alias || object.objectAlias} · ${object.size} bytes · complete encrypted package`),
      node('pre', 'story-byte-preview', object.hex.slice(0, 96)),
      node('p', 'story-byte-hint', 'First 48 actual bytes · fingerprint ' + (object.id || object.objectId).slice(0, 20) + '…'));
  }
  function inputContext(frame, story, holder) {
    if (['pending', 'unknown'].includes(frame.evidence)) return;
    const journey = getState()?.journey;
    if (!journey) return;
    if (frame.kind === 'transfer' && frame.artifact === 'encrypted') compactCipher(holder, journey.objects?.[0]);
    else if (frame.cryptoStepId === 'outer-open') compactCipher(holder, journey.downloads?.[0]);
    else if (frame.kind === 'transfer' && frame.artifact === 'post' || frame.id === 'public-read') {
      if (journey.post) { const message = node('blockquote', 'story-readable'); message.append(node('strong', '', journey.post.title), node('p', '', journey.post.text)); holder.append(message); }
    } else if (frame.kind === 'crypto') {
      const previous = story[index - 1];
      if (previous?.kind === 'crypto' && previous.side === frame.side && previous.layers?.length) {
        holder.append(layersView(previous.layers));
      }
    }
  }
  function artifactView(frame, holder) {
    const journey = getState()?.journey;
    if (!journey || !frame.artifact || ['pending', 'unknown'].includes(frame.evidence)) return;
    if (frame.artifact === 'encrypted') {
      const object = journey.objects?.[0];
      if (!object?.hex) return;
      const wrap = node('div', 'story-cipher');
      wrap.append(node('p', 'story-artifact-id', `${object.alias} · ${object.size} bytes · encrypted package`));
      const buttons = node('div', 'story-byte-tabs'), preview = node('pre', 'story-byte-preview'), hint = node('p', 'story-byte-hint');
      const sections = [{ name: 'Package', offset: 0, bytes: object.size }, ...(object.segments || [])];
      const select = section => {
        for (const button of buttons.children) button.setAttribute('aria-pressed', String(button.textContent.startsWith(section.name)));
        const bytes = object.hex.slice(section.offset * 2, (section.offset + section.bytes) * 2);
        preview.textContent = bytes.slice(0, 192);
        hint.textContent = `${Math.min(section.bytes, 96)} of ${section.bytes} actual bytes shown in hexadecimal. Select a segment to inspect it here.`;
      };
      for (const section of sections) {
        const button = node('button', '', section.name + (section.name === 'Package' ? '' : ' · ' + section.bytes + ' B')); button.type = 'button'; button.dataset.bytePart = section.name;
        button.addEventListener('click', () => select(section)); buttons.append(button);
      }
      wrap.append(buttons, preview, hint); select(sections[0]); holder.append(wrap);
      const edge = journey.flow?.edges?.find(edge => edge.id === frame.edgeId);
      if (frame.kind === 'transfer' && edge?.sameBytes === true) holder.append(node('p', 'story-match', '✓ Same complete bytes as the sender’s package.'));
    } else if (frame.artifact === 'post' && frame.kind === 'transfer' && journey.post) {
      const message = node('blockquote', 'story-readable'); message.append(node('strong', '', journey.post.title), node('p', '', journey.post.text)); holder.append(message);
      holder.append(node('p', 'story-byte-hint', 'P1 · the public content stays readable.'));
    } else if (frame.artifact === 'recovered') {
      const recovered = journey.recovered;
      if (!recovered || journey.failed) return;
      const message = node('blockquote', 'story-readable'); message.append(node('strong', '', recovered.title), node('p', '', recovered.text)); holder.append(message);
      holder.append(node('p', 'story-match', recovered.samePost && recovered.sameText && recovered.sameTitle ? '✓ Same post, title and message as ' + (labels[journey.author] || 'the sender') + '’s input.' : 'The recovered result has not matched every input check.'));
    }
  }
  function drawMap(frame, story) {
    const state = getState(), journey = state?.journey, visibility = journey?.visibility || getScenario(), privatePath = visibility === 'private';
    const author = journey?.author || 'alice', receiver = journey?.receiver || 'bob';
    const flow = journey?.flow || state?.flows?.[visibility];
    const carrier = person => person === 'alice' ? 'node-a' : 'node-b';
    const mainIds = privatePath ? [author, 'relay-a', 'relay-b', receiver] : [author, carrier(author), carrier(receiver), receiver];
    const ids = (privatePath ? mainIds : [...mainIds, 'registry-a', 'registry-b']).filter(id => !frame.hiddenNodes?.includes(id));
    const map = $('#story-map'); map.replaceChildren(); map.dataset.mode = privatePath ? 'private' : 'public'; map.dataset.narrow = String(narrow.matches);
    const contextDiagram = !privatePath && !!frame.diagramEdges;
    map.dataset.context = String(contextDiagram);
    const width = narrow.matches ? 320 : 1100, height = narrow.matches ? contextDiagram ? 640 : 470 : privatePath ? 240 : 260;
    const points = new Map();
    if (narrow.matches) {
      if (privatePath) { points.set(author, [160, 45]); points.set('relay-a', [77, 220]); points.set('relay-b', [243, 220]); points.set(receiver, [160, 410]); }
      else { mainIds.forEach((id, i) => points.set(id, [160, contextDiagram ? 65 + i * 165 : 45 + i * 125])); points.set('registry-a', [45, 48]); points.set('registry-b', [275, 423]); }
    } else {
      if (privatePath) { points.set(author, [105, 120]); points.set('relay-a', [550, 53]); points.set('relay-b', [550, 192]); points.set(receiver, [995, 120]); }
      else { mainIds.forEach((id, i) => points.set(id, [105 + i * (890 / 3), 176])); points.set('registry-a', [253, 38]); points.set('registry-b', [847, 38]); }
    }
    const svg = svgNode('svg', { viewBox: `0 0 ${width} ${height}`, 'aria-hidden': 'true', preserveAspectRatio: 'none' });
    const defs = svgNode('defs', {}), marker = svgNode('marker', { id: 'story-arrow', viewBox: '0 0 10 10', refX: 9, refY: 5, markerWidth: 5, markerHeight: 5, orient: 'auto' });
    marker.append(svgNode('path', { d: 'M 0 0 L 10 5 L 0 10 z', fill: 'currentColor' })); defs.append(marker); svg.append(defs); map.append(svg);
    // An explanatory starting point must not inherit connections observed later in setup.
    const routes = (frame.diagramEdges || flow?.edges || []).filter(edge => ids.includes(edge.from) && ids.includes(edge.to)).map(edge => ({ ...edge }));
    if (frame.route && !routes.some(edge => edge.from === frame.route.from && edge.to === frame.route.to)) routes.push({ id: 'current-setup', ...frame.route, kind: 'discovery' });
    const routeKey = edge => `${edge.from}-${edge.to}`;
    const seen = new Set();
    for (const edge of routes) {
      if (seen.has(routeKey(edge))) continue; seen.add(routeKey(edge));
      const from = points.get(edge.from), to = points.get(edge.to), isActive = frame.activeEdges?.includes(edge.id) || frame.route?.from === edge.from && frame.route?.to === edge.to;
      let sx = from[0], sy = from[1], ex = to[0], ey = to[1];
      if (narrow.matches) { sy += Math.sign(to[1] - from[1]) * 35; ey -= Math.sign(to[1] - from[1]) * 35; }
      else { sx += Math.sign(to[0] - from[0]) * 85; ex -= Math.sign(to[0] - from[0]) * 85; }
      const middle = narrow.matches ? (sy + ey) / 2 : (sx + ex) / 2;
      const d = narrow.matches ? `M ${sx},${sy} C ${sx},${middle} ${ex},${middle} ${ex},${ey}` : `M ${sx},${sy} C ${middle},${sy} ${middle},${ey} ${ex},${ey}`;
      const path = svgNode('path', { d, class: 'story-route', 'data-active': isActive, 'data-kind': edge.kind, 'data-edge': edge.id, 'marker-end': 'url(#story-arrow)' }); svg.append(path);
      if (isActive && frame.route) {
        const dot = svgNode('circle', { r: 8, cx: sx, cy: sy, class: 'story-marker' }); svg.append(dot);
        const markerLabel = svgNode('text', { x: sx + 13, y: sy - 12, class: 'story-marker-label' });
        markerLabel.textContent = frame.routeLabel || (frame.kind === 'setup' ? frame.id === 'private-invitation' ? 'Invitation' : frame.id === 'receiver-request' ? 'Read request' : 'Connection' : frame.artifact === 'encrypted' ? journey?.objects?.[0]?.alias || 'E1' : 'P1'); svg.append(markerLabel);
        const moveMarker = (x, y) => { const labelLeft = x > width * .65; dot.setAttribute('cx', x); dot.setAttribute('cy', y); markerLabel.setAttribute('x', x + (labelLeft ? -13 : 13)); markerLabel.setAttribute('y', y - 12); markerLabel.setAttribute('text-anchor', labelLeft ? 'end' : 'start'); };
        if (reducedMotion.matches || lastFrame === frame.id) moveMarker(ex, ey);
        else {
          let started;
          const length = path.getTotalLength();
          const travel = now => { started ??= now; const progress = Math.min((now - started) / 1600, 1), point = path.getPointAtLength(length * progress); moveMarker(point.x, point.y); if (progress < 1) animation = requestAnimationFrame(travel); };
          animation = requestAnimationFrame(travel);
        }
      }
    }
    const completedFrames = story.slice(0, index + 1);
    for (const id of ids) {
      const point = points.get(id), registry = id.startsWith('registry-'), active = frame.activeNodes?.includes(id);
      const card = node('div', 'story-device'); card.dataset.node = id; card.dataset.active = String(!!active); card.dataset.registry = String(registry);
      card.style.left = `${point[0] / width * 100}%`; card.style.top = `${point[1] / height * 100}%`;
      card.append(node('strong', '', labels[id] || id));
      const connection = journey?.setup?.connection || state?.connections?.[visibility];
      let status = registry ? 'Find public carriers' : id.startsWith('relay-') ? 'Encrypted storage' : id.startsWith('node-') ? 'Public records' : connection?.sameCommunity ? 'Community member' : 'Member’s device';
      if (id === author && frame.phase !== 'connect') status = 'Has the original text';
      if (id === receiver && completedFrames.some(item => item.id === 'receiver-download' && item.evidence === 'observed')) status = 'Has encrypted bytes';
      if (id === receiver && frame.phase === 'open' && frame.kind !== 'read') status = frame.evidence === 'unknown' || journey?.failed ? 'Result not verified' : 'Opening on this device';
      if (id === receiver && frame.kind === 'read' && frame.evidence === 'observed') status = 'Can read the message';
      card.append(node('span', '', frame.nodeNotes?.[id] || status));
      if (active && !registry) card.append(node('small', 'story-here', frame.evidence === 'unknown' || journey?.failed ? 'Not verified' : frame.kind === 'setup' ? frame.phase === 'connect' ? 'Connection setup' : 'Read request' : frame.route ? id === frame.route.from ? 'Sending' : id === frame.route.to ? 'Receiving' : 'Active' : 'Message here'));
      map.append(card);
    }
    lastFrame = frame.id;
  }
  function draw() {
    const story = frames(); if (!story.length) return;
    index = Math.min(index, story.length - 1);
    const frame = story[index], state = getState(), journey = state?.journey;
    host.dataset.frame = frame.id; host.dataset.playing = String(playing); host.dataset.running = String(running); host.dataset.phase = frame.phase;
    host.dataset.reducedMotion = String(reducedMotion.matches);
    put('#story-count', `Step ${index + 1} of ${story.length} · ${frame.phase === 'connect' ? 'Connection setup' : 'Follow the message'}`);
    put('#story-title', frame.title); put('#story-evidence', frame.phase === 'connect' && frame.evidence === 'explained' ? 'Illustrated explanation' : evidenceLabels[frame.evidence] || frame.evidence); $('#story-evidence').dataset.evidence = frame.evidence;
    const phaseOrder = ['connect', 'protect', 'send', 'open'];
    for (const item of host.querySelectorAll('.story-phases li')) { item.dataset.active = String(item.dataset.phase === frame.phase); item.dataset.done = String(phaseOrder.indexOf(item.dataset.phase) < phaseOrder.indexOf(frame.phase)); }
    const connection = journey?.setup?.connection || state?.connections?.[journey?.visibility || getScenario()];
    put('#story-community-name', frame.communityLabel || 'Shared community · ' + (journey?.community?.name || connection?.name || state?.communities?.[getScenario()]?.name || 'Preparing'));
    put('#story-members', frame.membershipLabel || (connection?.sameCommunity ? 'Alice and Bob · same community ID' : 'Membership is checked during setup'));
    put('#story-location', frame.location); put('#story-route-note', frame.route ? `${labels[frame.route.from] || frame.route.from} → ${labels[frame.route.to] || frame.route.to}` : 'Inside the highlighted device');
    put('#story-description', frame.detail); put('#story-source', frame.source);
    $('#story-input').replaceChildren(); $('#story-output').replaceChildren();
    appendText($('#story-input'), frame.input);
    inputContext(frame, story, $('#story-input'));
    if (frame.artifact !== 'recovered') appendText($('#story-output'), frame.output);
    if (frame.layers?.length && frame.artifact !== 'encrypted' && frame.artifact !== 'recovered') $('#story-output').append(layersView(frame.layers));
    artifactView(frame, $('#story-output'));
    cancelAnimationFrame(animation); drawMap(frame, story);
    $('#story-back').disabled = running || index === 0;
    $('#story-replay').disabled = running || !journey;
    $('#story-next').disabled = running || !readiness() || index === story.length - 1;
    $('#story-play').disabled = !playing && (running || !readiness() || index === story.length - 1);
    put('#story-play', playing ? 'Pause' : 'Play');
    const verifiedRead = frame.kind === 'read' && frame.evidence === 'observed' && !journey?.failed;
    put('#story-next', running ? 'Running operation…' : index === story.length - 1 ? verifiedRead ? (labels[journey?.receiver] || 'Receiver') + ' can read it' : 'Result not verified' : 'Next step →');
    put('#story-status', `Step ${index + 1} of ${story.length} · ` + (journey?.failed ? 'Stopped. Reset the demonstration to recover.' : journey?.recoverable ? 'Edit and prepare the message again.' : running ? 'Waiting for the real client…' : !journey ? 'Prepare the message above to begin.' : playing ? 'Playing · you can pause at any time' : index === story.length - 1 && verifiedRead ? 'Complete · replay without sending again' : 'Paused · advance at your own pace'));
  }
  function update() {
    const state = getState(), nextIdentity = `${state?.session || ''}/${state?.journey?.id || ''}/${getScenario()}`;
    if (identity !== nextIdentity) { identity = nextIdentity; index = 0; lastFrame = ''; stop({ render: false }); }
    if (state?.journey?.failed || state?.journey?.recoverable || state?.status === 'error' || !canRun() && !running && playing) stop({ render: false });
    draw();
  }
  $('#story-next').addEventListener('click', () => { stop({ render: false }); void advance(); });
  $('#story-back').addEventListener('click', () => { stop({ render: false }); index = Math.max(0, index - 1); draw(); });
  $('#story-replay').addEventListener('click', () => { stop({ render: false }); index = 0; draw(); });
  $('#story-play').addEventListener('click', () => { if (playing) stop(); else { playing = true; ++generation; void advance(); } });
  document.addEventListener('visibilitychange', () => { if (document.hidden) stop(); });
  reducedMotion.addEventListener('change', () => { cancelAnimationFrame(animation); draw(); });
  narrow.addEventListener('change', draw);
  return { update, stop, focus() { $('#story-title').focus({ preventScroll: true }); host.scrollIntoView({ block: 'start', behavior: 'auto' }); } };
}
