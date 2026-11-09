// A selected message's route, with transport observations kept distinct from inferred hops.
const labels = { alice: 'Alice', bob: 'Bob', 'node-a': 'Public node A', 'node-b': 'Public node B', 'relay-a': 'Private relay A', 'relay-b': 'Private relay B', 'registry-a': 'Registry A', 'registry-b': 'Registry B' };
const personNode = person => person === 'alice' ? 'node-a' : 'node-b';
const reached = (journey, stage) => ['draft', 'seal', 'store', 'download', 'open'].indexOf(journey?.stage) >= ['draft', 'seal', 'store', 'download', 'open'].indexOf(stage);

function node(id) {
  const kind = id.startsWith('node-') ? 'node' : id.startsWith('relay-') ? 'relay' : id.startsWith('registry-') ? 'registry' : 'person';
  return { id, label: labels[id], kind, detail: kind === 'person' ? 'Independent browser and Rust core.' : kind === 'node' ? 'Keeps and forwards readable public records.' : kind === 'relay' ? 'Keeps whole encrypted copies; has no group content keys.' : 'Answers public discovery requests; does not carry this post.' };
}

function uncertainAfterFailure(edge, journey) {
  if (!journey?.failed || edge.kind === 'discovery' || edge.kind === 'connection') return edge;
  if (edge.status === 'observed' || edge.status === 'inferred') return { ...edge, historical: true, evidence: [...edge.evidence, 'Captured before the failed operation; current delivery state is unverified.'] };
  return { ...edge, status: 'unknown', evidence: ['The operation may have partially completed. Reset before drawing conclusions about this hop.'] };
}

export function describeFlow({ visibility, journey = null, topology, discovery = [], availability = [] }) {
  const author = journey?.author || 'alice', receiver = author === 'alice' ? 'bob' : 'alice';
  const privatePath = visibility === 'private';
  const ids = privatePath ? [author, 'relay-a', 'relay-b', receiver] : [author, personNode(author), personNode(receiver), receiver, 'registry-a', 'registry-b'];
  const flow = { mode: privatePath ? 'private-replicas' : 'public-chain', nodes: ids.map(node), edges: [], discovery: [] };
  if (privatePath) {
    const objects = journey?.objects || [];
    const aliases = objects.map(object => object.alias);
    for (const service of ['relay-a', 'relay-b']) {
      const probe = availability.find(item => item.service === service && item.actor === author);
      flow.discovery.push({ id: `${service}-availability`, actor: author, source: 'Sandbox relay endpoint configuration, followed by an actual metadata request.',
        selected: [service], available: probe?.status === 200 ? [service] : [], status: probe?.status === 200 ? 'observed' : 'configured',
        evidence: probe ? [`${labels[author]} requested ${probe.path}; HTTP ${probe.status}, relay API ${probe.apiVersion}.`] : ['This endpoint is configured; no availability response has been captured for this actor.'] });
      flow.edges.push({ id: `${author}-${service}-availability`, from: author, to: service, kind: 'discovery', stage: 'setup', status: probe?.status === 200 ? 'observed' : 'configured',
        label: 'Check configured relay', input: ['Configured relay address'], action: 'GET /relay/v2/meta', output: probe ? [`HTTP ${probe.status} · API ${probe.apiVersion}`] : ['Availability not checked'],
        artifactIds: [], evidence: flow.discovery.at(-1).evidence, transport: 'Local HTTP' });
      const copies = objects.map(object => ({ object, stored: journey?.stored?.find(copy => copy.service === service && copy.objectId === object.id),
        upload: journey?.uploads?.find(upload => upload.service === service && upload.objectId === object.id) }));
      const stored = reached(journey, 'store') && copies.length > 0 && copies.every(({ object, stored, upload }) => stored?.identical && upload?.sha256 === object.id && upload?.receipt?.id === object.id && upload.receipt.size === object.size && upload.receipt.expires === object.expires && upload.status >= 200 && upload.status < 300);
      flow.edges.push({ id: `${author}-${service}-store`, from: author, to: service, kind: 'delivery', stage: 'store', status: stored ? 'observed' : 'pending',
        label: 'Upload encrypted copy', input: aliases, action: 'The production relay client uploads each complete encrypted package and checks its receipt.', output: stored ? aliases.map(alias => `${alias} stored copy`) : [], artifactIds: aliases,
        evidence: stored ? copies.map(({ object, upload }) => `${object.alias}: PUT HTTP ${upload.status}; receipt matches ${object.size} bytes; stored bytes equal the sender’s complete bytes.`) : ['No matching upload receipt and stored copy have been captured for this step.'], transport: 'Local HTTP',
        ...(stored ? { bytes: copies.reduce((size, { object }) => size + object.size, 0), sameBytes: true } : {}) });
    }
    const downloadService = journey?.downloads?.[0]?.service || 'relay-a';
    const downloaded = reached(journey, 'download') && objects.length > 0 && objects.every(object => journey?.downloads?.some(copy => copy.service === downloadService && copy.objectId === object.id && copy.sha256 === object.id && copy.identical && copy.status === 200 && copy.readProofUsed));
    flow.edges.push({ id: `${downloadService}-${receiver}-download`, from: downloadService, to: receiver, kind: 'delivery', stage: 'download', status: downloaded ? 'observed' : 'pending',
      label: 'Download encrypted copy', input: aliases.map(alias => `${alias} stored copy`), action: `${labels[receiver]} asks the core for a signed read proof, then requests the selected bytes.`,
      output: downloaded ? aliases.map(alias => `${alias} downloaded`) : [], artifactIds: aliases, transport: 'Local HTTP',
      evidence: downloaded ? journey.downloads.map(copy => `${copy.objectAlias}: GET HTTP ${copy.status}; ${copy.size} bytes; complete bytes and SHA-256 match the sender. Read proof used; its value is not exported.`) : ['No selected ciphertext has been downloaded in this step.'],
      ...(downloaded ? { bytes: journey.downloads.reduce((sum, copy) => sum + copy.size, 0), sameBytes: true } : {}) });
  } else {
    for (const lookup of discovery.slice(0, 2)) {
      const item = { id: lookup.id, actor: lookup.actor, source: 'The production registry client’s nodesFor lookup during setup.', selected: lookup.selected, available: lookup.available, status: 'observed',
        evidence: [`GET ${lookup.path} returned HTTP ${lookup.status}.`, `Available local routes: ${lookup.available.map(id => labels[id]).join(', ')}.`, `This demonstration selects ${lookup.selected.map(id => labels[id]).join(', ')} and blocks other browser routes.`] };
      flow.discovery.push(item);
      flow.edges.push({ id: lookup.id, from: lookup.actor, to: lookup.service, kind: 'discovery', stage: 'setup', status: 'observed', label: 'Find an available public node',
        input: ['Public community ID'], action: `Ask ${labels[lookup.service]} for public nodes carrying this community.`, output: lookup.available.map(id => labels[id]), artifactIds: [], evidence: item.evidence, transport: 'Local HTTP' });
    }
    const fromNode = personNode(author), toNode = personNode(receiver);
    const hasCopy = id => journey?.stored?.some(copy => copy.service === id && copy.matchingPost);
    const linked = person => topology?.clients?.some(client => client.id === person && client.node === personNode(person) && client.communityConnected);
    const inferred = Boolean(journey?.post && hasCopy(fromNode) && hasCopy(toNode) && topology?.enforced && topology?.nativeEntry?.configured && linked(author) && linked(receiver));
    const path = [[author, fromNode], [fromNode, toNode], [toNode, receiver]];
    for (const [index, [from, to]] of path.entries()) {
      const observedEndpoints = index === 0 ? journey?.post && hasCopy(fromNode) : index === 1 ? inferred : hasCopy(toNode) && journey?.recovered?.samePost && journey.recovered.sameText && journey.recovered.sameTitle;
      const proved = Boolean(inferred && observedEndpoints);
      const client = index === 0 ? author : receiver;
      flow.edges.push({ id: `${from}-${to}-public`, from, to, kind: journey ? 'delivery' : 'connection', stage: journey ? index === 2 ? 'open' : 'store' : 'setup',
        status: journey ? proved ? 'inferred' : 'pending' : index === 1 ? topology?.nativeEntry?.configured ? 'configured' : 'pending' : linked(client) ? 'observed' : 'pending',
        label: index === 1 ? 'Forward between public nodes' : index === 0 ? 'Send to the selected node' : 'Receive from the selected node',
        input: journey?.post ? ['P1'] : [], action: index === 1 ? 'The native nodes synchronize records through their configured peer connection.' : 'The browser synchronizes through its single allowed public node.',
        output: proved ? [index === 2 ? 'P1 readable on receiver' : 'P1 stored copy'] : [], artifactIds: journey?.post ? ['P1'] : [], transport: index === 1 ? 'Native QUIC · TLS 1.3' : 'WebRTC · DTLS',
        evidence: journey ? proved ? [
          index === 1 ? 'The selected post ID was observed in both native stores.' : index === 0 ? 'The sender returned this post; the selected carrier stores the same post ID.' : 'The selected carrier stores this post; the receiving core returned the same ID and text.',
          'The sandbox enforces Alice ↔ node A and Bob ↔ node B, with no browser shortcut.',
          'This hop is inferred from the enforced topology and endpoint evidence. Individual native packets were not captured.',
        ] : ['This selected message has not yet been verified at both ends of this hop.'] : index === 1 ? [topology?.nativeEntry?.source || 'The native entry route has not been configured.'] : linked(client) ? ['A completed WebRTC community connection was observed under the browser’s route allowlist. This alone is not a post receipt.'] : ['No completed community connection has been observed.'],
      });
    }
  }
  flow.edges = flow.edges.map(edge => uncertainAfterFailure(edge, journey));
  return flow;
}
