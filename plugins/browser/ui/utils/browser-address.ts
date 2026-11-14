/** Defaults a bare owner-entered address to HTTPS. The browser service validates the result. */
export function browserAddress(input: string): string {
  const address = input.trim();
  const explicitWebScheme = /^https?:/iu.test(address);
  const portWithoutScheme = /^[^:/?#\s]+:\d+(?=[/?#]|$)/u.test(address);
  const hasScheme = /^[a-z][a-z\d+.-]*:/iu.test(address) && !portWithoutScheme;
  const relative = /^[\\/?#]/u.test(address);
  const empty = address === '';
  return empty || explicitWebScheme || hasScheme || relative ? address : `https://${address}`;
}
