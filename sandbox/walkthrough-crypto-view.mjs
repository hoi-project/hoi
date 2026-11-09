// Packet structure for the local observer. Only existing public core outputs are measured;
// internal MLS, record encoding and padding remain an explicitly explained model.
import { createHash } from 'node:crypto';

const STAGES = ['draft', 'seal', 'store', 'download', 'open'];
const MAX_CIPHER_BYTES = 65536;

function packetSource(value, source) {
  const alias = value?.alias ?? value?.objectAlias;
  const id = value?.id ?? value?.objectId;
  if (!/^E[1-8]$/.test(alias) || !/^[a-f0-9]{64}$/.test(id)
    || !Number.isSafeInteger(value.size) || value.size < 4096 || value.size > MAX_CIPHER_BYTES
    || typeof value.hex !== 'string' || value.hex.length !== value.size * 2 || !/^[a-f0-9]+$/.test(value.hex)) return null;
  const bytes = Buffer.from(value.hex, 'hex');
  if (createHash('sha256').update(bytes).digest('hex') !== id) return null;
  return { alias, id, size: bytes.length, source, availableAt: source === 'sender' ? 'seal' : 'download' };
}

function layer(id, label, evidence, readable, availableAt, extra = {}) {
  return { id, label, evidence, readable: ['pending', 'unknown'].includes(evidence) ? null : readable, availableAt, ...extra };
}

function recordModel(evidence, availableAt, readable = true) {
  return layer('signed-record', 'Signed record · internal structure', evidence, readable, availableAt, { children: [
    layer('record-body', 'Post fields and author / community references', evidence, readable, availableAt),
    layer('record-signature', 'Ed25519 signature · authenticity, not secrecy', evidence, readable, availableAt),
  ] });
}

function mlsModel(evidence, availableAt) {
  return layer('mls', 'MLS private message', evidence, false, availableAt, { children: [
    layer('mls-header', 'MLS header · group, epoch and message type', evidence, false, availableAt),
    layer('mls-ciphertext', 'Encrypted signed records and supporting records', evidence, false, availableAt),
  ] });
}

function paddingModel(evidence, availableAt, size) {
  return layer('padding', 'Padded content · internal structure', evidence, false, availableAt, {
    ...(size === undefined ? {} : { bytes: size - 40, sizeBasis: 'Derived from the outer size minus its 24-byte nonce and 16-byte tag.' }),
    children: [
      layer('length', 'MLS length · 4-byte big-endian field; value not captured', evidence, false, availableAt, { bytes: 4 }),
      mlsModel(evidence, availableAt),
      layer('zeros', 'Zero padding · length not captured', evidence, false, availableAt),
    ],
  });
}

function packet(source) {
  const { alias, id, size, availableAt } = source;
  const range = (offset, length) => ({ artifactAlias: alias, source: source.source, offset, length });
  return layer(`${alias}-packet`, `${alias} · whole encrypted package`, 'observed', false, availableAt, {
    bytes: size, artifactAlias: alias, fingerprint: id,
    children: [
      layer(`${alias}-nonce`, 'Nonce · fresh random bytes', 'observed', false, availableAt, { bytes: 24, byteRange: range(0, 24) }),
      layer(`${alias}-ciphertext`, 'Ciphertext · padded MLS content is hidden here', 'observed', false, availableAt, {
        bytes: size - 40, byteRange: range(24, size - 40), children: [paddingModel('explained', availableAt, size)],
      }),
      layer(`${alias}-tag`, 'Authentication tag · detects changed ciphertext', 'observed', false, availableAt, { bytes: 16, byteRange: range(size - 16, 16) }),
    ],
  });
}

function plaintext(message, evidence, availableAt, alias) {
  return layer(`${alias}-plaintext`, `${alias} · readable title and message`, evidence, true, availableAt, { artifactAlias: alias, children: [
    layer(`${alias}-title`, 'Title', evidence, true, availableAt, { value: message.title, bytes: Buffer.byteLength(message.title) }),
    layer(`${alias}-text`, 'Message', evidence, true, availableAt, { value: message.text, bytes: Buffer.byteLength(message.text) }),
  ] });
}

function step(id, title, input, action, output, evidence, readable, availableAt, layers, reference) {
  return { id, title, input, action, output, evidence, readable: ['pending', 'unknown'].includes(evidence) ? null : readable,
    availableAt, layers, ...(reference ? { artifactAlias: reference.alias, fingerprint: reference.id } : {}) };
}

/** References existing ciphertext artifacts instead of copying large byte strings into each diagram. */
export function describeCryptoJourney(journey) {
  if (!journey || journey.visibility !== 'private') return null;
  const position = STAGES.indexOf(journey.stage);
  const failed = Boolean(journey.failed);
  const waiting = failed ? 'unknown' : 'pending';
  const sources = (journey.objects ?? []).map(value => packetSource(value, 'sender')).filter(Boolean);
  const downloaded = (journey.downloads ?? []).map(value => packetSource(value, 'receiver')).filter(Boolean);
  const hasPost = position >= 1 && typeof journey.post?.id === 'string' && journey.post.checks?.signature === 'valid';
  const hasOuter = position >= 1 && sources.length > 0 && sources.length === journey.objects.length;
  const hasDownload = !failed && position >= 3 && downloaded.length > 0 && downloaded.length === sources.length
    && downloaded.every((item, index) => item.id === sources[index].id);
  const hasRecovered = !failed && journey.complete && journey.stage === 'open' && journey.recovered?.samePost
    && journey.recovered.sameTitle && journey.recovered.sameText && journey.recovered.checks?.signature === 'valid';
  const senderInternal = hasOuter ? 'explained' : waiting;
  const receiverInternal = hasRecovered ? 'explained' : waiting;
  const postReference = hasPost ? { alias: 'P1', id: journey.post.id } : null;
  const packetReference = hasOuter ? sources[0] : null;
  const recovered = hasRecovered ? journey.recovered : null;
  const sourcePacket = hasOuter ? sources.map(packet) : [layer('outer-pending', 'Whole encrypted package · bytes not captured', waiting, null, 'seal')];
  const receiverPacket = hasDownload ? downloaded.map(packet) : [layer('download-pending', 'Whole encrypted package · receipt not verified', waiting, null, 'download')];
  const message = journey.message;
  return {
    copyModel: 'Each relay stores a complete copy of each encrypted package. The receiver opens one complete copy; there are no pieces to join.',
    modelNote: 'Observed values come from this run. Explained layers show the core’s structure; their intermediate bytes were not captured.',
    sender: [
      step('message', '1 · Your readable input', 'Title + message', 'Type the message', 'M1 · input text', 'observed', true, 'draft', [plaintext(message, 'observed', 'draft', 'M1')], { alias: 'M1' }),
      step('signed-post', '2 · Create and sign a post', 'M1 · input text', 'Encode the post; sign its ID inside the core', 'P1 · post view and signature check', hasPost ? 'observed' : waiting, true, 'seal',
        hasPost ? [plaintext(journey.post, 'observed', 'seal', 'P1'), recordModel('explained', 'seal')] : [recordModel(waiting, 'seal')], postReference),
      step('mls', '3 · Encrypt for group members', 'Signed records', 'The core creates an MLS private message', 'Encrypted group message · internal bytes not captured', senderInternal, false, 'seal', [mlsModel(senderInternal, 'seal')]),
      step('padding', '4 · Hide the exact message length', 'MLS private message', 'Add its length field and zero padding', 'Padded content · internal bytes not captured', senderInternal, false, 'seal', [paddingModel(senderInternal, 'seal', packetReference?.size)]),
      step('outer', '5 · Seal the delivery package', 'Padded MLS content', 'Seal with XChaCha20-Poly1305; hash the complete result with SHA-256', 'Whole encrypted package · nonce + ciphertext + authentication tag', hasOuter ? 'observed' : waiting, false, 'seal', sourcePacket, packetReference),
    ],
    receiver: [
      step('package', '1 · Receive the same encrypted bytes', 'One relay’s whole copy', 'Download and compare every byte and SHA-256', 'Encrypted package in the receiver’s browser', hasDownload ? 'observed' : waiting, false, 'download', receiverPacket, hasDownload ? downloaded[0] : null),
      step('outer-open', '2 · Authenticate and open the outer seal', 'Nonce + ciphertext + authentication tag', 'The core uses this member’s local group state', 'Padded MLS content · title and message still hidden', receiverInternal, false, 'open', [paddingModel(receiverInternal, 'open', hasDownload ? downloaded[0].size : undefined)]),
      step('unpad', '3 · Check and remove the padding', 'Length field + MLS message + zeros', 'Check the length and zero padding, then remove them', 'MLS private message · internal bytes not captured', receiverInternal, false, 'open', [mlsModel(receiverInternal, 'open')]),
      step('mls-open', '4 · Open the group message', 'MLS private message', 'MLS authenticates the sender and decrypts the payload', 'Signed records · readable inside the core', receiverInternal, true, 'open', [recordModel(receiverInternal, 'open')]),
      step('validate', '5 · Validate the signed post', 'Recovered signed records', 'Check signatures, identity and community rules', 'P1 · receiver’s post view and checks', hasRecovered ? 'observed' : waiting, true, 'open',
        [layer('validation', hasRecovered ? 'Signature valid · same post ID, title and message' : 'Validation result not verified', hasRecovered ? 'observed' : waiting, hasRecovered ? true : null, 'open',
          hasRecovered ? { artifactAlias: 'P1', fingerprint: recovered.id } : {})], hasRecovered ? { alias: 'P1', id: recovered.id } : null),
      step('readable', '6 · Read the original message', 'Validated P1', 'Return the post to the app', 'The original title and message, compared with M1', hasRecovered ? 'observed' : waiting, true, 'open',
        hasRecovered ? [plaintext(recovered, 'observed', 'open', 'P1')] : [layer('plaintext-pending', 'Readable result not verified', waiting, null, 'open')], hasRecovered ? { alias: 'P1', id: recovered.id } : null),
    ],
  };
}
