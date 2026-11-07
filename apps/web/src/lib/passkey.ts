// Passkey unlock: a WebAuthn credential whose PRF answer, for a salt kept in the
// vault's header, opens the vault as the passphrase does. The calls happen on the page (WebAuthn
// needs a window); the worker only ever sees the credential id and the 32-byte answer. The
// passphrase stays, and the recovery line stays the way back to the person.
import { fromBase64Url, toBase64Url } from './core/rpc-core';

export interface VaultPasskey { credential: string; salt: string }

/** Whether this browser offers WebAuthn at all. PRF support shows only when it is tried. */
export const passkeysPossible = (): boolean => typeof window !== 'undefined' && !!window.PublicKeyCredential && !!navigator.credentials;

const random = (n: number) => crypto.getRandomValues(new Uint8Array(n));

function prfFirst(credential: PublicKeyCredential): Uint8Array | null {
  const results = (credential.getClientExtensionResults() as { prf?: { results?: { first?: ArrayBuffer } } }).prf?.results?.first;
  return results ? new Uint8Array(results) : null;
}

/** Asks an authenticator for a key's answer, with each passkey's own salt. */
async function ask(passkeys: { credential: Uint8Array; salt: Uint8Array }[]): Promise<{ credential: Uint8Array; prf: Uint8Array }> {
  const got = (await navigator.credentials.get({
    publicKey: {
      challenge: random(32),
      allowCredentials: passkeys.map((p) => ({ type: 'public-key', id: p.credential as BufferSource })),
      userVerification: 'required',
      extensions: { prf: { evalByCredential: Object.fromEntries(passkeys.map((p) => [toBase64Url(p.credential), { first: p.salt as BufferSource }])) } } as AuthenticationExtensionsClientInputs,
    },
  })) as PublicKeyCredential | null;
  const prf = got && prfFirst(got);
  if (!got || !prf || prf.length !== 32) throw new Error("This passkey can't unlock hoi: it gave no PRF answer.");
  return { credential: new Uint8Array(got.rawId), prf };
}

/** Makes a passkey for this vault and its first answer: what the worker stores in a slot. */
export async function makePasskey(label: string): Promise<{ credential: string; salt: string; prf: string }> {
  const salt = random(32);
  const made = (await navigator.credentials.create({
    publicKey: {
      rp: { name: 'hoi' },
      user: { id: random(16), name: label, displayName: label },
      challenge: random(32),
      pubKeyCredParams: [{ type: 'public-key', alg: -7 }, { type: 'public-key', alg: -257 }],
      authenticatorSelection: { residentKey: 'preferred', userVerification: 'required' },
      extensions: { prf: { eval: { first: salt } } } as AuthenticationExtensionsClientInputs,
    },
  })) as PublicKeyCredential | null;
  if (!made) throw new Error('No passkey was made.');
  const credential = new Uint8Array(made.rawId);
  // Some authenticators answer at creation; the rest only when asked.
  const prf = prfFirst(made) ?? (await ask([{ credential, salt }])).prf;
  return { credential: toBase64Url(credential), salt: toBase64Url(salt), prf: toBase64Url(prf) };
}

/** Asks one of the vault's passkeys for its answer. */
export async function passkeyAnswer(passkeys: VaultPasskey[]): Promise<{ credential: string; prf: string }> {
  const got = await ask(passkeys.map((p) => ({ credential: fromBase64Url(p.credential), salt: fromBase64Url(p.salt) })));
  return { credential: toBase64Url(got.credential), prf: toBase64Url(got.prf) };
}
