// Versions of the client's own notices. Plain JavaScript, so the browser tests and the
// site can read them too. Bump a version when the words change in substance: the client shows
// that notice again before it continues.
export const NETWORK_NOTICE_VERSION = '1.2';
export const PRIVACY_NOTICE_VERSION = '1.2';
export const PRIVACY_NOTICE_DATE = '2026-11-05';
/** localStorage key of the Network & Safety acknowledgement: `{"version":"1.2","date":"2026-11-05"}`. */
export const CONSENT_KEY = 'hoi:consent:network';
