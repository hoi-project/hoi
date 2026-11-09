// Readable evidence for the local walkthrough. Cryptography remains in the production core.
import { createHash } from 'node:crypto';
import { describeCryptoJourney } from './walkthrough-crypto-view.mjs';

const names = { alice: 'Alice', bob: 'Bob' };
const PRIVATE_STAGES = ['draft', 'seal', 'store', 'download', 'open'];

export function cipherArtifact(object, index = 0) {
  const bytes = Buffer.from(object.body, 'base64url');
  const hash = createHash('sha256').update(bytes).digest('hex');
  if (bytes.length < 4096 || bytes.length > 65536 || bytes.length !== object.size || hash !== object.id) {
    throw new Error('The core did not return a bounded, matching encrypted object for this text walkthrough.');
  }
  return { alias: `E${index + 1}`, from: 'P1', id: object.id, room: object.room, size: bytes.length, expires: object.expires,
    hex: bytes.toString('hex'), hashAlgorithm: 'SHA-256',
    segments: [
      { name: 'Nonce', offset: 0, bytes: 24, hex: bytes.subarray(0, 24).toString('hex') },
      { name: 'Encrypted bytes', offset: 24, bytes: bytes.length - 40, hex: bytes.subarray(24, -16).toString('hex') },
      { name: 'Authentication tag', offset: bytes.length - 16, bytes: 16, hex: bytes.subarray(-16).toString('hex') },
    ] };
}

export function newJourney({ id, value, community, setup }) {
  return { id, mode: value.visibility === 'private' ? 'live' : 'recorded', visibility: value.visibility,
    author: value.author, receiver: value.author === 'alice' ? 'bob' : 'alice', community, stage: 'draft', complete: false,
    message: { alias: 'M1', title: value.title, text: value.text, utf8Bytes: Buffer.byteLength(value.text) },
    objects: [], stored: [], downloads: [], stages: [], permissions: [], setup };
}

function permissions(journey, stage) {
  const privatePath = journey.visibility === 'private';
  const position = PRIVATE_STAGES.indexOf(stage);
  const sender = { id: journey.author, label: names[journey.author], state: 'readable', detail: 'Keeps the original title and message on this device.' };
  const recipient = { id: journey.receiver, label: names[journey.receiver],
    state: position >= 4 ? 'readable' : privatePath && position >= 3 ? 'ciphertext' : 'none',
    detail: position >= 4 ? 'The core returned the original message and verified signed post.' : privatePath && position >= 3 ? 'Has the downloaded encrypted bytes. The core has not opened them yet.' : 'Has not received this message in this step.' };
  const serviceIds = privatePath ? ['relay-a', 'relay-b'] : ['node-a', 'node-b'];
  return [sender, ...serviceIds.map((id, index) => ({ id, label: `${privatePath ? 'Private relay' : 'Public node'} ${index ? 'B' : 'A'}`,
    state: position >= 2 ? privatePath ? 'ciphertext' : 'readable' : 'none',
    detail: position < 2 ? 'Has no stored copy of this message yet.' : privatePath ? 'Stores the encrypted package and delivery metadata. Has no group decryption keys.' : 'Stores a readable signed public record.' })), recipient];
}

/** Build presentation from captured evidence, never from an assumed network route. */
export function describeJourney(journey) {
  if (!journey) return null;
  const privatePath = journey.visibility === 'private';
  const ids = privatePath ? PRIVATE_STAGES : ['draft', 'seal', 'store', 'open'];
  const index = ids.indexOf(journey.stage);
  const receiver = names[journey.receiver];
  const objects = journey.objects.map(object => object.alias);
  const definitions = {
    draft: { title: 'Write a message', input: [], output: ['M1'], explanation: `${names[journey.author]} writes a title and message. No new post has been published yet.`,
      observations: ['The exact title and message are saved as the input to this demonstration.'], internals: [] },
    seal: { title: privatePath ? 'Sign and encrypt' : 'Sign and publish', input: ['M1'], output: privatePath ? ['P1', ...objects] : ['P1'],
      explanation: privatePath ? 'The core creates a signed post, then packs it into encrypted delivery bytes. Signing proves who authored it; encryption hides its contents.' : 'The core creates a signed public post. Signing provides authenticity; its text stays readable.',
      observations: journey.post ? [`The core returned post ${journey.post.alias} with a valid signature check.`, ...(privatePath ? [`The core returned ${objects.length} new encrypted package(s), each with a hash of its complete bytes.`] : [])] : [],
      internals: privatePath ? [
        'Title and text → encoded, signed post. The signature allows the receiver to check authorship and integrity.',
        'Signed records → MLS private message. MLS is the group-messaging layer that protects the message for the current members.',
        'MLS message → four-byte length, message bytes and zero padding. The exact inner length stays hidden.',
        'Padded message → XChaCha20-Poly1305 encrypted package: a 24-byte nonce, encrypted bytes and a 16-byte authentication tag.',
        'These inner operations are explained from the core code. Only the resulting post view and encrypted package are captured; intermediate buffers and secret keys are not exported.',
      ] : ['The public client automatically starts delivery after creating the signed post. Later panels review the resulting evidence, not paused network hops.'] },
    store: { title: privatePath ? 'Store encrypted copies' : 'Observe public copies', input: privatePath ? objects : ['P1'], output: privatePath ? objects.map(alias => `${alias} copies`) : ['P1 copies'],
      explanation: privatePath ? 'The same complete encrypted package is copied to two relays. These are replicas, not pieces to join together.' : 'Both carrier stores are inspected for the same signed public post. They can read its contents.',
      observations: journey.stored.map(copy => privatePath
        ? `${copy.label}: ${copy.objectAlias} ${copy.identical ? 'matches the original bytes' : 'has not been confirmed identical'}.`
        : `${copy.label}: ${copy.matchingPost ? 'the same post ID is present in its readable record store' : 'the selected post has not been confirmed'}.`),
      internals: privatePath ? ['Relays check upload proofs and object hashes. Their storage never receives the client’s group decryption keys.'] : [] },
    download: { title: `Download as ${receiver}`, input: objects.map(alias => `${alias} copy`), output: objects.map(alias => `${alias} downloaded`),
      explanation: `${receiver} requests an encrypted package from a relay. The downloaded bytes are held in that browser; the core has not decrypted them yet.`,
      observations: journey.downloads.map(copy => `${copy.objectAlias}: downloaded ${copy.size} bytes from ${copy.label}; ${copy.identical ? 'same complete bytes and hash' : 'match not confirmed'}.`),
      internals: ['The core signs a relay read request using the group’s room authority. That proof allows fetching ciphertext; it is not a decryption key.'] },
    open: { title: privatePath ? `Decrypt and read as ${receiver}` : `Read as ${receiver}`, input: privatePath ? objects.map(alias => `${alias} downloaded`) : ['P1 copies'], output: ['P1 recovered'],
      explanation: privatePath ? `${receiver} gives the downloaded ciphertext to the Rust core. With this member’s local group state, it opens and checks the message, then returns the original post.` : `${receiver}'s core receives and validates the public post. No private-group decryption is needed.`,
      observations: journey.recovered ? [
        `Post ID: ${journey.recovered.samePost ? 'matches' : 'does not match'} the author’s post.`,
        `Title: ${journey.recovered.sameTitle ? 'matches' : 'does not match'} the input. Message: ${journey.recovered.sameText ? 'matches' : 'does not match'} the input.`,
        `Receiver signature check: ${journey.recovered.checks.signature}.`,
      ] : [],
      internals: privatePath ? [
        'Encrypted package + local group state → authenticated padded bytes. XChaCha20-Poly1305 checks the tag while opening the outer encryption.',
        'Padded bytes → MLS message. The core checks the hidden length and zero padding, then removes them.',
        'MLS message + local membership state → decoded signed records. The core processes the group message and checks its sender.',
        'Signed records → readable post. Signature, identity and community validation run before the post is returned.',
        'These inner operations are explained from the core code; the recovered post below is measured. This example opens whole encrypted packages, not scattered pieces.',
      ] : [] },
  };
  journey.stages = ids.map((id, step) => ({ id, ...definitions[id], status: step <= index ? 'complete' : 'pending',
    permissions: permissions(journey, !privatePath && journey.complete && id !== 'draft' ? 'open' : id),
    ...(step > index ? { observations: [], permissions: [] } : {}) }));
  journey.permissions = permissions(journey, journey.stage);
  if (journey.failed) journey.permissions = journey.permissions.map(item => item.id === journey.author ? item : {
    ...item, state: 'unknown', detail: 'The step may have partially completed. Reset before drawing conclusions about this copy.',
  });
  journey.nextAction = journey.complete || journey.failed ? null : privatePath ? {
    draft: 'Sign and encrypt', seal: 'Store encrypted copies', store: `Download as ${receiver}`, download: `Decrypt and read as ${receiver}`,
  }[journey.stage] : 'Publish and record delivery';
  journey.crypto = describeCryptoJourney(journey);
  return journey;
}
