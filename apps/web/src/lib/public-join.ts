import { RELEASE } from './preview';
import { publicNameValid } from './registry';

export function publicTarget(query: string) {
  const params = new URLSearchParams(query);
  const id = params.get('c') ?? '';
  const name = params.get('name') ?? '';
  const protocol = params.get('p') ?? '';
  if (!/^[0-9a-f]{64}$/.test(id) || (name && !publicNameValid(name)) || (protocol && !/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(protocol))) {
    throw new Error('This public community link is invalid. It needs a full community key.');
  }
  return { id, name, protocol, compatible: !protocol || RELEASE.supportedProtocols.includes(protocol) };
}
