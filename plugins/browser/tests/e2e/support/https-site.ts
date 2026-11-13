import { execFileSync } from 'node:child_process';
import { randomBytes, X509Certificate } from 'node:crypto';
import { createServer } from 'node:https';
import type { TLSSocket } from 'node:tls';
import { fileURLToPath } from 'node:url';

const certificateScript = fileURLToPath(new URL('./self-signed.ps1', import.meta.url));

interface TestCertificate {
  cert: string;
  fingerprint: string;
  key: string;
  notAfter: number;
  thumbprint: string;
}

function certificateCommand(...args: string[]): string {
  return execFileSync('pwsh', ['-NoProfile', '-NonInteractive', '-File', certificateScript, ...args], {
    encoding: 'utf8',
    windowsHide: true,
  }).trim();
}

export function selfSignedCertificate(lifetimeSeconds = 3600): TestCertificate {
  const { cert, key } = JSON.parse(certificateCommand('-LifetimeSeconds', String(lifetimeSeconds))) as {
    cert: string;
    key: string;
  };
  const parsed = new X509Certificate(cert);
  return {
    cert,
    fingerprint: parsed.fingerprint256,
    key,
    notAfter: parsed.validToDate.getTime(),
    thumbprint: parsed.fingerprint.replaceAll(':', ''),
  };
}

export function fixtureCertificateInWindowsTrust(certificate: TestCertificate): boolean {
  return certificateCommand('-FindThumbprint', certificate.thumbprint) === 'present';
}

export interface HttpsSite {
  address: string;
  close: () => Promise<void>;
  holdClosedAt: () => number | undefined;
  redirectTo: string | undefined;
  requests: string[];
  rotate: (certificate: TestCertificate) => void;
  untrustedAsset: string | undefined;
}

export async function startHttpsSite(certificate: TestCertificate): Promise<HttpsSite> {
  const connections = new Set<TLSSocket>();
  const requests: string[] = [];
  let holdClosedAt: number | undefined;
  const server = createServer(certificate, (request, response) => {
    const path = request.url ?? '';
    requests.push(path);
    const redirectTo = path === '/redirect' ? site.redirectTo : undefined;
    if (redirectTo !== undefined) {
      response.writeHead(302, { Location: redirectTo }).end();
      return;
    }
    if (path === '/hold') {
      response.once('close', () => {
        holdClosedAt = Date.now();
      });
      response.writeHead(200, { 'Content-Type': 'text/plain' }).write('open\n');
      return;
    }
    if (path === '/fixture.js') {
      response
        .writeHead(200, { 'Content-Type': 'text/javascript' })
        .end("document.documentElement.dataset.fixtureScript = 'loaded'");
      return;
    }
    const hold = path === '/keep-connection' ? "<script>fetch('/hold')</script>" : '';
    const assetUrl = path === '/with-background' ? site.untrustedAsset : undefined;
    const asset =
      assetUrl === undefined
        ? ''
        : `<script src="${assetUrl}" onerror="document.documentElement.dataset.background = 'blocked'"></script>`;
    response
      .writeHead(200, { 'Cache-Control': 'no-store', 'Content-Type': 'text/html; charset=utf-8' })
      .end(
        `<!doctype html><html lang="en"><title>Self-signed fixture</title><h1>Self-signed fixture</h1><script src="/fixture.js"></script>${hold}${asset}`,
      );
  });
  server.on('secureConnection', (socket) => {
    connections.add(socket);
    socket.once('close', () => {
      connections.delete(socket);
    });
  });
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  const hasPort = address !== null && typeof address !== 'string';
  if (!hasPort) {
    throw new Error('HTTPS fixture did not open a TCP address');
  }
  const site: HttpsSite = {
    address: `https://127.0.0.1:${address.port}`,
    close: () => {
      server.closeAllConnections();
      return new Promise<void>((resolve, reject) => {
        server.close((error) => {
          if (error === undefined) {
            resolve();
          } else {
            reject(error);
          }
        });
      });
    },
    holdClosedAt: () => holdClosedAt,
    redirectTo: undefined,
    requests,
    rotate: (replacement) => {
      server.setSecureContext(replacement);
      server.setTicketKeys(randomBytes(48));
      for (const socket of connections) {
        socket.destroy();
      }
    },
    untrustedAsset: undefined,
  };
  return site;
}
