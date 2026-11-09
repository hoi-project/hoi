// GitHub sets missing secrets to empty strings. Tauri treats their presence as a request to
// import a certificate or notarize, even when unsigned installers were explicitly allowed.
export function clearEmptyAppleSigningEnv(env = process.env) {
  for (const name of ['APPLE_CERTIFICATE', 'APPLE_SIGNING_IDENTITY', 'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID']) {
    if (env[name] === '') delete env[name];
  }
  // A supplied PKCS#12 may have an empty password. Removing it would skip certificate import.
  if (env.APPLE_CERTIFICATE === undefined && env.APPLE_CERTIFICATE_PASSWORD === '') {
    delete env.APPLE_CERTIFICATE_PASSWORD;
  }
}
