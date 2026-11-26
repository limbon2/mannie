// Pure Node.js loopback fake Proxmox VE HTTPS + RFC 6455 WebSocket + RFB 3.8
// server for WebdriverIO end-to-end tests (V16; ADR-0080). Generates its own
// ECDSA P-256 self-signed X.509 v3 certificate in memory with zero external
// tools so tests run identically on Windows and Linux.
import { createHash, generateKeyPairSync, sign, X509Certificate } from 'node:crypto';
import type { IncomingMessage, ServerResponse } from 'node:http';
import { createServer, type Server } from 'node:https';
import type { Duplex } from 'node:stream';

const TICKET = 'PVE:e2e@pve:E2E_FAKE_TICKET_SECRET::sig';
const CSRF = 'E2E_FAKE_CSRF_SECRET';
const VNC_TICKET = 'E2EVNCTKT99';

interface FakeVm {
  readonly config: Record<string, string>;
  readonly height: number;
  readonly name: string;
  readonly node: string;
  readonly pixels: Buffer;
  status: string;
  readonly vmid: number;
  readonly width: number;
}

/** Recorded RFB key event sent to the fake Proxmox VNC endpoint. */
export interface RecordedKeyEvent {
  readonly down: boolean;
  readonly keysym: number;
  readonly vmid: number;
}

/** Recorded RFB pointer event sent to the fake Proxmox VNC endpoint. */
export interface RecordedPointerEvent {
  readonly buttonMask: number;
  readonly vmid: number;
  readonly x: number;
  readonly y: number;
}

interface GeneratedCert {
  readonly certPem: string;
  readonly fingerprint: string;
  readonly keyPem: string;
}

function derTlv(tag: number, body: Buffer): Buffer {
  let lenBuf: Buffer;
  if (body.length < 0x80) {
    lenBuf = Buffer.from([body.length]);
  } else if (body.length <= 0xff) {
    lenBuf = Buffer.from([0x81, body.length]);
  } else {
    lenBuf = Buffer.from([0x82, (body.length >> 8) & 0xff, body.length & 0xff]);
  }
  return Buffer.concat([Buffer.from([tag]), lenBuf, body]);
}

function derSeq(parts: readonly Buffer[]): Buffer {
  return derTlv(0x30, Buffer.concat(parts));
}

function derSet(parts: readonly Buffer[]): Buffer {
  return derTlv(0x31, Buffer.concat(parts));
}

function derOid(bytes: readonly number[]): Buffer {
  return derTlv(0x06, Buffer.from(bytes));
}

function derBitString(bytes: Buffer, unusedBits = 0): Buffer {
  return derTlv(0x03, Buffer.concat([Buffer.from([unusedBits]), bytes]));
}

function derExtension(oid: readonly number[], critical: boolean, value: Buffer): Buffer {
  const parts: Buffer[] = [derOid(oid)];
  if (critical) {
    parts.push(derTlv(0x01, Buffer.from([0xff])));
  }
  parts.push(derTlv(0x04, value));
  return derSeq(parts);
}

function derCnName(cn: string): Buffer {
  const attr = derSeq([derOid([0x55, 0x04, 0x03]), derTlv(0x0c, Buffer.from(cn, 'utf8'))]);
  return derSeq([derSet([attr])]);
}

function generateSelfSignedCert(): GeneratedCert {
  const { privateKey, publicKey } = generateKeyPairSync('ec', { namedCurve: 'prime256v1' });
  const spkiDer = publicKey.export({ format: 'der', type: 'spki' });
  const keyPem = privateKey.export({ format: 'pem', type: 'pkcs8' }).toString();

  const sigAlg = derSeq([derOid([0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02])]);
  const version = derTlv(0xa0, derTlv(0x02, Buffer.from([2])));
  const serial = derTlv(0x02, Buffer.from([0x42]));
  const name = derCnName('pve-e2e.local');
  const validity = derSeq([
    derTlv(0x17, Buffer.from('250101000000Z', 'ascii')),
    derTlv(0x17, Buffer.from('350101000000Z', 'ascii')),
  ]);

  const bcExt = derExtension([0x55, 0x1d, 0x13], true, derSeq([]));
  const kuExt = derExtension([0x55, 0x1d, 0x0f], true, derBitString(Buffer.from([0x80]), 7));
  const ekuExt = derExtension(
    [0x55, 0x1d, 0x25],
    false,
    derSeq([derOid([0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01])]),
  );
  const sanExt = derExtension(
    [0x55, 0x1d, 0x11],
    false,
    derSeq([derTlv(0x82, Buffer.from('localhost', 'ascii')), derTlv(0x87, Buffer.from([127, 0, 0, 1]))]),
  );
  const extensions = derTlv(0xa3, derSeq([bcExt, kuExt, ekuExt, sanExt]));

  const tbs = derSeq([version, serial, sigAlg, name, validity, name, spkiDer, extensions]);
  const signature = sign('sha256', tbs, privateKey);
  const certDer = derSeq([tbs, sigAlg, derBitString(signature)]);
  const b64 = certDer.toString('base64');
  const wrapped = b64.match(/.{1,64}/g)?.join('\n') ?? b64;
  const certPem = `-----BEGIN CERTIFICATE-----\n${wrapped}\n-----END CERTIFICATE-----\n`;
  const parsed = new X509Certificate(certPem);
  return { certPem, fingerprint: parsed.fingerprint256, keyPem };
}

function makeFakeVm(vmid: number, node: string, name: string, status: string): FakeVm {
  const width = 128;
  const height = 96;
  const pixels = Buffer.alloc(width * height * 4);
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const idx = (y * width + x) * 4;
      pixels[idx] = (x * 2) & 0xff;
      pixels[idx + 1] = (y * 2) & 0xff;
      pixels[idx + 2] = 180;
      pixels[idx + 3] = 255;
    }
  }
  return {
    config: {
      boot: 'order=scsi0;ide2',
      cores: '2',
      ide2: 'none,media=cdrom',
      memory: '2048',
      name,
      scsi0: `local-lvm:vm-${String(vmid)}-disk-0,size=20G`,
    },
    height,
    name,
    node,
    pixels,
    status,
    vmid,
    width,
  };
}

function sendWsBinary(socket: Duplex, payload: Buffer): void {
  let header: Buffer;
  if (payload.length < 126) {
    header = Buffer.from([0x82, payload.length]);
  } else if (payload.length <= 0xffff) {
    header = Buffer.alloc(4);
    header[0] = 0x82;
    header[1] = 126;
    header.writeUInt16BE(payload.length, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x82;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(payload.length), 2);
  }
  socket.write(Buffer.concat([header, payload]));
}

function encodeRawFrame(vm: FakeVm): Buffer {
  const header = Buffer.alloc(16);
  header[0] = 0; // FramebufferUpdate
  header.writeUInt16BE(1, 2); // 1 rect
  header.writeUInt16BE(0, 4);
  header.writeUInt16BE(0, 6);
  header.writeUInt16BE(vm.width, 8);
  header.writeUInt16BE(vm.height, 10);
  header.writeInt32BE(0, 12); // Raw
  return Buffer.concat([header, vm.pixels]);
}

async function readBody(request: IncomingMessage): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of request) {
    chunks.push(typeof chunk === 'string' ? Buffer.from(chunk) : (chunk as Buffer));
  }
  return Buffer.concat(chunks).toString('utf8');
}

function writeJson(response: ServerResponse, status: number, data: unknown): void {
  const body = JSON.stringify({ data });
  response.writeHead(status, {
    'Content-Length': Buffer.byteLength(body),
    'Content-Type': 'application/json',
  });
  response.end(body);
}

/** Loopback fake Proxmox VE HTTPS + VNC WebSocket server for E2E specs. */
export class FakeProxmoxE2eServer {
  readonly configUpdates: { readonly params: Record<string, string>; readonly vmid: number }[] = [];
  readonly fingerprint: string;
  readonly keyEvents: RecordedKeyEvent[] = [];
  readonly password = 'e2e-proxmox-password-9988';
  readonly pointerEvents: RecordedPointerEvent[] = [];
  readonly port: number;
  readonly powerActions: { readonly action: string; readonly vmid: number }[] = [];
  readonly username = 'e2e@pve';

  readonly #server: Server;
  readonly #sockets = new Set<Duplex>();
  readonly #vms: FakeVm[];

  private constructor(server: Server, port: number, fingerprint: string, vms: FakeVm[]) {
    this.#server = server;
    this.port = port;
    this.fingerprint = fingerprint;
    this.#vms = vms;
  }

  static async start(): Promise<FakeProxmoxE2eServer> {
    const cert = generateSelfSignedCert();
    const vms = [
      makeFakeVm(100, 'pve1', 'debian-lab', 'running'),
      makeFakeVm(200, 'pve1', 'prod-secret-db', 'running'),
    ];
    const holder: { current: FakeProxmoxE2eServer | null } = { current: null };

    const server = createServer({ cert: cert.certPem, key: cert.keyPem }, (request, response) => {
      if (holder.current !== null) {
        void holder.current.#handleHttp(request, response);
      }
    });
    server.on('upgrade', (request, socket) => {
      if (holder.current !== null) {
        holder.current.#handleUpgrade(request, socket);
      }
    });

    await new Promise<void>((resolve, reject) => {
      server.once('error', reject);
      server.listen(0, '127.0.0.1', resolve);
    });

    const address = server.address();
    const isTcpAddress = address !== null && typeof address !== 'string';
    if (!isTcpAddress) {
      throw new Error('Fake Proxmox E2E server did not bind to a TCP port');
    }

    const instance = new FakeProxmoxE2eServer(server, address.port, cert.fingerprint, vms);
    holder.current = instance;
    return instance;
  }

  async stop(): Promise<void> {
    for (const socket of this.#sockets) {
      socket.destroy();
    }
    this.#server.closeAllConnections();
    await new Promise<void>((resolve) => {
      this.#server.close(() => {
        resolve();
      });
    });
  }

  #findVm(vmid: number, node: string): FakeVm | undefined {
    return this.#vms.find((vm) => {
      const sameVmid = vm.vmid === vmid;
      const sameNode = vm.node === node;
      return sameVmid && sameNode;
    });
  }

  async #handleHttp(request: IncomingMessage, response: ServerResponse): Promise<void> {
    const url = new URL(request.url ?? '/', 'https://127.0.0.1');
    const method = request.method ?? 'GET';
    const path = url.pathname;

    const isTicketLogin = method === 'POST' && path === '/api2/json/access/ticket';
    if (isTicketLogin) {
      const params = new URLSearchParams(await readBody(request));
      const userOk = params.get('username') === this.username;
      const passOk = params.get('password') === this.password;
      if (!userOk || !passOk) {
        writeJson(response, 401, null);
        return;
      }
      writeJson(response, 200, {
        CSRFPreventionToken: CSRF,
        ticket: TICKET,
        username: this.username,
      });
      return;
    }

    const cookie = request.headers.cookie ?? '';
    if (!cookie.includes(`PVEAuthCookie=${TICKET}`)) {
      writeJson(response, 401, null);
      return;
    }

    const isMutation = method === 'POST' || method === 'PUT';
    const hasValidCsrf = request.headers.csrfpreventiontoken === CSRF;
    if (isMutation && !hasValidCsrf) {
      writeJson(response, 401, null);
      return;
    }

    const isClusterResources = method === 'GET' && path === '/api2/json/cluster/resources';
    if (isClusterResources) {
      const list = this.#vms.map((vm) => ({
        maxcpu: 2,
        maxmem: 2_147_483_648,
        name: vm.name,
        node: vm.node,
        status: vm.status,
        type: 'qemu',
        vmid: vm.vmid,
      }));
      writeJson(response, 200, list);
      return;
    }

    const segments = path.replace(/^\/+/u, '').split('/');
    const hasNodesPrefix =
      segments.length >= 5 && segments[0] === 'api2' && segments[1] === 'json' && segments[2] === 'nodes';
    if (!hasNodesPrefix) {
      writeJson(response, 404, null);
      return;
    }

    const node = segments[3] ?? '';
    const isStorageGet = segments[4] === 'storage' && method === 'GET';
    if (isStorageGet) {
      if (segments.length === 5) {
        writeJson(response, 200, [{ content: 'iso', storage: 'local' }]);
        return;
      }
      const isStorageContent = segments.length === 7 && segments[6] === 'content';
      if (isStorageContent) {
        writeJson(response, 200, [
          { content: 'iso', size: 680 * 1024 * 1024, volid: 'local:iso/debian-13-netinst.iso' },
        ]);
        return;
      }
    }

    const isTaskStatus = segments[4] === 'tasks' && method === 'GET' && segments[6] === 'status';
    if (isTaskStatus) {
      writeJson(response, 200, { exitstatus: 'OK', status: 'stopped' });
      return;
    }

    const isQemuRoute = segments[4] === 'qemu' && segments.length >= 7;
    if (isQemuRoute) {
      const vmid = Number.parseInt(segments[5] ?? '', 10);
      const vm = this.#findVm(vmid, node);
      if (vm === undefined) {
        writeJson(response, 404, null);
        return;
      }

      const isConfigGet = segments[6] === 'config' && method === 'GET';
      if (isConfigGet) {
        writeJson(response, 200, vm.config);
        return;
      }

      const isConfigUpdate = segments[6] === 'config' && isMutation;
      if (isConfigUpdate) {
        const params = Object.fromEntries(new URLSearchParams(await readBody(request)));
        Object.assign(vm.config, params);
        this.configUpdates.push({ params, vmid });
        writeJson(response, 200, null);
        return;
      }

      const isStatusCurrent = segments[6] === 'status' && segments[7] === 'current' && method === 'GET';
      if (isStatusCurrent) {
        writeJson(response, 200, { cpus: 2, maxmem: 2_147_483_648, name: vm.name, status: vm.status });
        return;
      }

      const action = segments[7];
      const isPowerPost = segments[6] === 'status' && method === 'POST' && action !== undefined;
      if (isPowerPost) {
        const isPowerOff = action === 'stop' || action === 'shutdown';
        vm.status = isPowerOff ? 'stopped' : 'running';
        this.powerActions.push({ action, vmid });
        writeJson(response, 200, `UPID:${node}:0001:0001:0001:qm${action}:${String(vmid)}:e2e@pve:`);
        return;
      }

      const isVncProxy = segments[6] === 'vncproxy' && method === 'POST';
      if (isVncProxy) {
        writeJson(response, 200, {
          port: '5900',
          ticket: VNC_TICKET,
          upid: `UPID:${node}:0002:0002:0002:vncproxy:${String(vmid)}:e2e@pve:`,
          user: this.username,
        });
        return;
      }
    }

    writeJson(response, 404, null);
  }

  #handleUpgrade(request: IncomingMessage, socket: Duplex): void {
    const url = new URL(request.url ?? '/', 'https://127.0.0.1');
    const cookie = request.headers.cookie ?? '';
    const vncOk = cookie.includes(`PVEAuthCookie=${TICKET}`) && url.searchParams.get('vncticket') === VNC_TICKET;
    if (!vncOk) {
      socket.write('HTTP/1.1 401 Unauthorized\r\n\r\n');
      socket.destroy();
      return;
    }

    const segments = url.pathname.replace(/^\/+/u, '').split('/');
    const node = segments[3] ?? 'pve1';
    const vmid = Number.parseInt(segments[5] ?? '100', 10);
    const vm = this.#findVm(vmid, node);
    if (vm === undefined) {
      socket.write('HTTP/1.1 404 Not Found\r\n\r\n');
      socket.destroy();
      return;
    }

    const wsKey = request.headers['sec-websocket-key'] ?? '';
    const accept = createHash('sha1').update(`${wsKey}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64');
    socket.write(
      `HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\nSec-WebSocket-Protocol: binary\r\n\r\n`,
    );

    this.#sockets.add(socket);
    socket.once('close', () => {
      this.#sockets.delete(socket);
    });

    this.#runRfbSession(socket, vm);
  }

  #runRfbSession(socket: Duplex, vm: FakeVm): void {
    let rawBuf: Buffer = Buffer.alloc(0);
    let rfbBuf: Buffer = Buffer.alloc(0);
    let stage: 'challenge' | 'client_init' | 'ready' | 'sec_type' | 'version' = 'version';

    sendWsBinary(socket, Buffer.from('RFB 003.008\n', 'ascii'));

    const processRfb = (): void => {
      while (rfbBuf.length > 0) {
        if (stage === 'version') {
          if (rfbBuf.length < 12) {
            return;
          }
          rfbBuf = rfbBuf.subarray(12);
          stage = 'sec_type';
          sendWsBinary(socket, Buffer.from([1, 2]));
          continue;
        }
        if (stage === 'sec_type') {
          rfbBuf = rfbBuf.subarray(1);
          stage = 'challenge';
          sendWsBinary(socket, Buffer.alloc(16, 0x22));
          continue;
        }
        if (stage === 'challenge') {
          if (rfbBuf.length < 16) {
            return;
          }
          rfbBuf = rfbBuf.subarray(16);
          stage = 'client_init';
          sendWsBinary(socket, Buffer.alloc(4, 0));
          continue;
        }
        if (stage === 'client_init') {
          rfbBuf = rfbBuf.subarray(1);
          stage = 'ready';
          const nameBytes = Buffer.from(vm.name, 'utf8');
          const init = Buffer.alloc(24 + nameBytes.length);
          init.writeUInt16BE(vm.width, 0);
          init.writeUInt16BE(vm.height, 2);
          Buffer.from([32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 0, 8, 16, 0, 0, 0]).copy(init, 4);
          init.writeUInt32BE(nameBytes.length, 20);
          nameBytes.copy(init, 24);
          sendWsBinary(socket, init);
          continue;
        }

        const msgType = rfbBuf[0];
        if (msgType === 0) {
          if (rfbBuf.length < 20) {
            return;
          }
          rfbBuf = rfbBuf.subarray(20);
        } else if (msgType === 2) {
          if (rfbBuf.length < 4) {
            return;
          }
          const count = rfbBuf.readUInt16BE(2);
          const total = 4 + count * 4;
          if (rfbBuf.length < total) {
            return;
          }
          rfbBuf = rfbBuf.subarray(total);
        } else if (msgType === 3) {
          if (rfbBuf.length < 10) {
            return;
          }
          const incremental = rfbBuf[1];
          rfbBuf = rfbBuf.subarray(10);
          if (incremental === 0) {
            sendWsBinary(socket, encodeRawFrame(vm));
          }
        } else if (msgType === 4) {
          if (rfbBuf.length < 8) {
            return;
          }
          const down = rfbBuf[1] !== 0;
          const keysym = rfbBuf.readUInt32BE(4);
          rfbBuf = rfbBuf.subarray(8);
          this.keyEvents.push({ down, keysym, vmid: vm.vmid });
          vm.pixels[0] = ((vm.pixels[0] ?? 0) + 31) & 0xff;
          sendWsBinary(socket, encodeRawFrame(vm));
        } else if (msgType === 5) {
          if (rfbBuf.length < 6) {
            return;
          }
          const buttonMask = rfbBuf[1] ?? 0;
          const x = rfbBuf.readUInt16BE(2);
          const y = rfbBuf.readUInt16BE(4);
          rfbBuf = rfbBuf.subarray(6);
          this.pointerEvents.push({ buttonMask, vmid: vm.vmid, x, y });
          vm.pixels[4] = ((vm.pixels[4] ?? 0) + 43) & 0xff;
          sendWsBinary(socket, encodeRawFrame(vm));
        } else if (msgType === 6) {
          if (rfbBuf.length < 8) {
            return;
          }
          const len = rfbBuf.readUInt32BE(4);
          if (rfbBuf.length < 8 + len) {
            return;
          }
          rfbBuf = rfbBuf.subarray(8 + len);
        } else {
          socket.destroy();
          return;
        }
      }
    };

    socket.on('data', (chunk: Buffer) => {
      rawBuf = Buffer.concat([rawBuf, chunk]);
      while (rawBuf.length >= 2) {
        const opcode = (rawBuf[0] ?? 0) & 0x0f;
        const masked = ((rawBuf[1] ?? 0) & 0x80) !== 0;
        let payloadLen = (rawBuf[1] ?? 0) & 0x7f;
        let offset = 2;
        if (payloadLen === 126) {
          if (rawBuf.length < 4) {
            return;
          }
          payloadLen = rawBuf.readUInt16BE(2);
          offset = 4;
        } else if (payloadLen === 127) {
          if (rawBuf.length < 10) {
            return;
          }
          payloadLen = Number(rawBuf.readBigUInt64BE(2));
          offset = 10;
        }
        const maskLen = masked ? 4 : 0;
        if (rawBuf.length < offset + maskLen + payloadLen) {
          return;
        }
        const mask = masked ? rawBuf.subarray(offset, offset + 4) : null;
        const payload = Buffer.from(rawBuf.subarray(offset + maskLen, offset + maskLen + payloadLen));
        rawBuf = rawBuf.subarray(offset + maskLen + payloadLen);
        if (mask !== null) {
          for (let i = 0; i < payload.length; i += 1) {
            payload[i] = (payload[i] ?? 0) ^ (mask[i % 4] ?? 0);
          }
        }
        if (opcode === 0x8) {
          socket.end();
          return;
        }
        if (opcode === 0x9) {
          socket.write(Buffer.concat([Buffer.from([0x8a, payload.length]), payload]));
          continue;
        }
        if (opcode <= 0x2) {
          rfbBuf = Buffer.concat([rfbBuf, payload]);
          processRfb();
        }
      }
    });
  }
}
