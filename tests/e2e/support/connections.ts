// A local server that counts the connections opened to it, so that a test can
// show that content which names it was never fetched (ADR-0004, ADR-0021).
import { createServer } from 'node:net';

/** The running counter. */
export interface ConnectionCounter {
  close(): Promise<void>;
  /** Connections opened so far. */
  connections(): number;
  /** The origin that test content names. */
  readonly origin: string;
}

/** Starts a counter on a free port of the loopback interface. */
export async function countConnections(): Promise<ConnectionCounter> {
  let connections = 0;
  const server = createServer((socket) => {
    connections += 1;
    socket.destroy();
  });
  await new Promise<void>((resolve) => {
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  const hasNoPort = address === null || typeof address === 'string';
  if (hasNoPort) {
    throw new Error('the connection counter has no port');
  }
  return {
    close: async () => {
      await new Promise<void>((resolve) => {
        server.close(() => {
          resolve();
        });
      });
    },
    connections: () => connections,
    origin: `https://127.0.0.1:${String(address.port)}`,
  };
}
