import release from '../release.json' with { type: 'json' };
import { RegistryObject, fetchRegistry } from './worker-runtime.mjs';

export class CommunityRegistry extends RegistryObject {
  constructor(ctx, env) { super(ctx, env, release); }
}

export default { fetch: fetchRegistry };
