// A presentation sequence over captured sandbox evidence. Moving between frames never runs crypto.
const NAMES = { alice: 'Alice', bob: 'Bob', 'node-a': 'Public node A', 'node-b': 'Public node B', 'relay-a': 'Private relay A', 'relay-b': 'Private relay B', 'registry-a': 'Registry A', 'registry-b': 'Registry B' };
const STAGES = { draft: 0, seal: 1, store: 2, download: 3, open: 4 };
const name = id => NAMES[id] || id;
const carrier = id => id === 'alice' ? 'node-a' : 'node-b';
const nonempty = values => values.filter(value => typeof value === 'string' && value.length);
const captured = value => value === 'observed' || value === 'inferred' || value === 'explained';

function frame(id, phase, kind, title, detail, requiredStage, activeNodes, input, output, extra = {}) {
  return { id, phase, kind, title, detail, requiredStage, location: activeNodes.map(name).join(' → '), activeNodes, activeEdges: [],
    evidence: 'pending', source: 'This result has not been captured yet.', input: { label: 'Input', text: input }, output: { label: 'Output', text: output }, ...extra };
}

function available(journey, stage) {
  return !!journey && !journey.failed && (STAGES[journey.stage] ?? -1) >= (STAGES[stage] ?? 0);
}

function observedStatus(journey, requiredStage, evidence) {
  if (journey?.failed) return 'unknown';
  return available(journey, requiredStage) ? evidence || 'pending' : 'pending';
}

// Even an accidentally overcomplete fixture must not put later plaintext or byte ranges into
// an earlier frame. Only this operation's own measured/structural layers may be inspected.
function safeLayers(layers, allowed, waiting = 'pending') {
  return (layers || []).map(item => {
    const evidence = allowed ? item.evidence : waiting;
    const result = { id: item.id, label: item.label, evidence, readable: allowed ? item.readable : null };
    if (allowed) {
      for (const key of ['availableAt', 'bytes', 'sizeBasis']) if (item[key] !== undefined) result[key] = item[key];
      if (item.evidence === 'observed') for (const key of ['value', 'artifactAlias', 'fingerprint', 'byteRange']) if (item[key] !== undefined) result[key] = item[key];
    }
    if (item.children?.length) result.children = safeLayers(item.children, allowed, waiting);
    return result;
  });
}

function setupFrames(state, journey, visibility, flow) {
  const info = journey?.setup?.connection || state?.connections?.[visibility];
  const community = journey?.community || state?.communities?.[visibility];
  const communityName = info?.name || community?.name || (visibility === 'private' ? 'The private group' : 'The public community');
  const communityId = info?.communityId || community?.id;
  const clients = info?.clients || [];
  const same = info?.sameCommunity === true;
  const common = nonempty([communityName, same && communityId ? 'Same community ID on both devices: ' + communityId : 'Matching membership has not been verified yet.']);
  const setupSource = 'Captured setup replay. The community was prepared before this message; these frames do not run membership operations again.';
  if (visibility === 'private') {
    const active = same && ['alice', 'bob'].every(id => clients.some(client => client.id === id && client.active === true));
    const invitation = info?.invitation?.method === 'local-handoff' && info.invitation.completed === true;
    const probes = (flow.discovery || []).filter(item => item.status === 'observed');
    const relays = info?.relays || ['relay-a', 'relay-b'];
    const ready = active && info?.sameRoom === true && info?.sameEpoch === true;
    return [
      frame('private-community', 'connect', 'setup', 'Alice and Bob share one private community',
        'Membership is on their devices. A storage relay does not make someone a member.', null, ['alice', 'bob'],
        ['Alice creates a private community; Bob has an independent identity.'], active ? common : ['The private community is being prepared.'],
        { evidence: active ? 'observed' : 'pending', source: setupSource + (active ? ' Both cores report active membership in the same community.' : ' Both membership results are needed before this can be verified.') }),
      frame('private-invitation', 'connect', 'setup', 'Bob joins through Alice’s invitation',
        'Bob creates a join request. Alice’s core creates a Welcome and encrypted bootstrap; Bob’s core joins. This sandbox hands the invitation directly between the clients.', null,
        ['alice', 'bob'], ['Bob’s join request → Alice’s invitation'], invitation ? ['Bob joined the same community.', 'The invitation was handed over locally; this is not a relay transfer.', 'Each device keeps its own private group state.'] : ['The invitation exchange has not been verified.'],
        { route: { from: 'alice', to: 'bob' }, evidence: invitation ? 'observed' : 'pending', source: setupSource + (invitation ? ' The real privateJoin call accepted the invitation; invitation contents and secret state are not exported.' : ' No completed invitation handoff has been verified.') }),
      frame('private-relays', 'connect', 'setup', 'Both members use the same relay services',
        'Alice and Bob use the configured relays for this group. Bob’s matching room authority lets his core request ciphertext; his local group state is needed to open it.', null,
        ['alice', ...relays, 'bob'], ['Configured relay addresses + each member’s local group state'], ready ? [relays.map(name).join(' and '), 'Same community, current epoch and opaque relay room confirmed.', ...(probes.length ? ['Relay metadata requests confirmed availability.'] : ['No relay availability response has been captured.'])] : ['Shared relay room and current membership are not yet verified.'],
        { evidence: ready ? 'observed' : 'pending', source: setupSource + ' Only equality checks are shown; room authority and group keys stay inside the cores. ' + probes.flatMap(item => item.evidence || []).join(' ') }),
    ];
  }
  const publicFlow = state?.flows?.public || flow;
  const joined = same && ['alice', 'bob'].every(id => clients.some(client => client.id === id && client.joined === true));
  const separateConnections = [
    { id: 'existing-alice-node', from: 'alice', to: 'node-a', kind: 'connection' },
    { id: 'existing-bob-node', from: 'bob', to: 'node-b', kind: 'connection' },
  ];
  const frames = [
    frame('public-existing-connections', 'connect', 'setup', 'Alice and Bob may already know different nodes',
      'Alice may use node A for one community, and Bob may use node B for another. Those connections do not mean the nodes know their new shared community or each other.', null,
      ['alice', 'node-a', 'bob', 'node-b'], ['Alice’s earlier community → node A', 'Bob’s different earlier community → node B'],
      ['Knowing a node is separate from that node knowing a community.', 'A connection between node A and node B is not assumed.'],
      { evidence: 'explained', location: 'Two separate existing connections', diagramEdges: separateConnections, activeEdges: separateConnections.map(edge => edge.id), hiddenNodes: ['registry-a', 'registry-b'],
        communityLabel: 'Possible starting point · separate communities', membershipLabel: 'The new shared community is not yet assumed',
        nodeNotes: { alice: 'Knows A from another community', bob: 'Knows B from another community', 'node-a': 'May not know the shared community', 'node-b': 'May not know the shared community' },
        source: 'Possible scenario, not captured history. This explains how people can arrive through different nodes; the local run’s actual registry discovery and configured node connection follow.' }),
    frame('public-introduce-community', 'connect', 'setup', 'Bob can introduce the shared community to node B',
      'After Alice shares the public community and Bob joins, Bob can ask node B to keep it. The request reveals its signed definition to B. Node B’s policy decides whether to accept.', null,
      ['bob', 'node-b'], ['Bob has joined the shared public community.', 'Bob asks node B to keep it, supplying the signed community definition.'],
      ['B learns the public community ID and definition from the request.', 'If B accepts, it can store and serve the public records.', 'An invitation alone does not notify every node Bob knows or tell B who invited him.'],
      { evidence: 'explained', route: { from: 'bob', to: 'node-b' }, routeLabel: 'Carry request',
        diagramEdges: separateConnections, hiddenNodes: ['registry-a', 'registry-b'],
        communityLabel: 'Possible next step · a shared public community', membershipLabel: 'Bob joins, then asks node B to keep it',
        nodeNotes: { alice: 'Shares the public community', bob: 'Joined; asks B to keep it', 'node-a': 'No new connection assumed', 'node-b': 'Learns from the request' },
        source: 'Protocol explanation, not a request captured in this run. Public nodes can refuse carry requests. Private invitations never make public nodes carry the private group.' }),
    frame('public-community', 'connect', 'setup', 'Alice and Bob join the same public community',
    'The community ID is the same on both devices. The two members can connect through different nodes.', null,
    ['alice', 'bob'], ['One public community ID, chosen by both clients'], joined ? common : ['Matching public community membership has not been verified.'],
    { evidence: joined ? 'observed' : 'pending', source: setupSource })];
  for (const id of ['alice', 'bob']) {
    const lookup = (publicFlow.discovery || []).find(item => item.actor === id);
    const discoveryEdge = (publicFlow.edges || []).find(edge => edge.kind === 'discovery' && edge.from === id);
    const node = clients.find(client => client.id === id)?.node || carrier(id);
    const registry = discoveryEdge?.to || (id === 'alice' ? 'registry-a' : 'registry-b');
    const connected = (publicFlow.edges || []).some(edge => edge.kind === 'connection' && edge.status === 'observed' && ((edge.from === id && edge.to === node) || (edge.to === id && edge.from === node)));
    frames.push(frame('discover-' + id, 'connect', 'setup', name(id) + ' finds an available node',
      name(registry) + ' returns node routes carrying the chosen community. It does not receive the post.', null, [id, registry, node],
      ['The public community ID'], lookup?.status === 'observed' ? ['Available: ' + (lookup.available || []).map(name).join(', '), 'Selected: ' + (lookup.selected || []).map(name).join(', ')] : ['No discovery response captured.'],
      { route: { from: id, to: registry }, activeEdges: discoveryEdge ? [discoveryEdge.id] : [], evidence: lookup?.status === 'observed' ? 'observed' : 'pending', source: setupSource + ' ' + (lookup?.evidence || []).join(' ') }));
    frames.push(frame('connect-' + id, 'connect', 'setup', name(id) + ' connects to ' + name(node),
      'The browser establishes a WebRTC connection for this community. This demonstration allows only its selected node.', null, [id, node],
      ['Selected node route + community ID'], connected ? ['A completed community connection was observed.', name(id) + ' can synchronize this community through ' + name(node) + '.'] : ['A completed community connection has not been captured.'],
      { route: { from: id, to: node }, evidence: connected ? 'observed' : 'pending', source: setupSource + ' A connection is setup evidence, not a receipt for this message.' }));
  }
  const native = info?.nativeLink;
  frames.push(frame('connect-carriers', 'connect', 'setup', 'The two public nodes connect the community',
    'Bob does not need Alice’s node. Node B is configured with node A’s public link, so the nodes can synchronize the shared community.', null,
    ['node-b', 'node-a'], ['Node A’s public node link'], native?.configured ? ['Node B starts with node A as an entry node.', 'Alice → node A ↔ node B ← Bob'] : ['The native entry link has not been verified.'],
    { route: { from: 'node-b', to: 'node-a' }, evidence: native?.configured ? 'explained' : 'pending', source: setupSource + ' This is the configured native route. Individual native connection packets were not captured.' }));
  return frames;
}

function cryptoFrame(journey, side, id, fallback) {
  const author = journey?.author || 'alice', receiver = journey?.receiver || 'bob';
  const sender = side === 'sender', person = sender ? author : receiver, stage = sender ? 'seal' : 'open';
  const step = journey?.crypto?.[side]?.find(item => item.id === id);
  const evidence = observedStatus(journey, stage, step?.evidence);
  const hasOutput = captured(evidence);
  const result = frame((sender ? 'protect-' : 'open-') + id, sender ? 'protect' : 'open', id === 'readable' ? 'read' : 'crypto',
    step?.title?.replace(/^\d+\s*·\s*/, '') || fallback.title, step?.action || fallback.action, stage, [person],
    [step?.input || fallback.input], hasOutput ? [step?.output || fallback.output] : ['This operation’s result has not been verified.'],
    { side, cryptoStepId: id, evidence, source: evidence === 'unknown' ? 'The operation may have partially completed. Reset before continuing.'
      : 'Visual breakdown of one ' + (sender ? 'protect' : 'open') + ' core call. ' + (evidence === 'explained'
        ? 'This internal step is explained from core code; its intermediate bytes were not captured.'
        : evidence === 'observed' ? 'The result was captured at the client boundary. The individual inner operations were not paused.' : 'The next required core call has not completed.'),
      layers: safeLayers(step?.layers, hasOutput, evidence) });
  if (evidence === 'observed') {
    if (id === 'signed-post') {
      result.artifact = 'post';
      result.input.text = nonempty([journey.message?.title, journey.message?.text]);
      result.output.text = nonempty([journey.post?.title, journey.post?.text, 'Signed post P1 · signature check valid']);
    } else if (id === 'outer') result.artifact = 'encrypted';
    else if (id === 'readable' && journey.recovered?.samePost && journey.recovered?.sameText && journey.recovered?.sameTitle) {
      result.artifact = 'recovered';
      result.output.text = nonempty([journey.recovered.title, journey.recovered.text, 'Same title, text and signed post as Alice’s input.'.replace('Alice', name(author))]);
    }
  }
  return result;
}

function transferFrame(journey, edge, index, visibility) {
  const author = journey?.author || 'alice', receiver = journey?.receiver || 'bob';
  const privatePath = visibility === 'private';
  const downloading = privatePath && index === 2;
  const from = edge?.from || (privatePath ? downloading ? 'relay-a' : author : index === 0 ? author : index === 1 ? carrier(author) : carrier(receiver));
  const to = edge?.to || (privatePath ? downloading ? receiver : index === 0 ? 'relay-a' : 'relay-b' : index === 0 ? carrier(author) : index === 1 ? carrier(receiver) : receiver);
  const requiredStage = privatePath ? downloading ? 'download' : 'store' : 'open';
  const evidence = observedStatus(journey, requiredStage, edge?.status);
  const result = frame(privatePath ? downloading ? 'receiver-download' : 'upload-' + to : 'public-hop-' + index, 'send', 'transfer',
    privatePath ? downloading ? name(receiver) + ' downloads the encrypted copy' : 'Upload a complete copy to ' + name(to) : name(from) + ' → ' + name(to),
    edge?.action || (privatePath ? 'Transfer one complete encrypted package.' : 'Synchronize the signed public post through the selected route.'), requiredStage, [from, to],
    edge?.input?.length ? edge.input : [privatePath ? 'Whole encrypted package' : 'Signed public post P1'],
    captured(evidence) ? edge.output : ['This transfer has not been verified.'],
    { route: { from, to }, activeEdges: edge ? [edge.id] : [], ...(edge ? { edgeId: edge.id } : {}), evidence,
      source: evidence === 'unknown' ? 'Current delivery state is unverified after a failed operation.' : (privatePath
        ? downloading ? 'This frame shows the selected download receipt and complete byte comparison. ' : 'The store action uploads both copies; this animation reviews each captured transfer in turn. '
        : 'Recorded public delivery. Route timing is illustrative; the public network runs automatically. ') + (edge?.evidence || []).join(' ') });
  if (captured(evidence)) result.artifact = privatePath ? 'encrypted' : 'post';
  return result;
}

/** Build a deterministic presentation. It has no timers, network calls or cryptographic operations. */
export function buildStory(state, scenario = 'private') {
  const journey = state?.journey || null;
  const visibility = journey?.visibility || scenario;
  const flow = journey?.flow || state?.flows?.[visibility] || { nodes: [], edges: [], discovery: [] };
  const author = journey?.author || 'alice', receiver = journey?.receiver || 'bob';
  const frames = setupFrames(state, journey, visibility, flow);
  const inputReady = !!journey?.message;
  frames.push(frame('message-input', 'protect', 'input', name(author) + ' writes the message',
    'The title and message begin as readable text on the sender’s device.', 'draft', [author], ['A title and message'],
    inputReady ? nonempty([journey.message.title, journey.message.text]) : ['Prepare a message to begin.'],
    { evidence: inputReady ? 'observed' : 'pending', source: 'This is the original local input. No transmission is performed by displaying it.', ...(inputReady ? { artifact: 'message' } : {}) }));
  if (visibility === 'private') {
    const senderSteps = [
      ['signed-post', { title: 'Create and sign a post', action: 'Encode the post and sign its ID inside the core.', input: 'Readable message', output: 'Signed post P1' }],
      ['mls', { title: 'Encrypt for group members', action: 'The core creates an MLS private message.', input: 'Signed records', output: 'MLS private message' }],
      ['padding', { title: 'Hide the exact message length', action: 'Add the length field and zero padding.', input: 'MLS private message', output: 'Padded content' }],
      ['outer', { title: 'Seal the delivery package', action: 'Apply the outer XChaCha20-Poly1305 protection.', input: 'Padded content', output: 'Nonce + ciphertext + authentication tag' }],
    ];
    for (const [id, fallback] of senderSteps) frames.push(cryptoFrame(journey, 'sender', id, fallback));
    const deliveries = (flow.edges || []).filter(edge => edge.kind === 'delivery');
    for (const [index, relay] of ['relay-a', 'relay-b'].entries()) frames.push(transferFrame(journey, deliveries.find(edge => edge.from === author && edge.to === relay), index, visibility));
    const download = deliveries.find(edge => edge.to === receiver);
    const proofCaptured = available(journey, 'download') && journey.downloads?.length > 0 && journey.downloads.every(item => item.readProofUsed === true);
    const source = download?.from || 'relay-a';
    frames.push(frame('receiver-request', 'send', 'setup', name(receiver) + ' requests the group’s encrypted object',
      'The receiving core uses its local room authority to sign a read request. The proof permits downloading ciphertext; it is not a decryption key.', 'download', [receiver, source],
      ['Object ID + local room authority'], proofCaptured ? ['A signed read request was used for this download.', 'The proof’s value is kept out of the walkthrough.'] : ['The selected read request has not been verified.'],
      { route: { from: receiver, to: source }, evidence: journey?.failed ? 'unknown' : proofCaptured ? 'observed' : 'pending', source: 'The download action makes this request and receives its response in one operation. This frame reviews the request before the response.' }));
    frames.push(transferFrame(journey, download, 2, visibility));
    const receiverSteps = [
      ['outer-open', { title: 'Authenticate and open the outer seal', action: 'Check the authentication tag and open the outer encryption.', input: 'Nonce + ciphertext + tag, with local group state', output: 'Authenticated padded content' }],
      ['unpad', { title: 'Check and remove the padding', action: 'Check the hidden length and zero padding.', input: 'Padded content', output: 'MLS private message' }],
      ['mls-open', { title: 'Open the group message', action: 'MLS checks the sender and opens the payload.', input: 'MLS private message + local membership state', output: 'Decoded signed records' }],
      ['validate', { title: 'Validate the signed post', action: 'Check signatures, identity and community rules.', input: 'Decoded signed records', output: 'Validated post P1' }],
      ['readable', { title: name(receiver) + ' can read the message', action: 'Return the verified post to the app.', input: 'Validated post P1', output: 'Original title and message' }],
    ];
    for (const [id, fallback] of receiverSteps) frames.push(cryptoFrame(journey, 'receiver', id, fallback));
  } else {
    const signed = available(journey, 'open') && journey.post?.checks?.signature === 'valid';
    frames.push(frame('public-sign', 'protect', 'crypto', name(author) + ' signs the public post',
      'Signing proves authorship and integrity. Public post contents remain readable.', 'open', [author],
      nonempty([journey?.message?.title, journey?.message?.text]), signed ? nonempty([journey.post.title, journey.post.text, 'Signed post P1']) : ['The signed post has not been verified.'],
      { evidence: journey?.failed ? 'unknown' : signed ? 'observed' : 'pending', source: 'Public creation and delivery run together. The following animation reviews their captured results; it does not pause individual packets.', ...(signed ? { artifact: 'post' } : {}) }));
    const deliveries = (flow.edges || []).filter(edge => edge.kind === 'delivery');
    const route = [[author, carrier(author)], [carrier(author), carrier(receiver)], [carrier(receiver), receiver]];
    route.forEach(([from, to], index) => frames.push(transferFrame(journey, deliveries.find(edge => edge.from === from && edge.to === to), index, visibility)));
    const recovered = available(journey, 'open') && journey.recovered?.samePost && journey.recovered?.sameTitle && journey.recovered?.sameText && journey.recovered?.checks?.signature === 'valid';
    frames.push(frame('public-read', 'open', 'read', name(receiver) + ' checks and reads the message',
      'The receiving core validates the public signed post. Private-group decryption is not needed.', 'open', [receiver], ['Signed public post P1'],
      recovered ? nonempty([journey.recovered.title, journey.recovered.text, 'Same signed post, title and message.']) : ['The receiving post has not been verified.'],
      { evidence: journey?.failed ? 'unknown' : recovered ? 'observed' : 'pending', source: 'The receiving core’s returned post and validation checks are measured.', ...(recovered ? { artifact: 'recovered' } : {}) }));
  }
  return frames;
}
