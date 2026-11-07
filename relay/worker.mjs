import { RelayObject, fetchRelay } from './worker-runtime.mjs';

export class OpaqueRelay extends RelayObject {}
export { fetchRelay };
export default { fetch: fetchRelay };
